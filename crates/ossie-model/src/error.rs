//! Errors for parsing and validation.

/// Errors surfaced by parsing, validation, or interchange operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("validation failed:\n{}", _0.join("\n"))]
    Validation(Vec<String>),

    #[error("not yet implemented")]
    NotImplemented,
}
