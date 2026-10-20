//! Semantic validation beyond Serde (uniqueness, references, schema constraints),
//! for semantic models here, and for ontologies and mappings in
//! [`validate_ontology`] and [`validate_mapping`].

mod ontology;

pub use ontology::{validate_mapping, validate_ontology};

use crate::error::Error;
use crate::{Expression, OSSIE_SPEC_VERSION, OssieDocument};
use std::collections::{HashMap, HashSet};

/// Structural checks on a document: spec version, name uniqueness, and
/// relationship references.
///
/// This is the **subset** that needs no expression compilation. It cannot tell
/// you whether an expression is valid, because that needs the model as context.
/// `ossie_compiler::compiled::CompiledModel::compile` runs
/// these checks and then compiles every expression; prefer it for loading a
/// model.
pub fn validate(doc: &OssieDocument) -> Result<(), Error> {
    let mut issues = Vec::new();

    if doc.version != OSSIE_SPEC_VERSION {
        issues.push(format!(
            "[Version] expected {:?}, got {:?}",
            OSSIE_SPEC_VERSION, doc.version
        ));
    }

    validate_semantic_model(&doc.model, &mut issues);

    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(issues))
    }
}

fn validate_semantic_model(model: &crate::SemanticModel, issues: &mut Vec<String>) {
    let model_name = &model.name;

    if model.datasets.is_empty() {
        issues.push(format!(
            "[SemanticModel] model {:?} must have at least one dataset",
            model_name
        ));
    }

    let dataset_name_list: Vec<&str> = model.datasets.iter().map(|d| d.name.as_str()).collect();
    for dup in duplicate_strings(&dataset_name_list) {
        issues.push(format!(
            "[Unique] duplicate dataset name {:?} in model {:?}",
            dup, model_name
        ));
    }

    for dataset in &model.datasets {
        let field_names: Vec<&str> = dataset.fields.iter().map(|f| f.name.as_str()).collect();
        for dup in duplicate_strings(&field_names) {
            issues.push(format!(
                "[Unique] duplicate field name {:?} in dataset {:?} (model {:?})",
                dup, dataset.name, model_name
            ));
        }

        for field in &dataset.fields {
            validate_expression(
                &format!("field {:?}.{:?}", dataset.name, field.name),
                &field.expression,
                issues,
            );
        }

        let ds_metric_names: Vec<&str> = dataset.metrics.iter().map(|m| m.name.as_str()).collect();
        for dup in duplicate_strings(&ds_metric_names) {
            issues.push(format!(
                "[Unique] duplicate metric name {:?} in dataset {:?} (model {:?})",
                dup, dataset.name, model_name
            ));
        }
        // Fields and dataset-scoped metrics share one namespace, so
        // `dataset.name` means exactly one thing.
        for name in &ds_metric_names {
            if field_names.contains(name) {
                issues.push(format!(
                    "[Unique] {:?} in dataset {:?} names both a field and a metric; they share \
                     one namespace (model {:?})",
                    name, dataset.name, model_name
                ));
            }
        }

        for metric in &dataset.metrics {
            validate_expression(
                &format!("metric {:?}.{:?}", dataset.name, metric.name),
                &metric.expression,
                issues,
            );
        }
    }

    let metric_names: Vec<&str> = model.metrics.iter().map(|m| m.name.as_str()).collect();
    for dup in duplicate_strings(&metric_names) {
        issues.push(format!(
            "[Unique] duplicate metric name {:?} in model {:?}",
            dup, model_name
        ));
    }

    // The schema puts no pattern on names, so a model-scoped metric may be
    // called `orders.total` — which a query could not tell apart from the
    // dataset-scoped `total` on `orders`.
    for metric in &model.metrics {
        let Some((ds, name)) = metric.name.split_once('.') else {
            continue;
        };
        let clashes = model
            .datasets
            .iter()
            .any(|d| d.name == ds && d.metrics.iter().any(|m| m.name == name));
        if clashes {
            issues.push(format!(
                "[Unique] model metric {:?} is spelled the same as the metric {:?} of dataset \
                 {:?} (model {:?})",
                metric.name, name, ds, model_name
            ));
        }
    }

    for metric in &model.metrics {
        validate_expression(
            &format!("metric {:?}", metric.name),
            &metric.expression,
            issues,
        );
    }

    let rel_names: Vec<&str> = model
        .relationships
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    for dup in duplicate_strings(&rel_names) {
        issues.push(format!(
            "[Unique] duplicate relationship name {:?} in model {:?}",
            dup, model_name
        ));
    }

    let known: HashSet<&str> = dataset_name_list.iter().copied().collect();

    for rel in &model.relationships {
        if rel.from_columns.len() != rel.to_columns.len() {
            issues.push(format!(
                "[Relationship] {:?}: from_columns length {} does not match to_columns length {}",
                rel.name,
                rel.from_columns.len(),
                rel.to_columns.len()
            ));
        }
        if !known.contains(rel.from.as_str()) {
            issues.push(format!(
                "[Reference] relationship {:?} references unknown dataset {:?} (model {:?})",
                rel.name, rel.from, model_name
            ));
        }
        if !known.contains(rel.to.as_str()) {
            issues.push(format!(
                "[Reference] relationship {:?} references unknown dataset {:?} (model {:?})",
                rel.name, rel.to, model_name
            ));
        }
    }
}

