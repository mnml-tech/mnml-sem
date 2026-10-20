//! Conformance of the ontology and mapping types to their vendored schemas
//! (`spec/ontology.json`, `spec/mapping.json`), as `conformance.rs` does for the
//! core spec.
//!
//! Both schemas `$ref` the core schema, and the mapping schema `$ref`s the
//! ontology schema, by their raw GitHub URLs. Those URLs are registered as the
//! vendored copies, so validation runs offline against exactly what is
//! vendored.

use std::collections::BTreeSet;

use ossie_model::parser;
use ossie_model::{ConceptType, MAPPING_SPEC_VERSION, Multiplicity, ONTOLOGY_SPEC_VERSION};
use serde_json::Value;

const CORE_SRC: &str = include_str!("../spec/ossie-schema.json");
const ONTOLOGY_SRC: &str = include_str!("../spec/ontology.json");
const MAPPING_SRC: &str = include_str!("../spec/mapping.json");

const CORE_URL: &str =
    "https://raw.githubusercontent.com/apache/ossie/main/core-spec/ossie-schema.json";
const ONTOLOGY_URL: &str =
    "https://raw.githubusercontent.com/apache/ossie/main/ontology/ontology.json";

fn json(src: &str) -> Value {
    serde_json::from_str(src).expect("vendored schema is valid JSON")
}

/// Serves the vendored schemas at the URLs they `$ref` each other by, and
/// refuses anything else, so a validation never reaches the network.
struct Vendored;

impl jsonschema::Retrieve for Vendored {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        match uri.as_str() {
            CORE_URL => Ok(json(CORE_SRC)),
            ONTOLOGY_URL => Ok(json(ONTOLOGY_SRC)),
            other => Err(format!("{other} is not vendored").into()),
        }
    }
}

fn validator(schema_src: &str) -> jsonschema::Validator {
    jsonschema::options()
        .with_retriever(Vendored)
        .build(&json(schema_src))
        .expect("vendored schema compiles")
}

fn errors(validator: &jsonschema::Validator, doc: &Value) -> Vec<String> {
    validator
        .iter_errors(doc)
        .map(|e| format!("  at {}: {e}", e.instance_path()))
        .collect()
}

