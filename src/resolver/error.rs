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

    /// Parsed as SQL, but outside the Ossie expression profile (or not yet
    /// implemented here) — e.g. a subquery or a window function.
    #[error("unsupported expression: {0}")]
    UnsupportedExpression(String),
}
