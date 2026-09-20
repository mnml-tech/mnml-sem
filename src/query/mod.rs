//! Query request types (nouns): what the user asks for against a semantic model.

mod spec;

pub use spec::{Direction, FilterOperator, FilterSpec, NullOrder, OrderBy, SemanticQuery};
