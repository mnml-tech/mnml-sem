//! Logical plan AST (nouns) for Ossie-backed queries.

mod display;
pub mod expr;
pub mod ossie_sql;
mod sql_format;

use crate::model::{DataType, Dialect, Expression};

pub use display::DisplayIndent;
pub use expr::{ColumnRef, Expr, JoinKey, Literal};
pub use sql_format::{SqlCtx, format_column, format_expr, format_join_on};

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
        /// One bare aggregate per output column.
        aggregates: Vec<NamedExpr>,
    },
    /// Compute output columns from the input's.
    ///
    /// Used above an [`LogicalPlan::Aggregate`] when a metric is a composite of
    /// aggregates (`SUM(a) / COUNT(b)`), which a Substrait measure cannot hold.
    /// The aggregates become measures and this recombines them.
    Project {
        input: Box<LogicalPlan>,
        /// Every output column, in order — including grouping columns that
        /// pass through unchanged.
        exprs: Vec<NamedExpr>,
    },
}

/// A column produced by a plan node: its dataset (empty when the node
/// synthesized it) and its name.
pub type SchemaColumn = (String, String);

impl LogicalPlan {
    /// The columns this node produces, in order.
    ///
    /// This is the one definition of a node's output. The planner checks
    /// expressions against it and the Substrait emitter resolves field
    /// references with it, so the two cannot drift.
    pub fn output_schema(&self) -> Vec<SchemaColumn> {
        match self {
            LogicalPlan::Scan {
                dataset, columns, ..
            } => columns
                .iter()
                .map(|c| (dataset.clone(), c.name.clone()))
                .collect(),
            LogicalPlan::Join { left, right, .. } => {
                let mut out = left.output_schema();
                out.extend(right.output_schema());
                out
            }
            LogicalPlan::Filter { input, .. } => input.output_schema(),
            LogicalPlan::Aggregate {
                group_by,
                aggregates,
                ..
            } => {
                // One output per grouping, then one per measure. A grouping
                // that is not a plain column still takes a slot, or every
                // measure index below it shifts.
                let mut out: Vec<SchemaColumn> = group_by
                    .iter()
                    .enumerate()
                    .map(|(i, e)| match e {
                        Expr::Column(c) => (c.dataset.clone(), c.column.clone()),
                        _ => (String::new(), format!("__group{i}")),
                    })
                    .collect();
                out.extend(aggregates.iter().map(|a| (String::new(), a.name.clone())));
                out
            }
            LogicalPlan::Project { exprs, .. } => exprs
                .iter()
                .map(|e| match &e.expr {
                    // A pass-through column keeps its identity.
                    Expr::Column(c) => (c.dataset.clone(), c.column.clone()),
                    _ => (String::new(), e.name.clone()),
                })
                .collect(),
        }
    }
}

/// Position of `(dataset, column)` in a schema.
///
/// An empty `dataset` matches on column name alone, which is how a
/// single-source query refers to its columns.
pub fn find_column(schema: &[SchemaColumn], dataset: &str, column: &str) -> Option<usize> {
    if dataset.is_empty() {
        return schema.iter().position(|(_, c)| c == column);
    }
    schema.iter().position(|(d, c)| d == dataset && c == column)
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

/// An expression with the name of the column it produces.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedExpr {
    pub name: String,
    pub expr: Expr,
}

impl NamedExpr {
    pub fn new(name: impl Into<String>, expr: Expr) -> Self {
        Self {
            name: name.into(),
            expr,
        }
    }
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
