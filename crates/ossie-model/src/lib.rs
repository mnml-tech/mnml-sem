//! Apache Ossie documents — types, parsing and structural validation.
//!
//! See the [Apache Ossie](https://github.com/apache/ossie) core schema
//! (`core-spec/ossie-schema.json`) for the authoritative document shape. This
//! crate targets [`OSSIE_SPEC_VERSION`], and its types mirror the schema
//! (ADR-0002).
//!
//! Ossie was previously named Open Semantic Interchange (OSI).
//!
//! Nothing here compiles an expression: that needs the model as context and
//! lives in `ossie-compiler`.

#![forbid(unsafe_code)]

mod ai_context;
mod dataset;
mod document;
mod enums;
mod expression;
mod extensions;
mod field;
mod metric;
mod relationship;
mod semantic_model;

pub mod error;
pub mod parser;
pub mod validate;

pub use ai_context::{AiContext, AiContextStructured};
pub use dataset::Dataset;
pub use document::{OSSIE_SPEC_VERSION, OssieDocument};
pub use enums::{DataType, Dialect};
pub use expression::{DialectExpression, Expression};
pub use extensions::CustomExtension;
pub use field::{Field, FieldDimension};
pub use metric::Metric;
pub use relationship::Relationship;
pub use semantic_model::SemanticModel;

pub use error::Error;
