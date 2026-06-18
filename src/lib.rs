//! Open Semantic Interchange (OSI) — types, parsing, validation, and query planning for the core metadata spec.
//!
//! See the [Open Semantic Interchange](https://github.com/open-semantic-interchange/OSI) core
//! schema (`osi-schema.json`) for the authoritative document shape.
//!
//! Layout: **nouns** — [`model`], [`query`], [`plan`]; **verbs** — [`parser`], [`resolver`], [`planner`], [`validate`].

#![forbid(unsafe_code)]

pub mod catalog;
pub mod emit;
pub mod error;
pub mod model;
pub mod parser;
pub mod plan;
pub mod planner;
pub mod query;
pub mod resolver;
pub(crate) mod sql_quote;
pub mod validate;

pub use resolver::QueryError;

pub mod prelude {
    pub use crate::emit::sql::to_sql;
    pub use crate::emit::EmitError;
    pub use crate::error::Error;
    pub use crate::model::OsiDocument;
    pub use crate::parser::{from_file, from_json_str, from_yaml_str};
    pub use crate::plan::{DisplayIndent, LogicalPlan, pick_expression_sql};
    pub use crate::planner::build_logical_plan;
    pub use crate::query::SemanticQuery;
    pub use crate::resolver::QueryError;
    pub use crate::resolver::{BoundJoin, BoundQuery, ResolvedField, bind_query};
    pub use crate::validate::validate;

    #[cfg(feature = "substrait")]
    pub use crate::emit::substrait::{encode_plan, to_plan};
}
