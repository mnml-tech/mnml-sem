//! Query request shape ([`SemanticQuery`]): dimensions, metrics, filters.
//!
//! Which [`SemanticModel`](crate::model::SemanticModel) to use is chosen by the caller (e.g. HTTP path or host API), not embedded here.
//! Field references use `dataset.field` (logical dataset name and field name).

use serde::{Deserialize, Serialize};

/// Query body: `GROUP BY` fields, metrics, filters. Model identity is provided separately to [`crate::resolver::bind_query`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticQuery {
    /// Field references (`dataset.field`) for `GROUP BY`.
    #[serde(default, alias = "groupBy")]
    pub group_by: Vec<String>,
    /// Metric names defined on the semantic model.
    #[serde(default)]
    pub metrics: Vec<String>,
    #[serde(default)]
    pub filters: Vec<FilterSpec>,
    /// When no field references appear in `group_by` or filters, selects the scan target for metrics-only queries.
    #[serde(default)]
    pub dataset: Option<String>,
    /// Ordering, applied to what the query selects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_by: Vec<OrderBy>,
    /// Maximum rows to return. Omitted means all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// Rows to skip before the first returned row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

/// One ordering term.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderBy {
    /// Something the query selects: a grouped `dataset.field` (or its bare
    /// field name) or a metric name. Not an arbitrary expression — you order
    /// by what you asked for.
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub direction: Direction,
    /// Where nulls sort. Omitted means they sort as greater than non-nulls:
    /// last when ascending, first when descending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nulls: Option<NullOrder>,
}

impl OrderBy {
    #[must_use]
    pub fn asc(reference: impl Into<String>) -> Self {
        Self {
            reference: reference.into(),
            direction: Direction::Asc,
            nulls: None,
        }
    }

    #[must_use]
    pub fn desc(reference: impl Into<String>) -> Self {
        Self {
            reference: reference.into(),
            direction: Direction::Desc,
            nulls: None,
        }
    }

    /// Where nulls go, resolving the default.
    #[must_use]
    pub fn null_order(&self) -> NullOrder {
        self.nulls.unwrap_or(match self.direction {
            Direction::Asc => NullOrder::Last,
            Direction::Desc => NullOrder::First,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Asc,
    Desc,
}

/// Engines disagree on where nulls sort by default, and Substrait makes the
/// choice part of the sort direction, so it is stated rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullOrder {
    First,
    Last,
}

/// Comparison used by a [`FilterSpec`].
///
/// Deserialized from the query JSON, so an unrecognized spelling fails there
/// rather than silently degrading to equality. Symbol aliases are accepted for
/// the operators that have a conventional one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    #[serde(alias = "=")]
    Eq,
    #[serde(alias = "!=", alias = "<>")]
    Ne,
    #[serde(alias = "<")]
    Lt,
    #[serde(alias = "<=")]
    Lte,
    #[serde(alias = ">")]
    Gt,
    #[serde(alias = ">=")]
    Gte,
    /// `value` must be a JSON array of literals.
    In,
    /// `value` must be a JSON array of literals.
    NotIn,
    /// `value` is ignored.
    IsNull,
    /// `value` is ignored.
    IsNotNull,
    /// Column-to-column equality; `value` is a `dataset.field` reference string.
    #[serde(alias = "eq_column")]
    EqField,
}

/// Predicate on a field (`dataset.field`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterSpec {
    /// `dataset.field`
    pub field: String,
    /// Omitted means [`FilterOperator::In`] when `value` is a JSON array and
    /// [`FilterOperator::Eq`] otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<FilterOperator>,
    /// Omitted is `null`, which is what the null-checking operators expect.
    #[serde(default)]
    pub value: serde_json::Value,
}

impl FilterSpec {
    #[must_use]
    pub fn new(field: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            field: field.into(),
            operator: None,
            value,
        }
    }

    /// The operator to apply, resolving the shape-based default.
    #[must_use]
    pub fn operator(&self) -> FilterOperator {
        self.operator.unwrap_or(if self.value.is_array() {
            FilterOperator::In
        } else {
            FilterOperator::Eq
        })
    }
}
