//! Apache Ossie — compile semantic models and queries into logical plans and SQL.
//!
//! Documents come from [`ossie_model`]; this crate lowers their expressions,
//! binds [`query::SemanticQuery`] against a [`compiled::CompiledModel`], plans
//! it into a [`plan::LogicalPlan`], and emits SQL. Substrait emission is the
//! `ossie-substrait` crate.
//!
//! Layout: **nouns** — [`query`], [`plan`]; **verbs** — [`resolver`], [`planner`], [`emit`].

#![forbid(unsafe_code)]

pub mod catalog;
pub mod compiled;
pub mod emit;
pub mod plan;
pub mod planner;
pub mod query;
pub mod resolver;
pub mod sql_dialect;

pub use resolver::QueryError;

pub mod prelude {
    pub use crate::compiled::CompiledModel;
    pub use crate::emit::EmitError;
    pub use crate::emit::sql::{to_sql, to_sql_with};
    pub use crate::plan::{DisplayIndent, LogicalPlan, pick_expression_sql};
    pub use crate::planner::build_logical_plan;
    pub use crate::query::SemanticQuery;
    pub use crate::resolver::QueryError;
    pub use crate::resolver::{BoundJoin, BoundQuery, ResolvedField, bind_query};
    pub use crate::sql_dialect::{Ansi, BigQuery, SqlDialect};
    pub use ossie_model::OssieDocument;
    pub use ossie_model::error::Error;
    pub use ossie_model::parser::{from_file, from_json_str, from_yaml_str};
    pub use ossie_model::validate::validate;
}
