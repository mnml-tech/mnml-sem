//! Emit SQL from a [`LogicalPlan`](crate::plan::LogicalPlan).
//!
//! A plan is a tree of operators that compose freely; SQL is a clause-based
//! language where one `SELECT` has exactly one `WHERE`, one `GROUP BY`, one
//! `ORDER BY`. So several plan nodes usually collapse into one `SELECT`, and
//! occasionally they cannot.
//!
//! This emitter accumulates clauses into a [`Block`] as it walks down, and
//! opens a subquery only at a real boundary: when a node wants a clause that
//! SQL evaluates *before* one already occupied. Filtering after aggregating,
//! for instance, cannot reuse the same block's `WHERE`, because `WHERE` runs
//! before `GROUP BY`.
//!
//! The same rule is why `Project` over `Aggregate` stays flat: a select list
//! can hold a composite of aggregates, so the measures the planner lifted out
//! are substituted back rather than wrapped.

use std::collections::HashMap;

use crate::catalog::Func;
use crate::emit::EmitError;
use crate::plan::{Expr, LogicalPlan, NamedExpr, SchemaColumn, SortKey, find_column};
use crate::plan::{SqlCtx, format_expr};
use crate::query::{Direction, NullOrder};
use crate::sql_dialect::{Ansi, SqlDialect};

/// Build a single ANSI `SELECT` statement for the plan tree.
pub fn to_sql(plan: &LogicalPlan) -> Result<String, EmitError> {
    to_sql_with(plan, &Ansi)
}

/// Build a `SELECT` statement for the plan tree in the given SQL `dialect`.
pub fn to_sql_with(plan: &LogicalPlan, dialect: &dyn SqlDialect) -> Result<String, EmitError> {
    let mut aliases = Aliases::default();
    let block = Block::build(plan, dialect, &mut aliases)?;
    Ok(block.render(dialect))
}

/// The clauses of one `SELECT`, in the order SQL evaluates them.
///
/// A node wanting clause `C` can join the current block only if nothing later
/// than `C` is already occupied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Clause {
    From,
    Where,
    GroupBy,
    Select,
    OrderBy,
    Fetch,
}

/// Fresh subquery aliases.
#[derive(Default)]
struct Aliases(usize);

impl Aliases {
    fn next(&mut self) -> String {
        let alias = format!("t{}", self.0);
        self.0 += 1;
        alias
    }
}

/// One `SELECT` under construction.
struct Block {
    /// Rendered `FROM`, including any joins.
    from: String,
    /// Whether the `FROM` has more than one source, so columns need
    /// qualifying.
    multi_source: bool,
    /// The columns this block exposes, for a node above it to resolve against.
    schema: Vec<SchemaColumn>,
    filters: Vec<Expr>,
    group_by: Vec<Expr>,
    /// `None` renders as `SELECT *`.
    select: Option<Vec<NamedExpr>>,
    order_by: Vec<SortKey>,
    offset: u64,
    limit: Option<u64>,
}

impl Block {
    fn source(from: String, multi_source: bool, schema: Vec<SchemaColumn>) -> Self {
        Self {
            from,
            multi_source,
            schema,
            filters: Vec::new(),
            group_by: Vec::new(),
            select: None,
            order_by: Vec::new(),
            offset: 0,
            limit: None,
        }
    }

    /// The latest clause this block has occupied.
    fn occupied(&self) -> Clause {
        if self.limit.is_some() || self.offset > 0 {
            Clause::Fetch
        } else if !self.order_by.is_empty() {
            Clause::OrderBy
        } else if self.select.is_some() {
            Clause::Select
        } else if !self.group_by.is_empty() {
            Clause::GroupBy
        } else if !self.filters.is_empty() {
            Clause::Where
        } else {
            Clause::From
        }
    }

