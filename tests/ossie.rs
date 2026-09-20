//! Integration tests: fixtures, upstream Ossie example, round-trip.

use ossie_substrait::model::{
    AiContext, DataType, DialectExpression, Expression, FieldDimension, OSSIE_SPEC_VERSION,
    OssieDocument, SemanticModel,
};
use ossie_substrait::compiled::CompiledModel;
use ossie_substrait::{parser, validate};
use std::path::PathBuf;

#[test]
fn steelwheels_fixture_parses_and_validates() {
    let yaml = include_str!("fixtures/steelwheels.yaml");
    let doc = parser::from_yaml_str(yaml).expect("parse steelwheels fixture");
    assert_eq!(doc.version, OSSIE_SPEC_VERSION);
    let m = &doc.model;
    assert_eq!(m.name, "steelwheels");
    assert_eq!(m.datasets.len(), 3);
    assert_eq!(m.relationships.len(), 2);
    assert_eq!(m.metrics.len(), 5);
    validate::validate(&doc).expect("validate steelwheels");
}

#[test]
fn minimal_fixture_parses_and_validates() {
    let yaml = include_str!("fixtures/minimal.yaml");
    let doc = parser::from_yaml_str(yaml).expect("parse minimal fixture");
    assert_eq!(doc.version, OSSIE_SPEC_VERSION);
    let m = &doc.model;
    assert_eq!(m.name, "minimal_model");
    assert_eq!(m.datasets.len(), 1);
    assert_eq!(m.datasets[0].name, "fact");
    validate::validate(&doc).expect("validate minimal");
}

#[test]
#[ignore = "clone github.com/apache/ossie as ../ossie (sibling of this repo), then: cargo test -- --include-ignored"]
fn tpcds_example_parses_and_validates() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ossie/examples/tpcds_semantic_model.yaml");
    assert!(
        path.exists(),
        "expected Ossie example at {} (clone github.com/apache/ossie next to this repo)",
        path.display()
    );
    let text = std::fs::read_to_string(&path).expect("read tpcds");
    let doc = parser::from_yaml_str(&text).expect("parse tpcds");
    assert_eq!(doc.version, OSSIE_SPEC_VERSION);
    let m = &doc.model;
    assert_eq!(m.name, "tpcds_retail_model");
    assert!(m.datasets.len() >= 5);
    assert_eq!(m.relationships.len(), 4);
    assert!(m.metrics.len() >= 3);
    validate::validate(&doc).expect("validate tpcds");
}

#[test]
fn ontology_documents_are_not_core_models() {
    // examples/flights.yaml is an ontology map (ontology/ontology.json), a
    // separate schema whose root requires `ontology`. The core parser must
    // reject it rather than silently accept a partial model.
    let yaml = format!(
        "version: \"{OSSIE_SPEC_VERSION}\"\nname: Flights\nrequires: []\nontology:\n  classes: []\n"
    );
    let err = parser::from_yaml_str(&yaml).expect_err("ontology map must not parse as a model");
    assert!(err.to_string().contains("unknown field"), "{err}");
}

#[test]
fn roundtrip_yaml_document() {
    let doc = OssieDocument {
        version: OSSIE_SPEC_VERSION.to_string(),
        model: SemanticModel {
            name: "rt".to_string(),
            description: None,
            ai_context: Some(AiContext::Text("hello".to_string())),
            datasets: vec![ossie_substrait::model::Dataset {
                name: "d".to_string(),
                source: "a.b.c".to_string(),
                primary_key: vec![],
                unique_keys: vec![],
                description: None,
                ai_context: None,
                fields: vec![ossie_substrait::model::Field {
                    name: "x".to_string(),
                    expression: Expression {
                        dialects: vec![DialectExpression::ansi_sql("x")],
                    },
                    dimension: Some(FieldDimension {
                        is_time: Some(false),
                    }),
                    label: None,
                    description: None,
                    datatype: Some(DataType::DateTimeTz),
                    ai_context: None,
                    custom_extensions: vec![],
                }],
                custom_extensions: vec![],
            }],
            relationships: vec![],
            metrics: vec![],
            custom_extensions: vec![],
        },
    };
    validate::validate(&doc).expect("valid");
    let yaml = serde_yaml::to_string(&doc).expect("serialize");

    // The model's properties must serialize at the document root.
    assert!(yaml.contains("\nname: rt\n"), "model name not at root:\n{yaml}");
    assert!(!yaml.contains("semantic_model"), "stale wrapper:\n{yaml}");

    let back: OssieDocument = parser::from_yaml_str(&yaml).expect("parse roundtrip");
    assert_eq!(back.version, doc.version);
    assert_eq!(back.model.name, "rt");
    let f = &back.model.datasets[0].fields[0];
    assert_eq!(f.datatype, Some(DataType::DateTimeTz));
    // Explicit is_time: false beats the temporal datatype default.
    assert!(!f.is_time_dimension());
}

#[test]
fn ai_context_string_on_dataset_from_json() {
    let v = serde_json::json!({
        "version": OSSIE_SPEC_VERSION,
        "name": "m",
        "datasets": [{
            "name": "d",
            "source": "s",
            "ai_context": "plain text",
            "fields": [{
                "name": "f",
                "expression": { "dialects": [{ "dialect": "ANSI_SQL", "expression": "1" }] }
            }]
        }]
    });
    let doc: OssieDocument = serde_json::from_value(v).expect("from_value");
    let ds = &doc.model.datasets[0];
    assert!(matches!(&ds.ai_context, Some(AiContext::Text(s)) if s == "plain text"));
}

