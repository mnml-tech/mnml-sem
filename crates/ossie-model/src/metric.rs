//! Metrics (`$defs.Metric`), model-scoped or dataset-scoped.

use super::ai_context::AiContext;
use super::enums::DataType;
use super::expression::Expression;
use super::extensions::CustomExtension;
use serde::{Deserialize, Serialize};

/// Quantitative measure defined on business data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub name: String,
    pub expression: Expression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Logical value type. Omit when unknown or unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datatype: Option<DataType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_context: Option<AiContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_extensions: Vec<CustomExtension>,
}
