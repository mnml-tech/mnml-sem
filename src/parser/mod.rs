//! Parse Ossie documents from YAML or JSON.

use crate::error::Error;
use crate::model::OssieDocument;
use std::path::Path;

/// Parse an Ossie document from a YAML string.
pub fn from_yaml_str(s: &str) -> Result<OssieDocument, Error> {
    serde_yaml::from_str(s).map_err(Error::from)
}

/// Parse an Ossie document from a JSON string.
pub fn from_json_str(s: &str) -> Result<OssieDocument, Error> {
    serde_json::from_str(s).map_err(Error::from)
}

/// Read and parse an Ossie document from a UTF-8 file (YAML or JSON by content).
pub fn from_file(path: impl AsRef<Path>) -> Result<OssieDocument, Error> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("json") {
        from_json_str(&text)
    } else {
        from_yaml_str(&text)
    }
}
