//! Semantic validation beyond Serde (uniqueness, references, schema constraints).

use crate::error::Error;
use crate::model::{
    count_mnml_dialects, pick_mnml_dialect, Expression, MnmlDatasetStorage, MnmlExpressionKind,
    OsiDocument, envelope_from_expression,
};
use std::collections::{HashMap, HashSet};

const EXPECTED_VERSION: &str = "0.1.1";

/// Validate an [`OsiDocument`] (version, uniqueness, references, expression shape).
pub fn validate(doc: &OsiDocument) -> Result<(), Error> {
    let mut issues = Vec::new();

    if doc.version != EXPECTED_VERSION {
        issues.push(format!(
            "[Version] expected {:?}, got {:?}",
            EXPECTED_VERSION, doc.version
        ));
    }

    for model in &doc.semantic_model {
        validate_semantic_model(model, &mut issues);
    }

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
            validate_expression_or_mnml(
                &format!("field {:?}.{:?}", dataset.name, field.name),
                &field.expression,
                issues,
                MnmlExpressionKind::FieldExpr,
            );
        }

        let mut mnml_storage: Vec<MnmlDatasetStorage> = Vec::new();
        for ext in &dataset.custom_extensions {
            if ext.vendor_name != crate::model::Vendor::Mnml {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&ext.data) else {
                issues.push(format!(
                    "[MNML] dataset {:?} (model {:?}) has invalid MNML extension JSON",
                    dataset.name, model_name
                ));
                continue;
            };
            if v.get("format").is_some() {
                match serde_json::from_value::<MnmlDatasetStorage>(v) {
                    Ok(storage) => mnml_storage.push(storage),
                    Err(e) => issues.push(format!(
                        "[MNML] dataset {:?} (model {:?}) invalid storage payload: {e}",
                        dataset.name, model_name
                    )),
                }
            }
        }
        validate_mnml_dataset_storage(model_name, &dataset.name, &mnml_storage, issues);
    }

    let metric_names: Vec<&str> = model.metrics.iter().map(|m| m.name.as_str()).collect();
    for dup in duplicate_strings(&metric_names) {
        issues.push(format!(
            "[Unique] duplicate metric name {:?} in model {:?}",
            dup, model_name
        ));
    }

    for metric in &model.metrics {
        validate_expression_or_mnml(
            &format!("metric {:?}", metric.name),
            &metric.expression,
            issues,
            MnmlExpressionKind::Metric,
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

fn validate_expression_nonempty(context: &str, expr: &Expression, issues: &mut Vec<String>) {
    if expr.dialects.is_empty() {
        issues.push(format!(
            "[Expression] {} must have at least one dialect entry",
            context
        ));
    }
}

/// Require a valid MNML dialect entry **or** at least one OSI dialect entry.
fn validate_expression_or_mnml(
    context: &str,
    expr: &Expression,
    issues: &mut Vec<String>,
    kind: MnmlExpressionKind,
) {
    let dialect_count = count_mnml_dialects(expr);
    if dialect_count > 1 {
        issues.push(format!(
            "[MNML] {context} has multiple MNML dialect entries; expected at most one"
        ));
    }

    if let Some(body) = pick_mnml_dialect(expr) {
        if body.as_object().is_none() {
            issues.push(format!(
                "[MNML] {context} MNML dialect body must be a mapping"
            ));
        }
        if let Err(e) = envelope_from_expression(expr, kind) {
            issues.push(format!("[MNML] {context} invalid MNML dialect: {e}"));
        }
        return;
    }

    validate_expression_nonempty(context, expr, issues);
}

fn validate_mnml_dataset_storage(
    model_name: &str,
    dataset_name: &str,
    mnml_exts: &[MnmlDatasetStorage],
    issues: &mut Vec<String>,
) {
    if mnml_exts.len() > 1 {
        issues.push(format!(
            "[MNML] dataset {:?} (model {:?}) has multiple MNML storage extensions; expected at most one",
            dataset_name, model_name
        ));
    }

    for ext in mnml_exts {
        if ext.version != 1 {
            issues.push(format!(
                "[MNML] dataset {:?} (model {:?}) has unsupported MNML storage version {}; expected 1",
                dataset_name, model_name, ext.version
            ));
        }

        let location = ext.location.trim();
        if location.is_empty() {
            issues.push(format!(
                "[MNML] dataset {:?} (model {:?}) has empty MNML storage location",
                dataset_name, model_name
            ));
        } else {
            if location.starts_with('/') {
                issues.push(format!(
                    "[MNML] dataset {:?} (model {:?}) location {:?} must be relative (no leading '/')",
                    dataset_name, model_name, ext.location
                ));
            }
            if location.contains("://") {
                issues.push(format!(
                    "[MNML] dataset {:?} (model {:?}) location {:?} must be relative (URI schemes are not allowed)",
                    dataset_name, model_name, ext.location
                ));
            }
            if location.split('/').any(|segment| segment == "..") {
                issues.push(format!(
                    "[MNML] dataset {:?} (model {:?}) location {:?} must not contain '..'",
                    dataset_name, model_name, ext.location
                ));
            }
        }

        if let Some(partitioning) = &ext.partitioning {
            if partitioning.keys.is_empty() {
                issues.push(format!(
                    "[MNML] dataset {:?} (model {:?}) partitioning.keys must not be empty",
                    dataset_name, model_name
                ));
            }

            let key_names: Vec<&str> = partitioning.keys.iter().map(String::as_str).collect();
            for dup in duplicate_strings(&key_names) {
                issues.push(format!(
                    "[MNML] dataset {:?} (model {:?}) partitioning.keys has duplicate key {:?}",
                    dataset_name, model_name, dup
                ));
            }
            for key in &partitioning.keys {
                if key.trim().is_empty() {
                    issues.push(format!(
                        "[MNML] dataset {:?} (model {:?}) partitioning.keys must not contain empty values",
                        dataset_name, model_name
                    ));
                    break;
                }
            }
        }
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

    #[test]
    fn validate_allows_valid_mnml_storage_extension() {
        let yaml = r#"
version: "0.1.1"
semantic_model:
  - name: m
    datasets:
      - name: people
        source: warehouse.public.people
        custom_extensions:
          - vendor_name: MNML
            data: >-
              {"version":1,"format":"parquet","location":"people_daily/","partitioning":{"style":"hive","keys":["dt"]}}
        fields:
          - name: id
            expression:
              dialects:
                - dialect: ANSI_SQL
                  expression: id
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        validate(&doc).expect("validate");
    }

    #[test]
    fn validate_rejects_duplicate_mnml_storage_extensions() {
        let yaml = r#"
version: "0.1.1"
semantic_model:
  - name: m
    datasets:
      - name: people
        source: warehouse.public.people
        custom_extensions:
          - vendor_name: MNML
            data: '{"version":1,"format":"parquet","location":"a/"}'
          - vendor_name: MNML
            data: '{"version":1,"format":"parquet","location":"b/"}'
        fields:
          - name: id
            expression:
              dialects:
                - dialect: ANSI_SQL
                  expression: id
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        let msg = err.to_string();
        assert!(msg.contains("multiple MNML storage extensions"), "{msg}");
    }

    #[test]
    fn validate_rejects_absolute_or_parent_location() {
        let yaml = r#"
version: "0.1.1"
semantic_model:
  - name: m
    datasets:
      - name: people
        source: warehouse.public.people
        custom_extensions:
          - vendor_name: MNML
            data: '{"version":1,"format":"parquet","location":"file:///tmp/people/../x"}'
        fields:
          - name: id
            expression:
              dialects:
                - dialect: ANSI_SQL
                  expression: id
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        let msg = err.to_string();
        assert!(msg.contains("must be relative"), "{msg}");
        assert!(msg.contains("must not contain '..'"), "{msg}");
    }

    #[test]
    fn validate_rejects_invalid_mnml_payload_json() {
        let yaml = r#"
version: "0.1.1"
semantic_model:
  - name: m
    datasets:
      - name: people
        source: warehouse.public.people
        custom_extensions:
          - vendor_name: MNML
            data: '{"version":1,"format":"parquet",'
        fields:
          - name: id
            expression:
              dialects:
                - dialect: ANSI_SQL
                  expression: id
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let err = validate(&doc).expect_err("must fail");
        let msg = err.to_string();
        assert!(msg.contains("invalid MNML extension JSON"), "{msg}");
    }
}
