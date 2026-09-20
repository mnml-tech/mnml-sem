//! Resolve a [`SemanticQuery`](crate::query::SemanticQuery) to concrete Ossie definitions (verb).

mod bind;
mod error;

pub use bind::{BoundFilter, BoundJoin, BoundQuery, ResolvedField, bind_query};
pub use error::QueryError;
