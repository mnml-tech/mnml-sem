//! Emit SQL from a [`LogicalPlan`](crate::plan::LogicalPlan).
//!
//! - **SQL** ([`sql`]): derived from the structured plan; predicates and
//!   aggregates are passed through as SQL text (same representation as in the
//!   planner).
//! - **Substrait** is the `ossie-substrait` crate, which reads the same plan
//!   and reports failures as [`EmitError`].

mod error;
pub mod sql;

pub use error::EmitError;
