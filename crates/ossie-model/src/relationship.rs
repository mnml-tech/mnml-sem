//! Relationships between datasets (`$defs.Relationship`).

use super::ai_context::AiContext;
use super::extensions::CustomExtension;
use serde::{Deserialize, Serialize};

/// Foreign key relationship between logical datasets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub name: String,
    pub from: String,
    pub to: String,
    pub from_columns: Vec<String>,
    pub to_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_context: Option<AiContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_extensions: Vec<CustomExtension>,
}
