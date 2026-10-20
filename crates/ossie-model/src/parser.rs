//! Parse Ossie documents from YAML or JSON.
//!
//! Semantic models, ontologies and mapping documents each have their own
//! functions; a document's kind is not recorded in it, so the caller says which
//! one it expects.

use crate::error::Error;
use crate::{MappingDocument, OntologyDocument, OssieDocument};
use serde::de::DeserializeOwned;
use std::path::Path;

/// Parse an Ossie document from a YAML string.
pub fn from_yaml_str(s: &str) -> Result<OssieDocument, Error> {
    yaml(s)
}

/// Parse an Ossie document from a JSON string.
pub fn from_json_str(s: &str) -> Result<OssieDocument, Error> {
    json(s)
}

/// Read and parse an Ossie document from a UTF-8 file (YAML or JSON by content).
pub fn from_file(path: impl AsRef<Path>) -> Result<OssieDocument, Error> {
    file(path.as_ref())
}

/// Parse an ontology document from a YAML string.
pub fn ontology_from_yaml_str(s: &str) -> Result<OntologyDocument, Error> {
    yaml(s)
}

/// Parse an ontology document from a JSON string.
pub fn ontology_from_json_str(s: &str) -> Result<OntologyDocument, Error> {
    json(s)
}

/// Read and parse an ontology document from a UTF-8 file.
pub fn ontology_from_file(path: impl AsRef<Path>) -> Result<OntologyDocument, Error> {
    file(path.as_ref())
}

/// Parse a mapping document from a YAML string.
pub fn mapping_from_yaml_str(s: &str) -> Result<MappingDocument, Error> {
    yaml(s)
}

/// Parse a mapping document from a JSON string.
pub fn mapping_from_json_str(s: &str) -> Result<MappingDocument, Error> {
    json(s)
}

/// Read and parse a mapping document from a UTF-8 file.
pub fn mapping_from_file(path: impl AsRef<Path>) -> Result<MappingDocument, Error> {
    file(path.as_ref())
}

fn yaml<T: DeserializeOwned>(s: &str) -> Result<T, Error> {
    serde_yaml::from_str(s).map_err(Error::from)
}

fn json<T: DeserializeOwned>(s: &str) -> Result<T, Error> {
    serde_json::from_str(s).map_err(Error::from)
}

/// JSON when the extension says so, YAML otherwise.
fn file<T: DeserializeOwned>(path: &Path) -> Result<T, Error> {
    let text = std::fs::read_to_string(path)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("json") {
        json(&text)
    } else {
        yaml(&text)
    }
}