fn yaml(name: &str, src: &str) -> Value {
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(src).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::to_value(yaml).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn ontology_fixtures() -> Vec<(&'static str, Value)> {
    [
        (
            "steelwheels.ontology.yaml",
            include_str!("fixtures/steelwheels.ontology.yaml"),
        ),
        (
            "flights.ontology.yaml",
            include_str!("fixtures/flights.ontology.yaml"),
        ),
    ]
    .into_iter()
    .map(|(name, src)| (name, yaml(name, src)))
    .collect()
}

fn mapping_fixtures() -> Vec<(&'static str, Value)> {
    [
        (
            "steelwheels.mapping.yaml",
            include_str!("fixtures/steelwheels.mapping.yaml"),
        ),
        (
            "flights.mapping.yaml",
            include_str!("fixtures/flights.mapping.yaml"),
        ),
    ]
    .into_iter()
    .map(|(name, src)| (name, yaml(name, src)))
    .collect()
}

fn assert_valid(validator: &jsonschema::Validator, docs: &[(&str, Value)]) {
    let failures: Vec<String> = docs
        .iter()
        .filter_map(|(name, doc)| {
            let errs = errors(validator, doc);
            (!errs.is_empty()).then(|| format!("{name}:\n{}", errs.join("\n")))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn ontology_fixtures_are_valid_ontologies() {
    assert_valid(&validator(ONTOLOGY_SRC), &ontology_fixtures());
}

#[test]
fn mapping_fixtures_are_valid_mappings() {
    assert_valid(&validator(MAPPING_SRC), &mapping_fixtures());
}

/// What this crate writes back must be as legal as what it read.
#[test]
fn serialized_documents_are_valid() {
    let ontology_validator = validator(ONTOLOGY_SRC);
    for (name, doc) in ontology_fixtures() {
        let parsed = parser::ontology_from_json_str(&doc.to_string())
            .unwrap_or_else(|e| panic!("{name} should parse: {e}"));
        let errs = errors(&ontology_validator, &serde_json::to_value(&parsed).unwrap());
        assert!(errs.is_empty(), "{name}:\n{}", errs.join("\n"));
    }
    let mapping_validator = validator(MAPPING_SRC);
    for (name, doc) in mapping_fixtures() {
        let parsed = parser::mapping_from_json_str(&doc.to_string())
            .unwrap_or_else(|e| panic!("{name} should parse: {e}"));
        let errs = errors(&mapping_validator, &serde_json::to_value(&parsed).unwrap());
        assert!(errs.is_empty(), "{name}:\n{}", errs.join("\n"));
    }
}

#[test]
fn spec_versions_match_the_schemas() {
    for (ours, src) in [
        (ONTOLOGY_SPEC_VERSION, ONTOLOGY_SRC),
        (MAPPING_SPEC_VERSION, MAPPING_SRC),
    ] {
        let schema = json(src);
        assert_eq!(
            Some(ours),
            schema["properties"]["version"]["const"].as_str(),
            "{}",
            schema["title"]
        );
    }
}

fn schema_enum(def: &str) -> Vec<String> {
    json(ONTOLOGY_SRC)["$defs"][def]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("$defs.{def} has no enum"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn wire<T: serde::Serialize>(values: &[T]) -> Vec<String> {
    values
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn enums_match_the_schema() {
    assert_eq!(
        wire(&[ConceptType::EntityType, ConceptType::ValueType]),
        schema_enum("ConceptType")
    );
    assert_eq!(
        wire(&[Multiplicity::ManyToOne, Multiplicity::OneToOne]),
        schema_enum("Multiplicity")
    );
}

/// An ontology using every property the schema allows, the deprecated
/// embedded mapping included.
fn everything_ontology() -> Value {
    serde_json::json!({
        "version": ONTOLOGY_SPEC_VERSION,
        "name": "everything",
        "description": "uses every property",
        "ai_context": "an ontology",
        "requires": ["COUNT[Person] > 0"],
        "prefixes": { "foaf": "http://xmlns.com/foaf/0.1/" },
        "ontology": [
            { "concept": "Nr", "type": "ValueType", "extends": ["Integer"] },
            {
                "concept": "Person",
                "type": "EntityType",
                "description": "a person",
                "extends": ["Any"],
                "derived_by": ["Person.nr > 0"],
                "identify_by": ["nr"],
                "requires": ["Person.nr"],
                "iri": "foaf:Person",
                "relationships": [{
                    "name": "nr",
                    "description": "identifier",
                    "roles": [{ "concept": "Nr", "name": "number" }],
                    "multiplicity": "OneToOne",
                    "derived_by": ["true"],
                    "requires": ["true"],
                    "verbalizes": ["{Person} has {Nr}"],
                    "iri": "foaf:nr"
                }]
            }
        ],
        "ontology_mappings": [{
            "name": "embedded",
            "description": "deprecated form",
            "semantic_model": {
                "version": "0.2.0.dev0",
                "name": "m",
                "datasets": [{ "name": "p", "source": "s.p", "fields": [] }]
            },
            "concept_mappings": [everything_concept_mapping()]
        }]
    })
}

fn everything_concept_mapping() -> Value {
    serde_json::json!({
        "concept": "Person",
        "object_mappings": [{
            "concept": "Person",
            "referent_mappings": [{
                "relationship": "nr",
                "expression": "p.nr",
                "referent_mappings": [{ "relationship": "x", "expression": "p.x" }]
            }],
            "expression": "p.nr"
        }],
        "link_mappings": [{
            "relationship": "nr",
            "object_mapping": { "expression": "p.nr" },
            "children": [{ "object_mapping": { "expression": "p.nr" } }]
        }]
    })
}

fn everything_mapping() -> Value {
    serde_json::json!({
        "version": MAPPING_SPEC_VERSION,
        "name": "everything",
        "description": "uses every property",
        "ontology_ref": { "name": "everything", "iri": "./everything.ontology.yaml" },
        "semantic_model_ref": { "name": "m", "iri": "./m.yaml" },
        "concept_mappings": [everything_concept_mapping()],
        "custom_extensions": [{ "vendor_name": "ACME", "data": "{}" }]
    })
}

/// Every property name that appears anywhere in `v`.
fn keys(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.insert(k.clone());
                keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| keys(i, out)),
        _ => {}
    }
}

/// Every property the schema declares, at the root and in each `$def`.
fn declared_properties(schema: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |props: &Value| {
        if let Some(map) = props.as_object() {
            out.extend(map.keys().cloned());
        }
    };
    add(&schema["properties"]);
    for def in schema["$defs"].as_object().into_iter().flatten() {
        add(&def.1["properties"]);
    }
    out
}

/// A document the schema accepts must parse here. The test documents use every
/// property the schemas declare, so a property added upstream fails here until
/// the types and these documents learn it.
#[test]
fn the_parser_is_not_stricter_than_the_schemas() {
    let ontology = everything_ontology();
    let mapping = everything_mapping();
    assert_valid(
        &validator(ONTOLOGY_SRC),
        &[("everything ontology", ontology.clone())],
    );
    assert_valid(
        &validator(MAPPING_SRC),
        &[("everything mapping", mapping.clone())],
    );

    parser::ontology_from_json_str(&ontology.to_string())
        .expect("an ontology the schema accepts must parse");
    parser::mapping_from_json_str(&mapping.to_string())
        .expect("a mapping the schema accepts must parse");

    let mut used = BTreeSet::new();
    keys(&ontology, &mut used);
    keys(&mapping, &mut used);
    let mut declared = declared_properties(&json(ONTOLOGY_SRC));
    declared.extend(declared_properties(&json(MAPPING_SRC)));
    let unused: Vec<&String> = declared.difference(&used).collect();
    assert!(
        unused.is_empty(),
        "the schemas declare properties the test documents do not use: {unused:?}"
    );
}

/// The validators enforce the schemas, including what reaches them through a
/// `$ref` to another vendored schema: a passing fixture proves nothing if
/// every document passes.
#[test]
fn the_validators_reject_what_the_schemas_reject() {
    let mapping = validator(MAPPING_SRC);
    let mut bad = everything_mapping();
    bad.as_object_mut().unwrap().remove("ontology_ref");
    assert!(!errors(&mapping, &bad).is_empty(), "a missing ontology_ref");

    // `ConceptMapping` lives in ontology.json; the mapping reaches it by URL.
    let mut bad = everything_mapping();
    bad["concept_mappings"][0]["unexpected"] = Value::Bool(true);
    assert!(
        !errors(&mapping, &bad).is_empty(),
        "an unknown concept mapping property"
    );

    let mut bad = everything_mapping();
    bad["concept_mappings"][0]
        .as_object_mut()
        .unwrap()
        .retain(|k, _| k == "concept");
    assert!(
        !errors(&mapping, &bad).is_empty(),
        "a concept mapping that maps nothing"
    );

    let ontology = validator(ONTOLOGY_SRC);
    let mut bad = everything_ontology();
    bad["ontology"][1]["type"] = Value::String("Thing".into());
    assert!(
        !errors(&ontology, &bad).is_empty(),
        "an unknown concept type"
    );
}