    fn ctx<'a>(&self, dialect: &'a dyn SqlDialect) -> SqlCtx<'a> {
        SqlCtx {
            dialect,
            qualify: self.multi_source,
        }
    }

    fn build(
        plan: &LogicalPlan,
        dialect: &dyn SqlDialect,
        aliases: &mut Aliases,
    ) -> Result<Block, EmitError> {
        match plan {
            LogicalPlan::Scan { source, .. } => Ok(Block::source(
                dialect.quote_source(source),
                false,
                plan.output_schema(),
            )),

            LogicalPlan::Join { left, right, on } => {
                if on.is_empty() {
                    return Err(EmitError::InvalidPlan("join with no keys"));
                }
                let (left_sql, mut schema) =
                    Block::build(left, dialect, aliases)?.into_join_side(false, dialect, aliases);
                let (right_sql, right_schema) =
                    Block::build(right, dialect, aliases)?.into_join_side(true, dialect, aliases);
                schema.extend(right_schema);

                let mut block = Block::source(String::new(), true, schema);
                // A wrapped side's columns moved behind its alias, so the keys
                // must be resolved against the schema rather than trusted.
                let on_sql = on
                    .iter()
                    .map(|k| {
                        let left = resolve_key(&block.schema, &k.left_dataset, &k.left_column)?;
                        let right = resolve_key(&block.schema, &k.right_dataset, &k.right_column)?;
                        Ok(format_expr(
                            &Expr::binary(Func::Equal, left, right),
                            block.ctx(dialect),
                        ))
                    })
                    .collect::<Result<Vec<_>, EmitError>>()?
                    .join(" AND ");
                block.from = format!("{left_sql} INNER JOIN {right_sql} ON {on_sql}");
                Ok(block)
            }

            LogicalPlan::Filter { input, predicate } => {
                let mut block = Block::build(input, dialect, aliases)?;
                // `WHERE` runs before `GROUP BY` and `SELECT`, so it cannot be
                // added to a block that already has them.
                let predicate = block.reserve(Clause::Where, dialect, aliases, [predicate])?;
                block.filters.extend(predicate);
                Ok(block)
            }

            LogicalPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => {
                let mut block = Block::build(input, dialect, aliases)?;
                let exprs = group_by.iter().chain(aggregates.iter().map(|a| &a.expr));
                let mut rewritten = block
                    .reserve(Clause::GroupBy, dialect, aliases, exprs)?
                    .into_iter();

                let group_by: Vec<Expr> = group_by
                    .iter()
                    .map(|_| rewritten.next().expect("group"))
                    .collect();
                let select: Vec<NamedExpr> = group_by
                    .iter()
                    .map(|e| NamedExpr::new(String::new(), e.clone()))
                    .chain(
                        aggregates
                            .iter()
                            .map(|a| NamedExpr::new(&a.name, rewritten.next().expect("measure"))),
                    )
                    .collect();

                block.group_by = group_by;
                block.select = Some(select);
                block.schema = plan.output_schema();
                Ok(block)
            }

            LogicalPlan::Project { input, exprs } => {
                let mut block = Block::build(input, dialect, aliases)?;
                let rewritten = block.reserve(
                    Clause::Select,
                    dialect,
                    aliases,
                    exprs.iter().map(|e| &e.expr),
                )?;

                // A select list can hold a composite of aggregates, so a
                // projection over one folds in rather than wrapping: the
                // lifted measures go back where they were written.
                let outputs: Vec<NamedExpr> = match block.select.take() {
                    Some(existing) => {
                        let by_name: HashMap<&str, &Expr> = existing
                            .iter()
                            .filter(|n| !n.name.is_empty())
                            .map(|n| (n.name.as_str(), &n.expr))
                            .collect();
                        exprs
                            .iter()
                            .zip(rewritten)
                            .map(|(named, e)| NamedExpr::new(&named.name, inline(&e, &by_name)))
                            .collect()
                    }
                    None => exprs
                        .iter()
                        .zip(rewritten)
                        .map(|(named, e)| NamedExpr::new(&named.name, e))
                        .collect(),
                };

                block.select = Some(outputs);
                block.schema = plan.output_schema();
                Ok(block)
            }

            LogicalPlan::Sort { input, keys } => {
                let mut block = Block::build(input, dialect, aliases)?;
                let rewritten = block.reserve(
                    Clause::OrderBy,
                    dialect,
                    aliases,
                    keys.iter().map(|k| &k.expr),
                )?;
                block.order_by = keys
                    .iter()
                    .zip(rewritten)
                    .map(|(k, expr)| SortKey {
                        expr,
                        direction: k.direction,
                        nulls: k.nulls,
                    })
                    .collect();
                Ok(block)
            }

            LogicalPlan::Fetch {
                input,
                offset,
                limit,
            } => {
                let mut block = Block::build(input, dialect, aliases)?;
                block.reserve::<std::iter::Empty<&Expr>>(
                    Clause::Fetch,
                    dialect,
                    aliases,
                    std::iter::empty(),
                )?;
                block.offset = *offset;
                block.limit = *limit;
                Ok(block)
            }
        }
    }

    /// Make room for `clause`, wrapping this block in a subquery if SQL would
    /// evaluate the new clause before one already occupied. Returns `exprs`
    /// rewritten against whatever the block now is.
    fn reserve<'e, I>(
        &mut self,
        clause: Clause,
        dialect: &dyn SqlDialect,
        aliases: &mut Aliases,
        exprs: I,
    ) -> Result<Vec<Expr>, EmitError>
    where
        I: IntoIterator<Item = &'e Expr>,
    {
        // `WHERE` merges with an existing `WHERE` by conjunction; every other
        // clause conflicts with itself.
        let conflicts = match clause {
            Clause::Where => self.occupied() > Clause::Where,
            // A projection folds into an existing select list instead.
            Clause::Select => self.occupied() > Clause::Select,
            other => self.occupied() >= other,
        };

        if conflicts {
            let alias = aliases.next();
            let inner = std::mem::replace(self, Block::source(String::new(), false, Vec::new()));
            let schema = inner.schema.clone();
            let sql = inner.render(dialect);
            *self = Block::source(
                format!("({sql}) AS {}", dialect.quote_ident(&alias)),
                false,
                schema
                    .iter()
                    .map(|c| SchemaColumn::new(&alias, &c.name, c.datatype))
                    .collect(),
            );
            // The columns now live behind the alias.
            return Ok(exprs
                .into_iter()
                .map(|e| requalify(e, &schema, &alias))
                .collect());
        }
        Ok(exprs.into_iter().cloned().collect())
    }

    /// Reduce to one side of a join. A bare join chain needs no subquery: on
    /// the left it continues the chain (`a JOIN b ON … JOIN c ON …`), and on
    /// the right it is parenthesised. Either way its tables keep their own
    /// names, which the nodes above refer to. Wrapping it in a subquery would
    /// hide those names behind an alias that nothing above uses.
    fn into_join_side(
        self,
        right: bool,
        dialect: &dyn SqlDialect,
        aliases: &mut Aliases,
    ) -> (String, Vec<SchemaColumn>) {
        if self.occupied() == Clause::From && self.multi_source {
            let from = if right {
                format!("({})", self.from)
            } else {
                self.from
            };
            return (from, self.schema);
        }
        self.into_source(dialect, aliases)
    }

    /// Reduce to something usable in a `FROM`: a bare source when nothing but
    /// `FROM` is set, otherwise a parenthesised subquery.
    fn into_source(
        self,
        dialect: &dyn SqlDialect,
        aliases: &mut Aliases,
    ) -> (String, Vec<SchemaColumn>) {
        if self.occupied() == Clause::From && !self.multi_source {
            return (self.from, self.schema);
        }
        let alias = aliases.next();
        let schema = self
            .schema
            .iter()
            .map(|c| SchemaColumn::new(&alias, &c.name, c.datatype))
            .collect();
        let sql = self.render(dialect);
        (
            format!("({sql}) AS {}", dialect.quote_ident(&alias)),
            schema,
        )
    }

    fn render(&self, dialect: &dyn SqlDialect) -> String {
        let ctx = self.ctx(dialect);

        let select_list = match &self.select {
            None => "*".to_string(),
            Some(outputs) => outputs
                .iter()
                .map(|o| {
                    let sql = format_expr(&o.expr, ctx);
                    // Alias only when it says something the expression does not.
                    if o.name.is_empty() || alias_is_redundant(&o.expr, &o.name) {
                        sql
                    } else {
                        format!("{sql} AS {}", ctx.dialect.quote_ident(&o.name))
                    }
                })
                .collect::<Vec<_>>()
                .join(", "),
        };

        let mut sql = format!("SELECT {select_list} FROM {}", self.from);

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(
                &self
                    .filters
                    .iter()
                    .map(|p| format_expr(p, ctx))
                    .collect::<Vec<_>>()
                    .join(" AND "),
            );
        }
        if !self.group_by.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(
                &self
                    .group_by
                    .iter()
                    .map(|e| format_expr(e, ctx))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&format_sort_keys(&self.order_by, ctx));
        }
        let fetch = dialect.render_fetch(self.offset, self.limit);
        if !fetch.is_empty() {
            sql.push(' ');
            sql.push_str(&fetch);
        }
        sql
    }
}

