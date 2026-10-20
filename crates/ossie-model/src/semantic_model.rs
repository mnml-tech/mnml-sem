//! Semantic model container (`$defs.SemanticModel`).

use super::ai_context::AiContext;
use super::dataset::Dataset;
use super::extensions::CustomExtension;
use super::metric::Metric;
use super::relationship::Relationship;
use serde::{Deserialize, Serialize};

/// Top-level container representing a complete semantic model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticModel {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_context: Option<AiContext>,
    pub datasets: Vec<Dataset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_extensions: Vec<CustomExtension>,
}
