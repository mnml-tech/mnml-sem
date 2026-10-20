//! Logical dataset fields (`$defs.Field`, field-level dimension metadata).

use super::ai_context::AiContext;
use super::enums::DataType;
use super::expression::Expression;
use super::extensions::CustomExtension;
use serde::{Deserialize, Serialize};

/// Dimension metadata on a field (`$defs.Dimension`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FieldDimension {
    /// Temporal-**role** marker, independent of the field's data type.
    ///
    /// When unset the effective value is derived from `datatype`; see
    /// [`Field::is_time_dimension`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_time: Option<bool>,
}

/// Row-level attribute for grouping, filtering, and metric expressions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub expression: Expression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimension: Option<FieldDimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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

impl Field {
    /// Effective temporal role of this field.
    ///
    /// An explicit `dimension.is_time` always wins. Otherwise it defaults to
    /// `true` for a temporal `datatype` (`Date`, `Time`, `DateTime`,
    /// `DateTimeTz`) and `false` for everything else — including when
    /// `datatype` is absent.
    pub fn is_time_dimension(&self) -> bool {
        match self.dimension.as_ref().and_then(|d| d.is_time) {
            Some(explicit) => explicit,
            None => self.datatype.is_some_and(DataType::is_temporal),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{DataType, Field};

    fn field(yaml: &str) -> Field {
        serde_yaml::from_str(yaml).expect("parse field")
    }

    const EXPR: &str = "expression:\n  dialects:\n    - dialect: ANSI_SQL\n      expression: x\n";

    #[test]
    fn temporal_datatype_defaults_is_time_true() {
        for dt in ["Date", "Time", "DateTime", "DateTimeTz"] {
            let f = field(&format!("name: f\n{EXPR}datatype: {dt}\n"));
            assert!(f.is_time_dimension(), "{dt} should default to a time role");
        }
    }

    #[test]
    fn non_temporal_datatype_defaults_is_time_false() {
        let f = field(&format!("name: f\n{EXPR}datatype: Integer\n"));
        assert!(!f.is_time_dimension());
    }

    #[test]
    fn absent_datatype_defaults_is_time_false() {
        let f = field(&format!("name: f\n{EXPR}"));
        assert!(!f.is_time_dimension());
    }

    #[test]
    fn explicit_is_time_overrides_the_datatype_default() {
        // Audit timestamp opting out of the temporal default.
        let f = field(&format!(
            "name: created_at\n{EXPR}datatype: DateTime\ndimension:\n  is_time: false\n"
        ));
        assert!(!f.is_time_dimension());

        // Integer year grain opting in.
        let f = field(&format!(
            "name: d_year\n{EXPR}datatype: Integer\ndimension:\n  is_time: true\n"
        ));
        assert!(f.is_time_dimension());
    }

    #[test]
    fn datatype_round_trips() {
        let f = field(&format!("name: f\n{EXPR}datatype: DateTimeTz\n"));
        assert_eq!(f.datatype, Some(DataType::DateTimeTz));
    }
}
