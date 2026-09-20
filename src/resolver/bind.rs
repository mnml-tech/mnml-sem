//! Bind a [`SemanticQuery`](crate::query::SemanticQuery) against a [`CompiledModel`].

use std::collections::{HashMap, HashSet};

use crate::compiled::{CompiledDataset, CompiledField, CompiledMetric, CompiledModel, IssueTarget};
use crate::model::Relationship;
use crate::plan::JoinKey;
use crate::query::{FilterOperator, SemanticQuery};
use crate::resolver::QueryError;

/// A compiled field together with the dataset it belongs to.
#[derive(Debug, Clone)]
pub struct ResolvedField {
    pub dataset: String,
    pub field: CompiledField,
}

/// Filter with a [`ResolvedField`].
#[derive(Debug, Clone)]
pub struct BoundFilter {
    pub field: ResolvedField,
    pub operator: FilterOperator,
    pub value: serde_json::Value,
    /// Set when `operator` is [`FilterOperator::EqField`]: the RHS field.
    pub rhs_field: Option<ResolvedField>,
}

/// One INNER JOIN step (left-deep): `left_dataset` already in the tree, add `right_dataset`.
#[derive(Debug, Clone)]
pub struct BoundJoin {
    pub left_dataset: String,
    pub right_dataset: String,
    /// Equi-join keys for this step.
    pub on: Vec<JoinKey>,
}

/// Fully bound query: one or more logical datasets joined by relationships.
#[derive(Debug, Clone)]
pub struct BoundQuery {
    pub model_name: String,
    /// Anchor scan / join root.
    pub root_dataset: String,
    /// Logical dataset name → physical `source` for every table in the join.
    pub sources: HashMap<String, String>,
    /// Ordered INNER JOINs after the root scan.
    pub joins: Vec<BoundJoin>,
    pub group_by: Vec<ResolvedField>,
    pub metrics: Vec<CompiledMetric>,
    pub filters: Vec<BoundFilter>,
    /// Trailing clauses, resolved against the plan's output by the planner.
    pub order_by: Vec<crate::query::OrderBy>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

/// Resolve `spec` against `model`.
pub fn bind_query(model: &CompiledModel, spec: &SemanticQuery) -> Result<BoundQuery, QueryError> {
    let mut paths: Vec<String> = Vec::new();
    paths.extend(spec.group_by.iter().cloned());
    paths.extend(spec.filters.iter().map(|f| f.field.clone()));
    paths.extend(spec.filters.iter().filter_map(|f| {
        (f.operator() == FilterOperator::EqField)
            .then(|| f.value.as_str().map(str::to_string))
            .flatten()
    }));

    let path_datasets: HashSet<String> = paths
        .iter()
        .map(|p| {
            let (ds, _) = parse_field_ref(p)?;
            Ok::<_, QueryError>(ds.to_string())
        })
        .collect::<Result<_, _>>()?;

    let root = if path_datasets.is_empty() {
        if spec.metrics.is_empty() {
            return Err(QueryError::EmptyQuery);
        }
        spec.dataset.clone().ok_or(QueryError::MissingBaseDataset)?
    } else if path_datasets.len() == 1 {
        spec.dataset
            .clone()
            .unwrap_or_else(|| path_datasets.iter().next().unwrap().clone())
    } else {
        spec.dataset.clone().ok_or(QueryError::MissingRootDataset)?
    };

    let metrics: Vec<CompiledMetric> = spec
        .metrics
        .iter()
        .map(|name| {
            model.metric(name).cloned().ok_or_else(|| {
                // A metric that failed to compile is absent here; say why
                // rather than claiming it does not exist.
                match model.issue_for(&IssueTarget::metric(name)) {
                    Some(issue) => QueryError::NotCompiled {
                        target: issue.target.to_string(),
                        reason: issue.message.clone(),
                    },
                    None => QueryError::MetricNotFound(name.clone()),
                }
            })
        })
        .collect::<Result<_, _>>()?;

    let mut required: HashSet<String> = path_datasets;
    required.insert(root.clone());
    // Ossie metrics may span datasets, so a metric's own expression decides
    // which joins the query needs even when nothing is grouped or filtered on
    // the far side. Compilation already worked this out.
    for m in &metrics {
        required.extend(m.datasets.iter().cloned());
    }

    let dataset_by_name: HashMap<String, &CompiledDataset> =
        model.datasets.iter().map(|d| (d.name.clone(), d)).collect();

    for name in &required {
        if !dataset_by_name.contains_key(name) {
            return Err(QueryError::DatasetNotFound(name.clone()));
        }
    }

    let resolve_field = |path: &str| -> Result<ResolvedField, QueryError> {
        let (ds_name, field_name) = parse_field_ref(path)?;
        if !required.contains(ds_name) {
            return Err(QueryError::DatasetNotFound(ds_name.to_string()));
        }
        let dataset = dataset_by_name
            .get(ds_name)
            .cloned()
            .ok_or_else(|| QueryError::DatasetNotFound(ds_name.to_string()))?;
        let field = dataset.field(field_name).cloned().ok_or_else(|| {
            match model.issue_for(&IssueTarget::field(ds_name, field_name)) {
                Some(issue) => QueryError::NotCompiled {
                    target: issue.target.to_string(),
                    reason: issue.message.clone(),
                },
                None => QueryError::FieldNotFound(format!("{ds_name}.{field_name}")),
            }
        })?;
        Ok(ResolvedField {
            dataset: ds_name.to_string(),
            field,
        })
    };

    let group_by: Vec<ResolvedField> = spec
        .group_by
        .iter()
        .map(|p| resolve_field(p))
        .collect::<Result<_, _>>()?;

    let mut filters = Vec::new();
    for f in &spec.filters {
        let field = resolve_field(&f.field)?;
        let operator = f.operator();
        let rhs_field = if operator == FilterOperator::EqField {
            let path = f.value.as_str().ok_or_else(|| QueryError::InvalidFilter {
                field: f.field.clone(),
                reason: "eq_field requires a string value naming `dataset.field`".to_string(),
            })?;
            Some(resolve_field(path)?)
        } else {
            None
        };
        filters.push(BoundFilter {
            field,
            operator,
            value: f.value.clone(),
            rhs_field,
        });
    }

    let joins = if required.len() <= 1 {
        Vec::new()
    } else {
        build_join_order(model, &required, &root)?
    };

    let mut sources: HashMap<String, String> = HashMap::new();
    for name in &required {
        let ds = dataset_by_name.get(name).unwrap();
        sources.insert((*name).clone(), ds.source.clone());
    }

    Ok(BoundQuery {
        model_name: model.name.clone(),
        root_dataset: root,
        sources,
        joins,
        group_by,
        metrics,
        filters,
        order_by: spec.order_by.clone(),
        limit: spec.limit,
        offset: spec.offset,
    })
}

fn parse_field_ref(s: &str) -> Result<(&str, &str), QueryError> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(QueryError::InvalidFieldRef(s.to_string()));
    }
    Ok((parts[0], parts[1]))
}

