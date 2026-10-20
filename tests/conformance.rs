//! Conformance against the vendored Apache Ossie schema.
//!
//! This crate's `model` types are an implementation of a spec it does not own,
//! so the failure mode that matters is silent drift: upstream adds a dialect,
//! renames a field, or moves the spec version, and nothing here notices until
//! a real document fails to parse.
//!
//! These tests pin the two together. They read `spec/ossie-schema.json` —
//! vendored, see [`spec/README.md`] — so they run offline; refreshing that file
//! is what surfaces an upstream change, as a failing test naming exactly what
//! moved.
//!
//! [`spec/README.md`]: https://github.com/apache/ossie/blob/main/core-spec/ossie-schema.json

use ossie_substrait::model::{DataType, Dialect, OSSIE_SPEC_VERSION};
use ossie_substrait::parser;
use serde_json::Value;

const SCHEMA_SRC: &str = include_str!("../spec/ossie-schema.json");

fn schema() -> Value {
    serde_json::from_str(SCHEMA_SRC).expect("vendored schema is valid JSON")
}

fn validator() -> jsonschema::Validator {
    jsonschema::validator_for(&schema()).expect("vendored schema compiles")
}

/// Every error, so one run tells you everything that is wrong.
fn errors(validator: &jsonschema::Validator, doc: &Value) -> Vec<String> {
    validator
        .iter_errors(doc)
        .map(|e| format!("  at {}: {e}", e.instance_path()))
        .collect()
}

fn fixture_docs() -> Vec<(&'static str, Value)> {
    [
        ("minimal.yaml", include_str!("fixtures/minimal.yaml")),
        (
            "steelwheels.yaml",
            include_str!("fixtures/steelwheels.yaml"),
        ),
        (
            "disconnected.yaml",
            include_str!("fixtures/disconnected.yaml"),
        ),
        (
            "expressions.yaml",
            include_str!("fixtures/expressions.yaml"),
        ),
        (
            "people_daily_tenant.yaml",
            include_str!("fixtures/people_daily_tenant.yaml"),
        ),
    ]
    .into_iter()
    .map(|(name, src)| {
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = serde_json::to_value(yaml).unwrap_or_else(|e| panic!("{name}: {e}"));
        (name, json)
    })
    .collect()
}

/// The fixtures must be legal Ossie documents, not merely documents this crate
/// happens to parse.
#[test]
fn fixtures_are_valid_ossie_documents() {
    let validator = validator();
    let mut failures = Vec::new();
    for (name, doc) in fixture_docs() {
        let errs = errors(&validator, &doc);
        if !errs.is_empty() {
            failures.push(format!("{name}:\n{}", errs.join("\n")));
        }
    }
    assert!(
        failures.is_empty(),
        "fixtures do not match the Ossie schema:\n\n{}",
        failures.join("\n\n")
    );
}

/// What this crate writes must also be legal Ossie, not only what it reads.
#[test]
fn serialized_documents_are_valid_ossie() {
    let validator = validator();
    for (name, doc) in fixture_docs() {
        let parsed = parser::from_json_str(&doc.to_string())
            .unwrap_or_else(|e| panic!("{name} should parse: {e}"));
        let round_tripped = serde_json::to_value(&parsed).unwrap_or_else(|e| panic!("{name}: {e}"));
        let errs = errors(&validator, &round_tripped);
        assert!(
            errs.is_empty(),
            "{name} does not round-trip to a valid document:\n{}",
            errs.join("\n")
        );
    }
}

/// The version this crate targets is the version the vendored schema pins.
/// Refreshing the schema without moving the constant fails here.
#[test]
fn spec_version_matches_the_schema() {
    let schema = schema();
    let expected = schema["properties"]["version"]["const"]
        .as_str()
        .expect("the schema pins a version const");
    assert_eq!(
        OSSIE_SPEC_VERSION, expected,
        "the vendored schema targets {expected:?} but this crate targets {OSSIE_SPEC_VERSION:?}"
    );
}

/// The wire form of every enum value, in schema order.
fn schema_enum(schema: &Value, def: &str) -> Vec<String> {
    schema["$defs"][def]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("$defs.{def} has no enum"))
        .iter()
        .map(|v| v.as_str().expect("enum values are strings").to_string())
        .collect()
}

