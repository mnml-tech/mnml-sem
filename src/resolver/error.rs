//! Errors from resolving a [`SemanticQuery`](crate::query::SemanticQuery) against an Ossie document.

/// Resolution failed (unknown model, bad references, inconsistent base dataset, etc.).
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("semantic model not found: {0:?}")]
    ModelNotFound(String),

    #[error("dataset not found: {0:?}")]
    DatasetNotFound(String),

    #[error("field not found: {0}")]
    FieldNotFound(String),

    #[error("metric not found: {0:?}")]
    MetricNotFound(String),

    /// A reference differs from a declared name only by case. Names in a
    /// document are strings, matched as written.
    #[error("{kind} {wanted:?} not found; the model declares {declared:?}")]
    CaseMismatch {
        kind: &'static str,
        wanted: String,
        declared: String,
    },

    #[error("invalid field reference {0:?}: expected dataset.field")]
    InvalidFieldRef(String),

    #[error("inconsistent base dataset: expected {expected:?}, saw {got:?}")]
    InconsistentBaseDataset { expected: String, got: String },

    #[error(
        "`dataset` is required when `group_by` and filters are empty but metrics are requested"
    )]
    MissingBaseDataset,

    #[error(
        "`dataset` is required as the join root when the query references more than one logical dataset"
    )]
    MissingRootDataset,

    #[error("datasets are not connected by relationships: {0:?}")]
    DisconnectedDatasets(Vec<String>),

    #[error("no relationship join path from {from:?} to {to:?}")]
    NoJoinPath { from: String, to: String },

    #[error("query must request at least one `group_by` field, metric, or filter")]
    EmptyQuery,

    #[error("expression has no dialect entries usable for planning")]
    MissingExpressionSql,

    #[error("invalid expression: {0}")]
    InvalidExpression(String),

    /// The model compiled, but this field or metric did not.
    #[error("{target} could not be compiled: {reason}")]
    NotCompiled { target: String, reason: String },

    #[error("invalid filter on {field:?}: {reason}")]
    InvalidFilter { field: String, reason: String },

    /// An expression referenced a column the plan does not produce. Caught
    /// while planning, where the query is still in hand.
    #[error("column {column:?} is not in scope; available: {available}")]
    ColumnNotInScope { column: String, available: String },

    /// The join repeats the rows this aggregate reads, so the result would be
    /// larger than the truth. Refused rather than returned.
    #[error(
        "metric {metric:?} would be inflated: {agg} reads {dataset:?}, whose rows this join \
         repeats once per matching row on the many side. Aggregate {dataset:?} in its own \
         query, or use an aggregate that repeated rows do not change (MIN, MAX, or \
         COUNT(DISTINCT ...))."
    )]
    FanOut {
        metric: String,
        dataset: String,
        agg: String,
    },

    /// You can only order by what the query selects.
    #[error("cannot order by {reference:?}: it is not selected; available: {available}")]
    OrderByNotSelected {
        reference: String,
        available: String,
    },

    /// Parsed as SQL, but outside the Ossie expression profile (or not yet
    /// implemented here) — e.g. a subquery or a window function.
    #[error("unsupported expression: {0}")]
    UnsupportedExpression(String),
}
