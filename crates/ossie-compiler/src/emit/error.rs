//! Errors from emitting SQL or Substrait from a [`LogicalPlan`](crate::plan::LogicalPlan).

/// Emission failed (unsupported shape, invalid plan, or Substrait build error).
#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    #[error("unsupported logical plan for emission: {0}")]
    Unsupported(&'static str),

    #[error("invalid plan structure: {0}")]
    InvalidPlan(&'static str),

    #[error("column not found in plan schema: {0}")]
    ColumnNotFound(String),

    #[error("unsupported expression for Substrait: {0}")]
    UnsupportedExpression(String),
}
