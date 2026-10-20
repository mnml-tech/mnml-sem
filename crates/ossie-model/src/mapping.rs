//! Mapping documents (`ontology/mapping.json`).
//!
//! A mapping maps the datasets of one semantic model onto one ontology. It
//! names both rather than embedding either, so the three documents are written
//! and versioned separately.

use serde::{Deserialize, Serialize};

use crate::extensions::CustomExtension;
use crate::ontology::ConceptMapping;

/// The `version` value mapping documents carry (`mapping.json` root const).
pub const MAPPING_SPEC_VERSION: &str = "0.2.0.dev0";

/// Root of a mapping document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingDocument {
    pub version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The ontology this mapping populates.
    pub ontology_ref: DocumentReference,
    /// The semantic model whose fields it reads.
    pub semantic_model_ref: DocumentReference,
    pub concept_mappings: Vec<ConceptMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_extensions: Vec<CustomExtension>,
}

/// Another document, by name (`$defs.DocumentReference`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReference {
    /// Must equal the referenced document's own `name`.
    pub name: String,
    /// Where to resolve it from, relative to the mapping or absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}
