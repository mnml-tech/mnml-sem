//! Compile a semantic model once, then bind many queries against it.
//!
//! Expressions in an Ossie document are SQL text. Parsing them per query means
//! re-parsing every field of every scanned dataset, plus each metric, on every
//! request — and it means a typo in a metric surfaces only when someone happens
//! to query it.
//!
//! [`CompiledModel::compile`] does that work once: it lowers every field and
//! metric to [`Expr`], records which physical column each field maps to and
//! which datasets each metric touches. Binding and planning then read resolved
//! data.
//!
//! Compilation is **partial by design**: a field or metric this crate cannot
//! compile is recorded in [`CompiledModel::issues`] and left out of the usable
//! set, rather than failing the whole model. A real model may well contain one
//! expression we do not support yet (a window function, say) while every other
//! metric is perfectly queryable. Referencing a failed item is an error that
//! quotes the original reason; [`CompiledModel::compile_strict`] refuses any
//! model with issues, for callers that want that.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use crate::error::Error;
use crate::model::{DataType, Dataset, Field, Metric, OssieDocument, Relationship, SemanticModel};
use crate::plan::ossie_sql::{self, MetricLookup};
use crate::plan::{Expr, pick_expression_dialect};
use crate::resolver::QueryError;

/// A field with its expression already lowered.
#[derive(Debug, Clone)]
pub struct CompiledField {
    pub name: String,
    pub datatype: Option<DataType>,
    /// Effective temporal role, with the spec's `datatype` default applied.
    pub is_time: bool,
    pub expr: Expr,
    /// The physical column this field reads, when it is a bare column
    /// reference. `None` for computed fields, which a scan cannot read directly.
    pub physical_column: Option<String>,
}

/// A dataset with its fields and dataset-scoped metrics compiled.
#[derive(Debug, Clone)]
pub struct CompiledDataset {
    pub name: String,
    pub source: String,
    pub fields: Vec<CompiledField>,
    pub metrics: Vec<CompiledMetric>,
}

impl CompiledDataset {
    pub fn field(&self, name: &str) -> Option<&CompiledField> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn metric(&self, name: &str) -> Option<&CompiledMetric> {
        self.metrics.iter().find(|m| m.name == name)
    }

    /// Physical columns a scan of this dataset exposes, in declaration order.
    ///
    /// Deduplicated case-insensitively: two fields spelling the same
    /// underlying column differently name one column, not two. The first
    /// spelling is the one exposed.
    pub fn scan_columns(&self) -> Vec<(&str, Option<DataType>)> {
        let mut out: Vec<(&str, Option<DataType>)> = Vec::new();
        for f in &self.fields {
            let Some(col) = f.physical_column.as_deref() else {
                continue;
            };
            if out.iter().any(|(name, _)| name.eq_ignore_ascii_case(col)) {
                continue;
            }
            out.push((col, f.datatype));
        }
        out
    }
}

/// A metric with its expression already lowered.
#[derive(Debug, Clone)]
pub struct CompiledMetric {
    pub name: String,
    /// The declaring dataset of a dataset-scoped metric; `None` for a
    /// model-scoped one.
    pub dataset: Option<String>,
    pub datatype: Option<DataType>,
    pub expr: Expr,
    /// Datasets this metric's expression reads. Ossie metrics may span
    /// datasets, so this is what decides the joins a query needs.
    pub datasets: BTreeSet<String>,
}

impl CompiledMetric {
    /// How a query names this metric, which is also its output column:
    /// `dataset.metric` when dataset-scoped, the bare name otherwise.
    pub fn reference(&self) -> String {
        match &self.dataset {
            Some(ds) => format!("{ds}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// What a [`CompileIssue`] is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTarget {
    Field { dataset: String, field: String },
    Metric { name: String },
    DatasetMetric { dataset: String, metric: String },
}

impl IssueTarget {
    pub fn field(dataset: impl Into<String>, field: impl Into<String>) -> Self {
        IssueTarget::Field {
            dataset: dataset.into(),
            field: field.into(),
        }
    }

    pub fn metric(name: impl Into<String>) -> Self {
        IssueTarget::Metric { name: name.into() }
    }

    pub fn dataset_metric(dataset: impl Into<String>, metric: impl Into<String>) -> Self {
        IssueTarget::DatasetMetric {
            dataset: dataset.into(),
            metric: metric.into(),
        }
    }
}

impl std::fmt::Display for IssueTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueTarget::Field { dataset, field } => write!(f, "field {dataset:?}.{field:?}"),
            IssueTarget::Metric { name } => write!(f, "metric {name:?}"),
            IssueTarget::DatasetMetric { dataset, metric } => {
                write!(f, "metric {dataset:?}.{metric:?}")
            }
        }
    }
}

/// An expression that could not be compiled.
#[derive(Debug, Clone)]
pub struct CompileIssue {
    pub target: IssueTarget,
    pub message: String,
}

impl std::fmt::Display for CompileIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Expression] {}: {}", self.target, self.message)
    }
}

