//! Emit ANSI-style SQL strings from a [`LogicalPlan`](crate::plan::LogicalPlan).
//!
//! SQL is derived from the structured plan (secondary to Substrait).

use std::collections::HashMap;

use crate::emit::EmitError;
use crate::plan::{format_expr, format_join_on, SqlCtx};
use crate::plan::{Expr, LogicalPlan, NamedExpr};
use crate::sql_dialect::{Ansi, SqlDialect};

/// Build a single ANSI `SELECT` statement for the plan tree.
pub fn to_sql(plan: &LogicalPlan) -> Result<String, EmitError> {
    to_sql_with(plan, &Ansi)
}

/// Build a single `SELECT` statement for the plan tree in the given SQL `dialect`.
///
/// A [`LogicalPlan::Project`] over an aggregate is flattened rather than
/// emitted as a subquery: SQL allows a composite of aggregates directly in the
/// select list, so the measures the planner lifted out are put back.
pub fn to_sql_with(plan: &LogicalPlan, dialect: &dyn SqlDialect) -> Result<String, EmitError> {
    let (aggregate, projection) = match plan {
        LogicalPlan::Aggregate { .. } => (plan, None),
        LogicalPlan::Project { input, exprs } if matches!(**input, LogicalPlan::Aggregate { .. }) => {
            (input.as_ref(), Some(exprs.as_slice()))
        }
        _ => {
            return Err(EmitError::Unsupported(
                "root must be Aggregate, or Project over Aggregate (use planner output)",
            ));
        }
    };

    let LogicalPlan::Aggregate {
        input,
        group_by,
        aggregates,
    } = aggregate
    else {
        unreachable!("matched above");
    };

    let ctx = SqlCtx {
        dialect,
        qualify: is_multi_table(input),
    };

    let outputs: Vec<NamedExpr> = match projection {
        None => aggregates.clone(),
        Some(exprs) => {
            let measures: HashMap<&str, &Expr> = aggregates
                .iter()
                .map(|a| (a.name.as_str(), &a.expr))
                .collect();
            exprs
                .iter()
                .map(|e| NamedExpr::new(&e.name, inline_measures(&e.expr, &measures)))
                .collect()
        }
    };

    emit_aggregate_root(input.as_ref(), group_by, &outputs, ctx)
}

/// Substitute references to lifted measures back with the aggregates
/// themselves, so the select list reads as it was written in the model.
fn inline_measures(expr: &Expr, measures: &HashMap<&str, &Expr>) -> Expr {
    if let Expr::Column(c) = expr {
        if c.dataset.is_empty() {
            if let Some(agg) = measures.get(c.column.as_str()) {
                return (*agg).clone();
            }
        }
    }
    expr.map_children(&mut |child| inline_measures(child, measures))
}

fn is_multi_table(plan: &LogicalPlan) -> bool {
    match plan {
        LogicalPlan::Join { .. } => true,
        LogicalPlan::Filter { input, .. } => is_multi_table(input),
        _ => false,
    }
}

