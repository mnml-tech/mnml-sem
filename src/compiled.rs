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

use std::collections::BTreeSet;

use crate::error::Error;
use crate::model::{DataType, Dataset, Field, Metric, OssieDocument, Relationship, SemanticModel};
use crate::plan::ossie_sql;
use crate::plan::{Expr, pick_expression_dialect};

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

/// A dataset with its fields compiled.
#[derive(Debug, Clone)]
pub struct CompiledDataset {
    pub name: String,
    pub source: String,
    pub fields: Vec<CompiledField>,
}

impl CompiledDataset {
    pub fn field(&self, name: &str) -> Option<&CompiledField> {
        self.fields.iter().find(|f| f.name == name)
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
    pub datatype: Option<DataType>,
    pub expr: Expr,
    /// Datasets this metric's expression reads. Ossie metrics may span
    /// datasets, so this is what decides the joins a query needs.
    pub datasets: BTreeSet<String>,
}

/// What a [`CompileIssue`] is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTarget {
    Field { dataset: String, field: String },
    Metric { name: String },
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
}

impl std::fmt::Display for IssueTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueTarget::Field { dataset, field } => write!(f, "field {dataset:?}.{field:?}"),
            IssueTarget::Metric { name } => write!(f, "metric {name:?}"),
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
            .map(|ds| compile_dataset(ds, &mut issues))
            .collect();

        // Metrics resolve `dataset.field` through the model, so they need the
        // raw model rather than the partially compiled one.
        let metrics: Vec<CompiledMetric> = model
            .metrics
            .iter()
            .filter_map(|m| compile_metric(m, model, &mut issues))
            .collect();

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

    pub fn metric(&self, name: &str) -> Option<&CompiledMetric> {
        self.metrics.iter().find(|m| m.name == name)
    }
}

fn compile_dataset(ds: &Dataset, issues: &mut Vec<CompileIssue>) -> CompiledDataset {
    let fields = ds
        .fields
        .iter()
        .filter_map(|f| compile_field(f, &ds.name, issues))
        .collect();
    CompiledDataset {
        name: ds.name.clone(),
        source: ds.source.clone(),
        fields,
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

fn compile_metric(
    metric: &Metric,
    model: &SemanticModel,
    issues: &mut Vec<CompileIssue>,
) -> Option<CompiledMetric> {
    let target = IssueTarget::metric(&metric.name);
    let (dialect, sql) = match pick_expression_dialect(&metric.expression) {
        Some(picked) => picked,
        None => {
            issues.push(CompileIssue {
                target,
                message: "no usable dialect entry".to_string(),
            });
            return None;
        }
    };
    let expr = match ossie_sql::lower_metric_expression(&sql, dialect, model) {
        Ok(expr) => expr,
        Err(e) => {
            issues.push(CompileIssue {
                target,
                message: e.to_string(),
            });
            return None;
        }
    };
    Some(CompiledMetric {
        name: metric.name.clone(),
        datatype: metric.datatype,
        datasets: expr.referenced_datasets(),
        expr,
    })
}