/// A semantic model with every expression compiled.
#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub name: String,
    pub datasets: Vec<CompiledDataset>,
    pub metrics: Vec<CompiledMetric>,
    pub relationships: Vec<Relationship>,
    /// Expressions that failed to compile. The named fields and metrics are
    /// absent from the lists above and cannot be queried.
    pub issues: Vec<CompileIssue>,
}

impl CompiledModel {
    /// Load a document: validate its structure, then compile its expressions.
    ///
    /// This is the entry point — it performs every check the crate makes.
    /// Structural problems (wrong spec version, duplicate names, relationships
    /// naming datasets that do not exist) are fatal, because they leave the
    /// model ambiguous. A single expression that will not compile is not: it is
    /// recorded in [`Self::issues`] and that one field or metric becomes
    /// unusable while the rest of the model still works.
    pub fn compile(doc: &OssieDocument) -> Result<Self, Error> {
        crate::validate::validate(doc)?;
        Self::compile_model(&doc.model)
    }

    /// Compile, and refuse the model if any expression failed.
    pub fn compile_strict(doc: &OssieDocument) -> Result<Self, Error> {
        let compiled = Self::compile(doc)?;
        if !compiled.issues.is_empty() {
            return Err(Error::Validation(
                compiled.issues.iter().map(ToString::to_string).collect(),
            ));
        }
        Ok(compiled)
    }

    /// Compile a model's expressions without the document-level structural
    /// checks. Prefer [`Self::compile`], which does both.
    pub fn compile_model(model: &SemanticModel) -> Result<Self, Error> {
        let mut issues = Vec::new();

        let datasets: Vec<CompiledDataset> = model
            .datasets
            .iter()
            .map(|ds| compile_dataset(ds, model, &mut issues))
            .collect();

        // A metric may reference another, so they resolve through a shared
        // resolver that compiles each once and catches cycles, rather than
        // being compiled independently.
        let resolver = MetricResolver::new(model, &datasets);
        let mut metrics: Vec<CompiledMetric> = Vec::new();
        for m in &model.metrics {
            match resolver.resolve(&m.name) {
                Ok(expr) => metrics.push(CompiledMetric {
                    name: m.name.clone(),
                    dataset: None,
                    datatype: m.datatype,
                    datasets: expr.referenced_datasets(),
                    expr,
                }),
                Err(e) => issues.push(CompileIssue {
                    target: IssueTarget::metric(&m.name),
                    message: e.to_string(),
                }),
            }
        }

        Ok(Self {
            name: model.name.clone(),
            datasets,
            metrics,
            relationships: model.relationships.clone(),
            issues,
        })
    }

    /// Why `target` failed to compile, if it did.
    pub fn issue_for(&self, target: &IssueTarget) -> Option<&CompileIssue> {
        self.issues.iter().find(|i| i.target == *target)
    }

    pub fn dataset(&self, name: &str) -> Option<&CompiledDataset> {
        self.datasets.iter().find(|d| d.name == name)
    }

    /// A model-scoped metric by name.
    pub fn metric(&self, name: &str) -> Option<&CompiledMetric> {
        self.metrics.iter().find(|m| m.name == name)
    }

    /// A dataset-scoped metric of `dataset`.
    pub fn dataset_metric(&self, dataset: &str, name: &str) -> Option<&CompiledMetric> {
        self.dataset(dataset)?.metric(name)
    }
}

/// Compiles metrics on demand, so a metric that references another is
/// resolved by substituting what that one compiled to.
///
/// Substitution is closed under the expression IR — aggregates stay at the
/// leaves — so nothing downstream needs to know a reference was involved.
struct MetricResolver<'a> {
    model: &'a SemanticModel,
    /// Already compiled, so a dataset-scoped metric is lowered once however
    /// many model-scoped metrics build on it.
    datasets: &'a [CompiledDataset],
    compiled: RefCell<HashMap<String, Expr>>,
    /// Names currently being resolved, innermost last. A name already here is
    /// a cycle, and the stack is the path to report.
    in_progress: RefCell<Vec<String>>,
}

impl<'a> MetricResolver<'a> {
    fn new(model: &'a SemanticModel, datasets: &'a [CompiledDataset]) -> Self {
        Self {
            model,
            datasets,
            compiled: RefCell::new(HashMap::new()),
            in_progress: RefCell::new(Vec::new()),
        }
    }

    fn resolve(&self, name: &str) -> Result<Expr, QueryError> {
        if let Some(done) = self.compiled.borrow().get(name) {
            return Ok(done.clone());
        }
        if let Some(at) = self.in_progress.borrow().iter().position(|n| n == name) {
            let mut path = self.in_progress.borrow()[at..].join(" -> ");
            path.push_str(" -> ");
            path.push_str(name);
            return Err(QueryError::MetricCycle { path });
        }

        let metric = self
            .model
            .metrics
            .iter()
            .find(|m| m.name == name)
            .ok_or_else(|| QueryError::UnknownMetricReference(name.to_string()))?;
        let (dialect, sql) =
            pick_expression_dialect(&metric.expression).ok_or(QueryError::MissingExpressionSql)?;

        self.in_progress.borrow_mut().push(name.to_string());
        let lowered = ossie_sql::lower_metric_expression(&sql, dialect, self.model, self);
        self.in_progress.borrow_mut().pop();

        let expr = lowered?;
        self.compiled
            .borrow_mut()
            .insert(name.to_string(), expr.clone());
        Ok(expr)
    }
}

