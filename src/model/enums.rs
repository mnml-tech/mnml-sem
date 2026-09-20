//! Apache Ossie dialect and data-type enumerations.

use serde::{Deserialize, Serialize};

/// Supported SQL and expression language dialects (`$defs.Dialect`).
///
/// Mirrors the closed enum in `core-spec/ossie-schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Dialect {
    AnsiSql,
    Snowflake,
    Mdx,
    Tableau,
    Databricks,
    Maql,
    /// Google BigQuery (GoogleSQL). Wire form is `BIGQUERY`, not `BIG_QUERY`.
    #[serde(rename = "BIGQUERY")]
    BigQuery,
    Sigma,
    /// ThoughtSpot formula language. Wire form is `THOUGHTSPOT`.
    #[serde(rename = "THOUGHTSPOT")]
    ThoughtSpot,
    Dax,
}

/// Logical value type of a field or metric (`$defs.DataType`).
///
/// Declares the value type independently of role and physical representation.
/// Omit when the type is unknown; use [`DataType::Opaque`] for a known type
/// outside this vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    String,
    Integer,
    Decimal,
    Float,
    Boolean,
    Date,
    Time,
    DateTime,
    DateTimeTz,
    Opaque,
}

impl DataType {
    /// Whether this is one of the temporal types that default `is_time` to `true`.
    ///
    /// Per the spec: `Date`, `Time`, `DateTime`, `DateTimeTz`.
    pub fn is_temporal(self) -> bool {
        matches!(
            self,
            DataType::Date | DataType::Time | DataType::DateTime | DataType::DateTimeTz
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire forms are load-bearing: `SCREAMING_SNAKE_CASE` would otherwise
    /// render `BIGQUERY` as `BIG_QUERY` and `THOUGHTSPOT` as `THOUGHT_SPOT`.
    #[test]
    fn dialect_wire_forms_match_schema_enum() {
        for (variant, wire) in [
            (Dialect::AnsiSql, "ANSI_SQL"),
            (Dialect::Snowflake, "SNOWFLAKE"),
            (Dialect::Mdx, "MDX"),
            (Dialect::Tableau, "TABLEAU"),
            (Dialect::Databricks, "DATABRICKS"),
            (Dialect::Maql, "MAQL"),
            (Dialect::BigQuery, "BIGQUERY"),
            (Dialect::Sigma, "SIGMA"),
            (Dialect::ThoughtSpot, "THOUGHTSPOT"),
            (Dialect::Dax, "DAX"),
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""), "{variant:?}");
            let back: Dialect = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn datatype_wire_forms_match_schema_enum() {
        for (variant, wire) in [
            (DataType::String, "String"),
            (DataType::Integer, "Integer"),
            (DataType::Decimal, "Decimal"),
            (DataType::Float, "Float"),
            (DataType::Boolean, "Boolean"),
            (DataType::Date, "Date"),
            (DataType::Time, "Time"),
            (DataType::DateTime, "DateTime"),
            (DataType::DateTimeTz, "DateTimeTz"),
            (DataType::Opaque, "Opaque"),
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""), "{variant:?}");
        }
    }

    #[test]
    fn temporal_types_drive_is_time_default() {
        assert!(DataType::Date.is_temporal());
        assert!(DataType::Time.is_temporal());
        assert!(DataType::DateTime.is_temporal());
        assert!(DataType::DateTimeTz.is_temporal());
        assert!(!DataType::Integer.is_temporal());
        assert!(!DataType::String.is_temporal());
    }
}