/// A missing dialect is not a parse error you find later — it is a document
/// this crate rejects outright. `SCREAMING_SNAKE_CASE` also mangles `BIGQUERY`
/// and `THOUGHTSPOT`, so the wire forms are checked, not the variant names.
#[test]
fn dialect_enum_matches_the_schema() {
    let ours: Vec<String> = [
        Dialect::AnsiSql,
        Dialect::Snowflake,
        Dialect::Mdx,
        Dialect::Tableau,
        Dialect::Databricks,
        Dialect::Maql,
        Dialect::BigQuery,
        Dialect::Sigma,
        Dialect::ThoughtSpot,
        Dialect::Dax,
        Dialect::HolisticsAql,
        Dialect::OssieSql2026,
    ]
    .iter()
    .map(|d| {
        serde_json::to_value(d)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();

    assert_sets_match("Dialect", &schema_enum(&schema(), "Dialect"), &ours);
}

#[test]
fn datatype_enum_matches_the_schema() {
    let ours: Vec<String> = [
        DataType::String,
        DataType::Integer,
        DataType::Decimal,
        DataType::Float,
        DataType::Boolean,
        DataType::Date,
        DataType::Time,
        DataType::DateTime,
        DataType::DateTimeTz,
        DataType::Opaque,
    ]
    .iter()
    .map(|d| {
        serde_json::to_value(d)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();

    assert_sets_match("DataType", &schema_enum(&schema(), "DataType"), &ours);
}

/// Report both directions: what upstream added, and what we have that it does
/// not. Order is irrelevant, membership is not.
fn assert_sets_match(what: &str, schema_values: &[String], ours: &[String]) {
    let missing: Vec<&String> = schema_values.iter().filter(|v| !ours.contains(v)).collect();
    let extra: Vec<&String> = ours.iter().filter(|v| !schema_values.contains(v)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what} has drifted from the schema\n  missing here: {missing:?}\n  not in schema: {extra:?}"
    );
}

/// The root's required fields are the ones this crate makes non-optional.
#[test]
fn required_root_fields_match_the_schema() {
    let schema = schema();
    let mut required: Vec<String> = schema["required"]
        .as_array()
        .expect("the root declares required fields")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    required.sort();

    // Everything else on the root is optional in this crate's types.
    assert_eq!(
        required,
        vec![
            "datasets".to_string(),
            "name".to_string(),
            "version".to_string()
        ],
        "the root's required fields changed upstream; OssieDocument must follow"
    );
}

/// A document this crate rejects must be one the schema rejects too — a
/// stricter parser would refuse legal Ossie.
#[test]
fn the_parser_is_not_stricter_than_the_schema() {
    let validator = validator();

    // Every optional field the schema allows, together.
    let doc = serde_json::json!({
        "version": OSSIE_SPEC_VERSION,
        "name": "everything",
        "description": "uses every optional field the schema permits",
        "ai_context": { "instructions": "hi", "synonyms": ["a"], "examples": ["b"] },
        "datasets": [{
            "name": "d",
            "source": "a.b.c",
            "primary_key": ["id"],
            "unique_keys": [["id"], ["a", "b"]],
            "description": "a dataset",
            "ai_context": "plain string form",
            "fields": [{
                "name": "id",
                "expression": { "dialects": [{ "dialect": "ANSI_SQL", "expression": "id" }] },
                "dimension": { "is_time": false },
                "label": "Identifier",
                "description": "the id",
                "datatype": "Integer",
                "ai_context": { "synonyms": ["key"] },
                "custom_extensions": [{ "vendor_name": "ACME", "data": "{}" }]
            }],
            "custom_extensions": [{ "vendor_name": "DBT", "data": "{}" }]
        }],
        "relationships": [],
        "metrics": [{
            "name": "n",
            "expression": { "dialects": [{ "dialect": "ANSI_SQL", "expression": "COUNT(*)" }] },
            "description": "a count",
            "datatype": "Integer",
            "ai_context": "counts",
            "custom_extensions": []
        }],
        "custom_extensions": [{ "vendor_name": "SNOWFLAKE", "data": "{}" }]
    });

    let errs = errors(&validator, &doc);
    assert!(
        errs.is_empty(),
        "the test document is not valid Ossie:\n{}",
        errs.join("\n")
    );

    parser::from_json_str(&doc.to_string())
        .expect("a document the schema accepts must parse here too");
}

/// Dataset-scoped metrics are implemented ahead of the schema (ADR-0014), so
/// this crate accepts `datasets[].metrics` while the vendored schema does not.
///
/// When a refresh brings the property in, this fails. Then: add
/// `dataset_metrics.yaml` to [`fixture_docs`], add `metrics` to the dataset in
/// [`the_parser_is_not_stricter_than_the_schema`], check the merged rules
/// against ADR-0014, and delete this test.
#[test]
fn dataset_metrics_are_still_ahead_of_the_schema() {
    let schema = schema();
    let dataset = &schema["$defs"]["Dataset"]["properties"];
    assert!(
        dataset.is_object(),
        "the schema no longer has $defs.Dataset.properties"
    );
    assert!(
        dataset.get("metrics").is_none(),
        "the vendored schema now defines datasets[].metrics; this crate implemented it ahead \
         of the spec, so reconcile the two (see the comment on this test) and remove it"
    );
}