impl MetricLookup for MetricResolver<'_> {
    fn lookup(&self, name: &str) -> Result<Expr, QueryError> {
        self.resolve(name).map_err(|e| match e {
            // A cycle is about the chain, not about this one reference.
            cycle @ QueryError::MetricCycle { .. } => cycle,
            // Otherwise say whose failure this is, rather than repeating
            // another metric's error as if it were this metric's own.
            other => QueryError::MetricReferenceFailed {
                metric: name.to_string(),
                reason: other.to_string(),
            },
        })
    }

    /// A dataset-scoped metric references no metric, so it cannot be part of
    /// a cycle and is compiled already.
    fn lookup_dataset_metric(&self, dataset: &str, name: &str) -> Result<Expr, QueryError> {
        let compiled = self
            .datasets
            .iter()
            .find(|d| d.name == dataset)
            .and_then(|d| d.metric(name));
        if let Some(m) = compiled {
            return Ok(m.expr.clone());
        }
        // Declared but absent: it failed to compile. Lowering it again is the
        // simplest way to say why, and this path is only taken on failure.
        let reason = self
            .model
            .datasets
            .iter()
            .find(|d| d.name == dataset)
            .and_then(|d| d.metrics.iter().find(|m| m.name == name))
            .map(|m| match pick_expression_dialect(&m.expression) {
                None => QueryError::MissingExpressionSql.to_string(),
                Some((dialect, sql)) => {
                    ossie_sql::lower_dataset_metric_expression(&sql, dialect, self.model, dataset)
                        .err()
                        .map_or_else(|| "it did not compile".to_string(), |e| e.to_string())
                }
            })
            .unwrap_or_else(|| "it is not declared".to_string());
        Err(QueryError::MetricReferenceFailed {
            metric: format!("{dataset}.{name}"),
            reason,
        })
    }
}

fn compile_dataset(
    ds: &Dataset,
    model: &SemanticModel,
    issues: &mut Vec<CompileIssue>,
) -> CompiledDataset {
    let fields = ds
        .fields
        .iter()
        .filter_map(|f| compile_field(f, &ds.name, issues))
        .collect();
    let metrics = ds
        .metrics
        .iter()
        .filter_map(|m| compile_dataset_metric(m, &ds.name, model, issues))
        .collect();
    CompiledDataset {
        name: ds.name.clone(),
        source: ds.source.clone(),
        fields,
        metrics,
    }
}

/// A dataset-scoped metric reads only its own dataset's fields, so it needs
/// no resolver: there is no metric for it to reference.
fn compile_dataset_metric(
    metric: &Metric,
    dataset: &str,
    model: &SemanticModel,
    issues: &mut Vec<CompileIssue>,
) -> Option<CompiledMetric> {
    let target = IssueTarget::dataset_metric(dataset, &metric.name);
    let lowered = pick_expression_dialect(&metric.expression)
        .ok_or(QueryError::MissingExpressionSql)
        .and_then(|(dialect, sql)| {
            ossie_sql::lower_dataset_metric_expression(&sql, dialect, model, dataset)
        });
    match lowered {
        Ok(expr) => Some(CompiledMetric {
            name: metric.name.clone(),
            dataset: Some(dataset.to_string()),
            datatype: metric.datatype,
            datasets: expr.referenced_datasets(),
            expr,
        }),
        Err(e) => {
            issues.push(CompileIssue {
                target,
                message: e.to_string(),
            });
            None
        }
    }
}

fn compile_field(
    field: &Field,
    dataset: &str,
    issues: &mut Vec<CompileIssue>,
) -> Option<CompiledField> {
    let target = IssueTarget::field(dataset, &field.name);
    let (dialect, sql) = match pick_expression_dialect(&field.expression) {
        Some(picked) => picked,
        None => {
            issues.push(CompileIssue {
                target,
                message: "no usable dialect entry".to_string(),
            });
            return None;
        }
    };
    let expr = match ossie_sql::lower_field_expression(&sql, dialect, dataset) {
        Ok(expr) => expr,
        Err(e) => {
            issues.push(CompileIssue {
                target,
                message: e.to_string(),
            });
            return None;
        }
    };
    let physical_column = match &expr {
        Expr::Column(c) => Some(c.column.clone()),
        _ => None,
    };
    Some(CompiledField {
        name: field.name.clone(),
        datatype: field.datatype,
        is_time: field.is_time_dimension(),
        expr,
        physical_column,
    })
}