#[test]
fn legacy_semantic_model_array_is_rejected() {
    let yaml = "version: \"0.1.1\"\nsemantic_model:\n  - name: m\n    datasets:\n      - name: d\n        source: s\n";
    let err = parser::from_yaml_str(yaml).expect_err("0.1.1 array shape must not parse");
    let msg = err.to_string();
    assert!(
        msg.contains("semantic_model") || msg.contains("unknown field"),
        "expected an unknown-field error naming the removed wrapper, got: {msg}"
    );
}

#[test]
fn unknown_root_key_is_rejected_with_a_located_error() {
    let yaml = format!(
        "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: s\nbogus: 1\n"
    );
    let err = parser::from_yaml_str(&yaml).expect_err("unknown root key must not parse");
    let msg = err.to_string();
    assert!(msg.contains("bogus"), "{msg}");
    assert!(msg.contains("line"), "error should carry a position: {msg}");
}

#[test]
fn removed_root_dialects_and_vendors_are_rejected() {
    for key in ["dialects:\n  - ANSI_SQL", "vendors:\n  - SNOWFLAKE"] {
        let yaml = format!(
            "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: s\n{key}\n"
        );
        let err = parser::from_yaml_str(&yaml).expect_err("removed root key must not parse");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}

#[test]
fn newly_added_dialects_parse() {
    for dialect in ["MAQL", "BIGQUERY", "SIGMA", "THOUGHTSPOT", "DAX"] {
        let yaml = format!(
            "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: s\n    fields:\n      - name: f\n        expression:\n          dialects:\n            - dialect: {dialect}\n              expression: f\n"
        );
        let doc = parser::from_yaml_str(&yaml).unwrap_or_else(|e| panic!("{dialect}: {e}"));
        validate::validate(&doc).unwrap_or_else(|e| panic!("{dialect}: {e}"));
    }
}

#[test]
fn free_form_vendor_names_are_accepted() {
    for vendor in ["GOODDATA", "HONEYDEW", "WISDOM", "POWER_BI", "SIGMA", "AcmeCorp"] {
        let yaml = format!(
            "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: s\ncustom_extensions:\n  - vendor_name: {vendor}\n    data: '{{}}'\n"
        );
        let doc = parser::from_yaml_str(&yaml).unwrap_or_else(|e| panic!("{vendor}: {e}"));
        assert_eq!(doc.model.custom_extensions[0].vendor_name, vendor);
    }
}

#[test]
fn the_mnml_dialect_is_gone() {
    // `MNML` was never in the Ossie enum; a document using it must now fail
    // like any other unknown dialect.
    let yaml = format!(
        "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: s\n    fields:\n      - name: f\n        expression:\n          dialects:\n            - dialect: MNML\n              col: f\n"
    );
    let err = parser::from_yaml_str(&yaml).expect_err("MNML must not parse");
    assert!(err.to_string().contains("MNML"), "{err}");
}

#[test]
fn dialect_entries_take_only_dialect_and_expression() {
    // Structured sibling keys were the MNML surface form; the schema sets
    // `additionalProperties: false` on a dialect entry.
    let yaml = format!(
        "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: s\n    fields:\n      - name: f\n        expression:\n          dialects:\n            - dialect: ANSI_SQL\n              expression: f\n              col: f\n"
    );
    let err = parser::from_yaml_str(&yaml).expect_err("extra keys must not parse");
    assert!(err.to_string().contains("unknown field"), "{err}");
}

// ---------------------------------------------------------------------------
// One load path: compile performs every check the crate makes.
// ---------------------------------------------------------------------------

/// Structural problems leave the model ambiguous, so they are fatal — and
/// `compile` must catch them, not only `validate`.
#[test]
fn compile_rejects_structural_problems() {
    let duplicate_datasets = format!(
        "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: a\n  - name: d\n    source: b\n"
    );
    let doc = parser::from_yaml_str(&duplicate_datasets).expect("parse");
    let err = CompiledModel::compile(&doc).expect_err("duplicate dataset names are fatal");
    assert!(err.to_string().contains("duplicate dataset name"), "{err}");

    let bad_relationship = format!(
        "version: \"{OSSIE_SPEC_VERSION}\"\nname: m\ndatasets:\n  - name: d\n    source: a\nrelationships:\n  - name: r\n    from: d\n    to: nope\n    from_columns: [x]\n    to_columns: [y]\n"
    );
    let doc = parser::from_yaml_str(&bad_relationship).expect("parse");
    let err = CompiledModel::compile(&doc).expect_err("dangling relationship is fatal");
    assert!(err.to_string().contains("unknown dataset"), "{err}");

    let wrong_version = "version: \"0.1.1\"\nname: m\ndatasets:\n  - name: d\n    source: a\n";
    let doc = parser::from_yaml_str(wrong_version).expect("parse");
    assert!(CompiledModel::compile(&doc).is_err(), "wrong version is fatal");
}

/// A broken expression is not fatal: it costs you that one item.
#[test]
fn compile_keeps_going_past_a_broken_expression() {
    let yaml = format!(
        r#"version: "{OSSIE_SPEC_VERSION}"
name: m
datasets:
  - name: d
    source: a
    fields:
      - name: good
        expression:
          dialects: [{{ dialect: ANSI_SQL, expression: good }}]
metrics:
  - name: usable
    expression:
      dialects: [{{ dialect: ANSI_SQL, expression: "COUNT(*)" }}]
  - name: broken
    expression:
      dialects: [{{ dialect: ANSI_SQL, expression: "RANK() OVER (ORDER BY d.good)" }}]
"#
    );
    let doc = parser::from_yaml_str(&yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("the model still loads");
    assert_eq!(model.issues.len(), 1);
    assert!(model.metric("usable").is_some(), "the good metric survives");
    assert!(model.metric("broken").is_none(), "the broken one is excluded");
}
