//! MNML structured expression typing shared by the dialect compiler.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MnmlExpressionKind {
    FieldExpr,
    Metric,
}

/// Shorthand metric aggregate definition inside `expr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MnmlMetricDef {
    pub agg: String,
    #[serde(default)]
    pub distinct: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg: Option<serde_json::Value>,
}

/// Returns true when JSON looks like MNML dataset storage (`format`, no `kind`).
pub fn is_mnml_storage_json(data: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
        return false;
    };
    v.get("format").is_some() && v.get("kind").is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_storage_from_non_storage() {
        assert!(is_mnml_storage_json(
            r#"{"version":1,"format":"parquet","location":"x/"}"#
        ));
        assert!(!is_mnml_storage_json(
            r#"{"version":1,"kind":"metric","expr":{"agg":"count"}}"#
        ));
    }
}
