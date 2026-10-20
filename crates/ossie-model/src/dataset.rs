//! Logical datasets (`$defs.Dataset`).

use super::ai_context::AiContext;
use super::extensions::CustomExtension;
use super::field::Field;
use super::metric::Metric;
use serde::{Deserialize, Serialize};

/// Logical dataset (fact or dimension table).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_key: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unique_keys: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_context: Option<AiContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    /// Dataset-scoped metrics: aggregates over this dataset's declared fields,
    /// referenced from outside as `dataset.metric`. Fields and these metrics
    /// share one namespace.
    ///
    /// Ahead of the vendored schema: proposed upstream in apache/ossie#591 and
    /// implemented early, see ADR-0014.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_extensions: Vec<CustomExtension>,
}
