//! AI context (`$defs.AIContext`): string or structured object with optional extra keys.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Additional context for AI tools: plain string or structured object.
///
/// The object form allows `instructions`, `synonyms`, `examples`, and any other keys
/// (JSON Schema `additionalProperties: true` on the object branch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AiContext {
    Text(String),
    Structured(AiContextStructured),
}

/// Structured AI context with known fields plus arbitrary extension properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiContextStructured {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synonyms: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, JsonValue>,
}