fn join_keys(left: &str, right: &str, rel: &Relationship) -> Option<Vec<JoinKey>> {
    if left == rel.from && right == rel.to {
        if rel.from_columns.len() != rel.to_columns.len() {
            return None;
        }
        return Some(
            rel.from_columns
                .iter()
                .zip(rel.to_columns.iter())
                .map(|(a, b)| JoinKey {
                    left_dataset: left.to_string(),
                    left_column: a.clone(),
                    right_dataset: right.to_string(),
                    right_column: b.clone(),
                })
                .collect(),
        );
    }
    if left == rel.to && right == rel.from {
        if rel.from_columns.len() != rel.to_columns.len() {
            return None;
        }
        return Some(
            rel.from_columns
                .iter()
                .zip(rel.to_columns.iter())
                .map(|(a, b)| JoinKey {
                    left_dataset: left.to_string(),
                    left_column: b.clone(),
                    right_dataset: right.to_string(),
                    right_column: a.clone(),
                })
                .collect(),
        );
    }
    None
}

/// Prim-style: grow from `root` until `required` is covered.
fn build_join_order(
    model: &CompiledModel,
    required: &HashSet<String>,
    root: &str,
) -> Result<Vec<BoundJoin>, QueryError> {
    let mut included: HashSet<String> = std::iter::once(root.to_string()).collect();
    let mut joins = Vec::new();

    while included != *required {
        let mut step: Option<(String, String, BoundJoin)> = None;
        for rel in &model.relationships {
            let a = rel.from.as_str();
            let b = rel.to.as_str();
            if included.contains(a) && required.contains(b) && !included.contains(b) {
                if let Some(on) = join_keys(a, b, rel) {
                    step = Some((
                        a.to_string(),
                        b.to_string(),
                        BoundJoin {
                            left_dataset: a.to_string(),
                            right_dataset: b.to_string(),
                            on,
                        },
                    ));
                    break;
                }
            }
            if included.contains(b) && required.contains(a) && !included.contains(a) {
                if let Some(on) = join_keys(b, a, rel) {
                    step = Some((
                        b.to_string(),
                        a.to_string(),
                        BoundJoin {
                            left_dataset: b.to_string(),
                            right_dataset: a.to_string(),
                            on,
                        },
                    ));
                    break;
                }
            }
        }
        let (left, right, join) = step.ok_or_else(|| {
            QueryError::DisconnectedDatasets(
                required.difference(&included).cloned().collect::<Vec<_>>(),
            )
        })?;
        included.insert(right.clone());
        joins.push(join);
        let _ = left;
    }

    Ok(joins)
}
