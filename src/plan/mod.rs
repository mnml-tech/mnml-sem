//! Logical plan AST (nouns) for Ossie-backed queries.

mod display;
pub mod expr;
pub mod ossie_sql;
mod sql_format;

use crate::model::{DataType, Dialect, Expression};

pub use display::DisplayIndent;
pub use expr::{ColumnRef, Expr, JoinKey, Literal};
pub use sql_format::{format_column, format_expr, format_join_on, SqlCtx};

/// Root logical plan node (scan / join → optional filter → aggregate).
#[derive(Debug, Clone)]
pub enum LogicalPlan {
    Scan {
        /// Underlying relation id ([`Dataset::source`](crate::model::Dataset::source)).
        source: String,
        /// Logical dataset name (schema context alias).
        dataset: String,
        /// Columns this scan exposes, in order.
        columns: Vec<ScanColumn>,
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

/// A column exposed by a [`LogicalPlan::Scan`].
///
/// `datatype` is whatever the model declared. It is `None` when the field omits
/// `datatype`, which the core spec permits — consumers that need a concrete
/// type must resolve it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanColumn {
    pub name: String,
    pub datatype: Option<DataType>,
}

impl ScanColumn {
    pub fn new(name: impl Into<String>, datatype: Option<DataType>) -> Self {
        Self {
            name: name.into(),
            datatype,
        }
    }
}

/// Aggregate with output label (metric name).
#[derive(Debug, Clone, PartialEq)]
pub struct NamedAggregate {
    pub name: String,
    pub expr: Expr,
}

/// Prefer `ANSI_SQL`, else first dialect entry.
pub fn pick_expression_sql(expr: &Expression) -> Option<String> {
    pick_expression_dialect(expr).map(|(_, sql)| sql)
}

/// The dialect entry to compile, with its text.
///
/// Prefers `ANSI_SQL`, then any other dialect this crate can parse as SQL, then
/// the first entry — so a model carrying only `SNOWFLAKE` still compiles, and a
/// non-SQL dialect surfaces as a clear error rather than being silently picked
/// over a usable one.
pub fn pick_expression_dialect(expr: &Expression) -> Option<(Dialect, String)> {
    let pick = expr
        .dialects
        .iter()
        .find(|d| d.dialect == Dialect::AnsiSql)
        .or_else(|| {
            expr.dialects
                .iter()
                .find(|d| d.sql_expression().is_some() && ossie_sql::is_sql_dialect(d.dialect))
        })
        .or_else(|| expr.dialects.first())?;
    pick.sql_expression().map(|s| (pick.dialect, s.to_string()))
}
