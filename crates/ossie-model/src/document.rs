//! Root Ossie document (core metadata file).

use super::ai_context::AiContext;
use super::dataset::Dataset;
use super::extensions::CustomExtension;
use super::metric::Metric;
use super::relationship::Relationship;
use super::semantic_model::SemanticModel;
use serde::{Deserialize, Serialize};

/// The `version` value this crate targets (`ossie-schema.json` root const).
pub const OSSIE_SPEC_VERSION: &str = "0.2.0.dev0";

/// Root document for the Ossie core metadata spec.
///
/// Since `0.2.0.dev0` a standalone document carries **exactly one** model, with
/// the model's properties at the document root — the `semantic_model` array and
/// the root-level `dialects` / `vendors` declarations were removed. The reusable
/// [`SemanticModel`] shape is unchanged and is still what an ontology map embeds
/// under its own `semantic_model` property, so it stays a distinct type here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(as = "DocumentWire"))]
#[serde(from = "DocumentWire", into = "DocumentWire")]
pub struct OssieDocument {
    /// Ossie specification version (schema const [`OSSIE_SPEC_VERSION`]).
    pub version: String,
    /// The single model carried by this document.
    pub model: SemanticModel,
}

/// Flat on-the-wire shape of a standalone document.
///
/// Kept separate from [`SemanticModel`] so the root can reject unknown keys with
/// a located error; `#[serde(flatten)]` would drop the field list and position
/// from the message. Adding a field to [`SemanticModel`] without adding it here
/// is a compile error in the conversions below.
///
/// It is also the TypeScript `OssieDocument`, since it is what is on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "OssieDocument"))]
#[serde(deny_unknown_fields)]
struct DocumentWire {
    version: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ai_context: Option<AiContext>,
    datasets: Vec<Dataset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relationships: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<CustomExtension>,
}

impl From<DocumentWire> for OssieDocument {
    fn from(w: DocumentWire) -> Self {
        Self {
            version: w.version,
            model: SemanticModel {
                name: w.name,
                description: w.description,
                ai_context: w.ai_context,
                datasets: w.datasets,
                relationships: w.relationships,
                metrics: w.metrics,
                custom_extensions: w.custom_extensions,
            },
        }
    }
}

impl From<OssieDocument> for DocumentWire {
    fn from(d: OssieDocument) -> Self {
        let SemanticModel {
            name,
            description,
            ai_context,
            datasets,
            relationships,
            metrics,
            custom_extensions,
        } = d.model;
        Self {
            version: d.version,
            name,
            description,
            ai_context,
            datasets,
            relationships,
            metrics,
            custom_extensions,
        }
    }
}
