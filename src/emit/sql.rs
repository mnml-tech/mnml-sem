//! Emit ANSI-style SQL strings from a [`LogicalPlan`](crate::plan::LogicalPlan).
//!
//! SQL is derived from the structured plan (secondary to Substrait).

use crate::emit::EmitError;
use crate::plan::{format_expr, format_join_on, SqlCtx};
use crate::plan::{LogicalPlan, NamedAggregate};
use crate::sql_dialect::{Ansi, SqlDialect};

/// Build a single ANSI `SELECT` statement for the plan tree.
pub fn to_sql(plan: &LogicalPlan) -> Result<String, EmitError> {
    to_sql_with(plan, &Ansi)
}

/// Build a single `SELECT` statement for the plan tree in the given SQL `dialect`.
pub fn to_sql_with(plan: &LogicalPlan, dialect: &dyn SqlDialect) -> Result<String, EmitError> {
    match plan {
        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            let ctx = SqlCtx {
                dialect,
                qualify: is_multi_table(input),
            };
            emit_aggregate_root(input.as_ref(), group_by, aggregates, ctx)
        }
        _ => Err(EmitError::Unsupported(
            "root must be Aggregate (use planner output)",
        )),
    }
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
    group_by: &[crate::plan::Expr],
    aggregates: &[NamedAggregate],
    ctx: SqlCtx,
) -> Result<String, EmitError> {
    let (from_sql, where_sql) = from_and_where(input, ctx)?;

    let mut select_items: Vec<String> = group_by.iter().map(|e| format_expr(e, ctx)).collect();

    for a in aggregates {
        select_items.push(format!(
            "{} AS {}",
            format_expr(&a.expr, ctx),
            ctx.dialect.quote_ident(&a.name)
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
        LogicalPlan::Aggregate { .. } => Err(EmitError::Unsupported(
            "nested Aggregate under FROM",
        )),
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
            aggregates: vec![crate::plan::NamedAggregate {
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
            aggregates: vec![crate::plan::NamedAggregate {
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