/// A join key as the joined schema exposes it — behind a subquery alias when
/// that side was wrapped.
fn resolve_key(schema: &[SchemaColumn], dataset: &str, column: &str) -> Result<Expr, EmitError> {
    // Try the declared dataset first, then the column alone: a wrapped side
    // exposes the same column name under a new qualifier.
    let index = find_column(schema, dataset, column)
        .or_else(|| find_column(schema, "", column))
        .ok_or_else(|| EmitError::ColumnNotFound(format!("{dataset}.{column}")))?;
    let found = &schema[index];
    Ok(Expr::column(&found.dataset, &found.name))
}

/// Point column references at a subquery alias.
fn requalify(expr: &Expr, inner: &[SchemaColumn], alias: &str) -> Expr {
    if let Expr::Column(c) = expr {
        if find_column(inner, &c.dataset, &c.column).is_some() {
            return Expr::column(alias, &c.column);
        }
    }
    expr.map_children(&mut |child| requalify(child, inner, alias))
}

/// Substitute references to a select list's named outputs with the
/// expressions that produce them.
fn inline(expr: &Expr, by_name: &HashMap<&str, &Expr>) -> Expr {
    if let Expr::Column(c) = expr {
        if c.dataset.is_empty() {
            if let Some(source) = by_name.get(c.column.as_str()) {
                return (*source).clone();
            }
        }
    }
    expr.map_children(&mut |child| inline(child, by_name))
}

