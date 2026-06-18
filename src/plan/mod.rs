//! Logical plan AST (nouns) for OSI-backed queries.

mod display;
pub mod expr;
pub mod mnml;
mod sql_format;

use crate::model::{Dialect, Expression};

pub use display::DisplayIndent;
pub use expr::{AggFunc, ColumnRef, Expr, JoinKey, Literal, parse_column_sql, parse_metric_sql};
pub use sql_format::{format_column, format_expr, format_join_on, SqlCtx};

/// Root logical plan node (scan / join → optional filter → aggregate).
#[derive(Debug, Clone)]
pub enum LogicalPlan {
    Scan {
        /// Underlying relation id ([`Dataset::source`](crate::model::Dataset::source)).
        source: String,
        /// Logical dataset name (schema context alias).
        dataset: String,
        /// Physical column names exposed by this scan (from OSI field expressions).
        columns: Vec<String>,
    },
    Join {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
        /// Equi-join keys.
        on: Vec<JoinKey>,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr,
    },
    Aggregate {
        input: Box<LogicalPlan>,
        group_by: Vec<Expr>,
        aggregates: Vec<NamedAggregate>,
    },
}

/// Aggregate with output label (metric name).
#[derive(Debug, Clone, PartialEq)]
pub struct NamedAggregate {
    pub name: String,
    pub expr: Expr,
}

/// Prefer `ANSI_SQL`, else first dialect entry.
pub fn pick_expression_sql(expr: &Expression) -> Option<String> {
    expr.dialects
        .iter()
        .find(|d| d.dialect == Dialect::AnsiSql)
        .or_else(|| expr.dialects.first())
        .and_then(|d| d.sql_expression().map(|s| s.to_string()))
}
