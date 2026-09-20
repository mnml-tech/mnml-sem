//! Semantic validation beyond Serde (uniqueness, references, schema constraints).

use crate::error::Error;
use crate::model::{Expression, OSSIE_SPEC_VERSION, OssieDocument};
use std::collections::{HashMap, HashSet};

/// Structural checks on a document: spec version, name uniqueness, and
/// relationship references.
///
/// This is the **subset** that needs no expression compilation. It cannot tell
/// you whether an expression is valid, because that needs the model as context.
/// [`CompiledModel::compile`](crate::compiled::CompiledModel::compile) runs
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

fn validate_semantic_model(model: &crate::model::SemanticModel, issues: &mut Vec<String>) {
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
    }

    let metric_names: Vec<&str> = model.metrics.iter().map(|m| m.name.as_str()).collect();
    for dup in duplicate_strings(&metric_names) {
        issues.push(format!(
            "[Unique] duplicate metric name {:?} in model {:?}",
            dup, model_name
        ));
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
}
