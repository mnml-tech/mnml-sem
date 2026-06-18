//! Indented tree display for [`LogicalPlan`], similar in spirit to DataFusion's `display_indent()`.

use std::fmt::{self, Display, Formatter};

use super::{format_expr, format_join_on, SqlCtx};
use super::{LogicalPlan, NamedAggregate};
use crate::sql_dialect::Ansi;

/// Debug display always renders ANSI SQL fragments.
fn ctx(qualify: bool) -> SqlCtx<'static> {
    SqlCtx { dialect: &Ansi, qualify }
}

/// Indented, multi-line plan text (use with `format!("{}", plan.display_indent())`).
pub struct DisplayIndent<'a>(pub(super) &'a LogicalPlan);

impl Display for DisplayIndent<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write_node(f, self.0, 0, false)
    }
}

impl LogicalPlan {
    /// Return a value that formats like DataFusion's logical plan tree (operator per line, children indented).
    pub fn display_indent(&self) -> DisplayIndent<'_> {
        DisplayIndent(self)
    }
}

impl Display for LogicalPlan {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.display_indent().fmt(f)
    }
}

fn write_node(f: &mut Formatter<'_>, plan: &LogicalPlan, depth: usize, qualify: bool) -> fmt::Result {
    let pad = "  ".repeat(depth);
    match plan {
        LogicalPlan::Scan {
            source,
            dataset,
            columns,
        } => {
            writeln!(f, "{pad}Scan: {source} (dataset={dataset}, cols={})", columns.len())
        }
        LogicalPlan::Join { left, right, on } => {
            let on_s = format_join_on(on, ctx(qualify)).join(" AND ");
            writeln!(f, "{pad}Join: inner on={on_s}")?;
            write_node(f, left, depth + 1, qualify)?;
            write_node(f, right, depth + 1, qualify)
        }
        LogicalPlan::Filter { input, predicate } => {
            writeln!(f, "{pad}Filter: {}", format_expr(predicate, ctx(qualify)))?;
            write_node(f, input, depth + 1, qualify)
        }
        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            let gb = if group_by.is_empty() {
                "[]".to_string()
            } else {
                format!(
                    "[{}]",
                    group_by
                        .iter()
                        .map(|e| format_expr(e, ctx(qualify)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let ag = format_aggregates(aggregates, qualify);
            writeln!(f, "{pad}Aggregate: groupBy={gb}, aggr=[{ag}]")?;
            write_node(f, input, depth + 1, qualify)
        }
    }
}

fn format_aggregates(aggregates: &[NamedAggregate], qualify: bool) -> String {
    if aggregates.is_empty() {
        return String::new();
    }
    aggregates
        .iter()
        .map(|a| format!("{}: {}", a.name, format_expr(&a.expr, ctx(qualify))))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use crate::plan::expr::{AggFunc, Expr};
    use crate::plan::{LogicalPlan, NamedAggregate};

    #[test]
    fn display_indent_matches_datafusion_style_tree() {
        let plan = LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    source: "warehouse.public.f".into(),
                    dataset: "fact".into(),
                    columns: vec!["id".into()],
                }),
                predicate: Expr::Eq(
                    Box::new(Expr::column("", "id")),
                    Box::new(Expr::Literal(super::super::expr::Literal::Int64(1))),
                ),
            }),
            group_by: vec![Expr::column("", "id")],
            aggregates: vec![NamedAggregate {
                name: "row_count".into(),
                expr: Expr::Agg {
                    func: AggFunc::Count,
                    distinct: false,
                    arg: None,
                },
            }],
        };
        let s = format!("{}", plan.display_indent());
        assert!(
            s.contains("Aggregate:")
                && s.contains("Filter:")
                && s.contains("Scan: warehouse.public.f"),
            "{s}"
        );
    }
}