fn emit_aggregate_root(
    input: &LogicalPlan,
    group_by: &[Expr],
    outputs: &[NamedExpr],
    ctx: SqlCtx,
) -> Result<String, EmitError> {
    let (from_sql, where_sql) = from_and_where(input, ctx)?;

    let mut select_items: Vec<String> = group_by.iter().map(|e| format_expr(e, ctx)).collect();

    for o in outputs {
        select_items.push(format!(
            "{} AS {}",
            format_expr(&o.expr, ctx),
            ctx.dialect.quote_ident(&o.name)
        ));
    }

    if select_items.is_empty() {
        return Err(EmitError::InvalidPlan("empty SELECT"));
    }

    let select_list = select_items.join(", ");
    let mut sql = format!("SELECT {select_list} FROM {from_sql}");

    if let Some(w) = where_sql {
        sql.push_str(" WHERE ");
        sql.push_str(&w);
    }

    if !group_by.is_empty() {
        sql.push_str(" GROUP BY ");
        sql.push_str(
            &group_by
                .iter()
                .map(|e| format_expr(e, ctx))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    Ok(sql)
}

fn from_and_where(plan: &LogicalPlan, ctx: SqlCtx) -> Result<(String, Option<String>), EmitError> {
    match plan {
        LogicalPlan::Filter { input, predicate } => {
            let (from, preds) = collect_filters(input.as_ref(), vec![predicate.clone()], ctx)?;
            let where_clause = match preds.len() {
                0 => None,
                1 => Some(format_expr(&preds[0], ctx)),
                _ => Some(
                    preds
                        .iter()
                        .map(|p| format_expr(p, ctx))
                        .collect::<Vec<_>>()
                        .join(" AND "),
                ),
            };
            Ok((from, where_clause))
        }
        _ => {
            let from = from_join_or_scan(plan, ctx)?;
            Ok((from, None))
        }
    }
}

fn collect_filters(
    plan: &LogicalPlan,
    mut preds: Vec<crate::plan::Expr>,
    ctx: SqlCtx,
) -> Result<(String, Vec<crate::plan::Expr>), EmitError> {
    match plan {
        LogicalPlan::Filter { input, predicate } => {
            preds.push(predicate.clone());
            collect_filters(input.as_ref(), preds, ctx)
        }
        _ => {
            let from = from_join_or_scan(plan, ctx)?;
            Ok((from, preds))
        }
    }
}

fn from_join_or_scan(plan: &LogicalPlan, ctx: SqlCtx) -> Result<String, EmitError> {
    match plan {
        LogicalPlan::Scan { source, .. } => Ok(ctx.dialect.quote_source(source)),
        LogicalPlan::Join { left, right, on } => {
            let l = from_join_or_scan(left.as_ref(), ctx)?;
            let r = from_join_or_scan(right.as_ref(), ctx)?;
            let on_clause = format_join_on(on, ctx).join(" AND ");
            Ok(format!("{l} INNER JOIN {r} ON {on_clause}"))
        }
        LogicalPlan::Filter { .. } => Err(EmitError::InvalidPlan(
            "Filter must be peeled before FROM",
        )),
        LogicalPlan::Aggregate { .. } | LogicalPlan::Project { .. } => Err(
            EmitError::Unsupported("nested Aggregate/Project under FROM"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Func;
    use crate::plan::expr::{Expr, Literal};
    use crate::plan::{LogicalPlan, ScanColumn};
    use crate::sql_dialect::BigQuery;

    fn minimal_aggregate() -> LogicalPlan {
        LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    source: "warehouse.public.fact_table".into(),
                    dataset: "fact".into(),
                    columns: vec![ScanColumn::new("id", None)],
                }),
                predicate: Expr::binary(
                    Func::Equal,
                    Expr::column("", "id"),
                    Expr::Literal(Literal::Int64(1)),
                ),
            }),
            group_by: vec![Expr::column("", "id")],
            aggregates: vec![crate::plan::NamedExpr {
                name: "row_count".into(),
                expr: Expr::Agg {
                    func: Func::Count,
                    distinct: false,
                    args: vec![],
                },
            }],
        }
    }

    #[test]
    fn aggregate_filter_scan_sql() {
        let sql = to_sql(&minimal_aggregate()).expect("sql");
        assert!(sql.starts_with("SELECT "));
        assert!(sql.contains("FROM warehouse.public.fact_table"), "{sql}");
        assert!(sql.contains("WHERE id = 1"));
        assert!(sql.contains("GROUP BY id"));
        assert!(sql.contains("COUNT(*) AS row_count"));
    }

    #[test]
    fn bigquery_uses_backtick_table_quoting() {
        let sql = to_sql_with(&minimal_aggregate(), &BigQuery).expect("sql");
        assert!(sql.contains("FROM `warehouse.public.fact_table`"), "{sql}");
        // ANSI treats the same source as a qualified name, part by part.
        assert!(to_sql(&minimal_aggregate())
            .unwrap()
            .contains("FROM warehouse.public.fact_table"));
    }

    #[test]
    fn bigquery_renders_current_date_as_call() {
        let plan = LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Scan {
                source: "ds.t".into(),
                dataset: "t".into(),
                columns: vec![ScanColumn::new("d", None)],
            }),
            group_by: vec![],
            aggregates: vec![crate::plan::NamedExpr {
                name: "today".into(),
                expr: Expr::Agg {
                    func: Func::Max,
                    distinct: false,
                    args: vec![Expr::call(Func::CurrentDate, vec![])],
                },
            }],
        };
        let sql = to_sql_with(&plan, &BigQuery).expect("sql");
        assert!(sql.contains("MAX(CURRENT_DATE())"), "{sql}");
        assert!(to_sql(&plan).unwrap().contains("MAX(CURRENT_DATE)"));
    }
}