/// Every expression needs at least one dialect entry (schema `minItems: 1`).
///
/// Whether the text *compiles* is not checked here: that needs the query
/// context (which dataset an identifier belongs to) and is reported by the
/// planner, where the error can name the query that hit it.
fn validate_expression(context: &str, expr: &Expression, issues: &mut Vec<String>) {
    if expr.dialects.is_empty() {
        issues.push(format!(
            "[Expression] {context} must have at least one dialect entry"
        ));
    }
}

/// Names that appear more than once (stable order of first occurrence of each duplicated value).
fn duplicate_strings(names: &[&str]) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for n in names {
        *counts.entry(*n).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .filter(|(_, c)| *c > 1)
        .map(|(k, _)| k.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::parser::from_yaml_str;

    const VALID: &str = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: people
    source: warehouse.public.people
    fields:
      - name: id
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: id
"#;

    #[test]
    fn validate_accepts_a_minimal_document() {
        let doc = from_yaml_str(VALID).expect("parse");
        validate(&doc).expect("validate");
    }

    /// Vendor extensions are carried, never interpreted.
    #[test]
    fn validate_ignores_vendor_extension_payloads() {
        let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: people
    source: warehouse.public.people
    custom_extensions:
      - vendor_name: ACME
        data: 'not json at all'
    fields:
      - name: id
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: id
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        validate(&doc).expect("an opaque vendor payload must not fail validation");
    }

    #[test]
    fn validate_rejects_a_wrong_version() {
        let doc = from_yaml_str(&VALID.replace("0.2.0.dev0", "0.1.1")).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(err.to_string().contains("[Version]"), "{err}");
    }

    #[test]
    fn validate_rejects_duplicate_dataset_names() {
        let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: d
    source: a
  - name: d
    source: b
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(err.to_string().contains("duplicate dataset name"), "{err}");
    }

    #[test]
    fn validate_rejects_unknown_relationship_datasets() {
        let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: d
    source: a
relationships:
  - name: r
    from: d
    to: nope
    from_columns: [x]
    to_columns: [y]
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(err.to_string().contains("unknown dataset"), "{err}");
    }

    #[test]
    fn validate_rejects_mismatched_relationship_columns() {
        let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: a
    source: a
  - name: b
    source: b
relationships:
  - name: r
    from: a
    to: b
    from_columns: [x, y]
    to_columns: [z]
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    /// `people` with a field `id` and the given dataset-scoped metrics.
    fn with_dataset_metrics(metrics: &[&str]) -> String {
        let mut yaml = VALID.to_string();
        yaml.push_str("    metrics:\n");
        for name in metrics {
            yaml.push_str(&format!(
                "      - name: {name}\n        expression:\n          dialects:\n            \
                 - dialect: ANSI_SQL\n              expression: COUNT(id)\n"
            ));
        }
        yaml
    }

    #[test]
    fn validate_accepts_dataset_metrics() {
        let doc = from_yaml_str(&with_dataset_metrics(&["n", "m"])).expect("parse");
        validate(&doc).expect("validate");
    }

    #[test]
    fn validate_rejects_duplicate_dataset_metric_names() {
        let doc = from_yaml_str(&with_dataset_metrics(&["n", "n"])).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(
            err.to_string()
                .contains(r#"duplicate metric name "n" in dataset "people""#),
            "{err}"
        );
    }

    /// Fields and dataset-scoped metrics share one namespace.
    #[test]
    fn validate_rejects_a_metric_named_like_a_field() {
        let doc = from_yaml_str(&with_dataset_metrics(&["id"])).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(
            err.to_string().contains("names both a field and a metric"),
            "{err}"
        );
    }

    #[test]
    fn validate_rejects_a_model_metric_spelled_like_a_dataset_metric() {
        let mut yaml = with_dataset_metrics(&["n"]);
        yaml.push_str(
            "metrics:\n  - name: people.n\n    expression:\n      dialects:\n        \
             - dialect: ANSI_SQL\n          expression: COUNT(people.id)\n",
        );
        let doc = from_yaml_str(&yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        assert!(err.to_string().contains("is spelled the same as"), "{err}");
    }
}