/// Nulls placement is always spelled out: engines disagree on the default,
/// which is why the plan carries it.
fn format_sort_keys(keys: &[SortKey], ctx: SqlCtx) -> String {
    keys.iter()
        .map(|k| {
            let direction = match k.direction {
                Direction::Asc => "ASC",
                Direction::Desc => "DESC",
            };
            let nulls = match k.nulls {
                NullOrder::First => "NULLS FIRST",
                NullOrder::Last => "NULLS LAST",
            };
            format!("{} {direction} {nulls}", format_expr(&k.expr, ctx))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `name` merely restates what `expr` already produces.
fn alias_is_redundant(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Column(c) if c.dataset.is_empty() => c.column == name,
        Expr::Column(c) => name == format!("{}.{}", c.dataset, c.column),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Func;
    use crate::plan::expr::{Expr, Literal};
    use crate::plan::{LogicalPlan, NamedExpr, ScanColumn};
    use crate::sql_dialect::BigQuery;

    fn scan(dataset: &str, source: &str, columns: &[&str]) -> LogicalPlan {
        LogicalPlan::Scan {
            source: source.into(),
            dataset: dataset.into(),
            columns: columns.iter().map(|c| ScanColumn::new(*c, None)).collect(),
        }
    }

    fn count(name: &str) -> NamedExpr {
        NamedExpr::new(
            name,
            Expr::Agg {
                func: Func::Count,
                distinct: false,
                args: vec![],
            },
        )
    }

    fn minimal_aggregate() -> LogicalPlan {
        LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(scan("fact", "warehouse.public.fact_table", &["id"])),
                predicate: Expr::binary(
                    Func::Equal,
                    Expr::column("", "id"),
                    Expr::Literal(Literal::Int64(1)),
                ),
            }),
            group_by: vec![Expr::column("", "id")],
            aggregates: vec![count("row_count")],
        }
    }

    #[test]
    fn aggregate_filter_scan_sql() {
        let sql = to_sql(&minimal_aggregate()).expect("sql");
        assert!(sql.starts_with("SELECT "));
        assert!(sql.contains("FROM warehouse.public.fact_table"), "{sql}");
        assert!(sql.contains("WHERE id = 1"), "{sql}");
        assert!(sql.contains("GROUP BY id"), "{sql}");
        assert!(sql.contains("COUNT(*) AS row_count"), "{sql}");
        // One block: filter, group and select all fit in the same SELECT.
        assert!(!sql.contains("SELECT * FROM ("), "{sql}");
    }

    #[test]
    fn bigquery_uses_backtick_table_quoting() {
        let sql = to_sql_with(&minimal_aggregate(), &BigQuery).expect("sql");
        assert!(sql.contains("FROM `warehouse.public.fact_table`"), "{sql}");
        assert!(
            to_sql(&minimal_aggregate())
                .unwrap()
                .contains("FROM warehouse.public.fact_table")
        );
    }

    #[test]
    fn bigquery_renders_current_date_as_call() {
        let plan = LogicalPlan::Aggregate {
            input: Box::new(scan("t", "ds.t", &["d"])),
            group_by: vec![],
            aggregates: vec![NamedExpr::new(
                "today",
                Expr::Agg {
                    func: Func::Max,
                    distinct: false,
                    args: vec![Expr::call(Func::CurrentDate, vec![])],
                },
            )],
        };
        assert!(
            to_sql_with(&plan, &BigQuery)
                .expect("sql")
                .contains("MAX(CURRENT_DATE())")
        );
        assert!(to_sql(&plan).unwrap().contains("MAX(CURRENT_DATE)"));
    }

    /// `WHERE` runs before `GROUP BY`, so filtering an aggregate's output
    /// cannot share its block.
    #[test]
    fn filtering_after_aggregating_opens_a_subquery() {
        let plan = LogicalPlan::Filter {
            input: Box::new(LogicalPlan::Aggregate {
                input: Box::new(scan("t", "ds.t", &["a"])),
                group_by: vec![Expr::column("t", "a")],
                aggregates: vec![count("n")],
            }),
            predicate: Expr::binary(
                Func::GreaterThan,
                Expr::column("", "n"),
                Expr::Literal(Literal::Int64(5)),
            ),
        };
        let sql = to_sql(&plan).expect("sql");
        assert!(sql.starts_with("SELECT * FROM (SELECT "), "{sql}");
        // One source above the subquery, so no qualifier is needed.
        assert!(sql.contains(") AS t0 WHERE n > 5"), "{sql}");
    }

    /// The shape a fan-out-correct plan needs: aggregate one side, then join.
    #[test]
    fn an_aggregate_under_a_join_becomes_a_subquery_source() {
        let plan = LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Join {
                left: Box::new(scan("sales", "s.sales", &["amount", "store_id"])),
                right: Box::new(LogicalPlan::Aggregate {
                    input: Box::new(scan("store", "s.store", &["id", "headcount"])),
                    group_by: vec![Expr::column("store", "id")],
                    aggregates: vec![NamedExpr::new(
                        "staff",
                        Expr::Agg {
                            func: Func::Sum,
                            distinct: false,
                            args: vec![Expr::column("store", "headcount")],
                        },
                    )],
                }),
                on: vec![crate::plan::JoinKey {
                    left_dataset: "sales".into(),
                    left_column: "store_id".into(),
                    right_dataset: "store".into(),
                    right_column: "id".into(),
                }],
            }),
            group_by: vec![],
            aggregates: vec![NamedExpr::new(
                "total",
                Expr::Agg {
                    func: Func::Sum,
                    distinct: false,
                    args: vec![Expr::column("sales", "amount")],
                },
            )],
        };
        let sql = to_sql(&plan).expect("sql");
        assert!(
            sql.contains("INNER JOIN (SELECT "),
            "inner aggregate: {sql}"
        );
        assert!(sql.contains("SUM(headcount) AS staff"), "{sql}");
        // The join key follows the wrapped side behind its alias.
        assert!(sql.contains("ON sales.store_id = t0.id"), "{sql}");
        assert!(sql.contains("SUM(sales.amount) AS total"), "{sql}");
    }

    /// Three datasets join as one chain. Wrapping the first join in a
    /// subquery would hide `b` behind an alias while the select list still
    /// names `b.y`.
    #[test]
    fn a_join_chain_stays_flat() {
        let key = |l: &str, lc: &str, r: &str, rc: &str| crate::plan::JoinKey {
            left_dataset: l.into(),
            left_column: lc.into(),
            right_dataset: r.into(),
            right_column: rc.into(),
        };
        let plan = LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Join {
                left: Box::new(LogicalPlan::Join {
                    left: Box::new(scan("a", "s.a", &["b_id", "c_id"])),
                    right: Box::new(scan("b", "s.b", &["id", "y"])),
                    on: vec![key("a", "b_id", "b", "id")],
                }),
                right: Box::new(scan("c", "s.c", &["id", "z"])),
                on: vec![key("a", "c_id", "c", "id")],
            }),
            group_by: vec![Expr::column("b", "y"), Expr::column("c", "z")],
            aggregates: vec![count("n")],
        };
        let sql = to_sql(&plan).expect("sql");
        assert_eq!(
            sql,
            "SELECT b.y, c.z, COUNT(*) AS n FROM s.a INNER JOIN s.b ON a.b_id = b.id \
             INNER JOIN s.c ON a.c_id = c.id GROUP BY b.y, c.z"
        );
    }

    #[test]
    fn aggregating_an_aggregate_opens_a_subquery() {
        let plan = LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Aggregate {
                input: Box::new(scan("t", "ds.t", &["a", "b"])),
                group_by: vec![Expr::column("t", "a")],
                aggregates: vec![NamedExpr::new(
                    "inner_sum",
                    Expr::Agg {
                        func: Func::Sum,
                        distinct: false,
                        args: vec![Expr::column("t", "b")],
                    },
                )],
            }),
            group_by: vec![],
            aggregates: vec![NamedExpr::new(
                "outer_sum",
                Expr::Agg {
                    func: Func::Sum,
                    distinct: false,
                    args: vec![Expr::column("", "inner_sum")],
                },
            )],
        };
        let sql = to_sql(&plan).expect("sql");
        assert!(sql.contains("FROM (SELECT "), "{sql}");
        assert!(sql.contains("SUM(inner_sum) AS outer_sum"), "{sql}");
    }

    #[test]
    fn limiting_a_limit_opens_a_subquery() {
        let plan = LogicalPlan::Fetch {
            input: Box::new(LogicalPlan::Fetch {
                input: Box::new(scan("t", "ds.t", &["a"])),
                offset: 0,
                limit: Some(10),
            }),
            offset: 0,
            limit: Some(5),
        };
        let sql = to_sql(&plan).expect("sql");
        assert!(sql.contains("FETCH NEXT 10 ROWS ONLY) AS t0"), "{sql}");
        assert!(sql.trim_end().ends_with("FETCH NEXT 5 ROWS ONLY"), "{sql}");
    }

    /// A select list holds a composite of aggregates, so this stays flat.
    #[test]
    fn projecting_over_an_aggregate_stays_in_one_block() {
        let plan = LogicalPlan::Project {
            input: Box::new(LogicalPlan::Aggregate {
                input: Box::new(scan("t", "ds.t", &["a"])),
                group_by: vec![],
                aggregates: vec![count("__agg0"), count("__agg1")],
            }),
            exprs: vec![NamedExpr::new(
                "ratio",
                Expr::binary(
                    Func::Divide,
                    Expr::column("", "__agg0"),
                    Expr::column("", "__agg1"),
                ),
            )],
        };
        let sql = to_sql(&plan).expect("sql");
        assert!(!sql.contains("FROM ("), "should not wrap: {sql}");
        assert!(sql.contains("(COUNT(*) / COUNT(*)) AS ratio"), "{sql}");
        assert!(!sql.contains("__agg"), "lifted names must not leak: {sql}");
    }
}
