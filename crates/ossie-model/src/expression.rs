//! Multi-dialect expressions (`$defs.DialectExpression`, `$defs.Expression`).
//!
//! A dialect entry is `{ dialect, expression }` and nothing else, matching the
//! schema's `additionalProperties: false`. The text is compiled by
//! `ossie-compiler` (`plan::ossie_sql`).

use super::enums::Dialect;
use serde::{Deserialize, Serialize};

/// Expression in a specific dialect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DialectExpression {
    pub dialect: Dialect,
    pub expression: String,
}

impl DialectExpression {
    pub fn sql_expression(&self) -> Option<&str> {
        Some(self.expression.as_str())
    }

    pub fn ansi_sql(expression: impl Into<String>) -> Self {
        Self {
            dialect: Dialect::AnsiSql,
            expression: expression.into(),
        }
    }
}

/// Expression definition with multi-dialect support (at least one dialect per schema).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Expression {
    pub dialects: Vec<DialectExpression>,
}
