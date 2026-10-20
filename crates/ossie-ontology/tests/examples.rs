//! The checker against whole documents: the Steel Wheels mapping written for
//! this repository, upstream's flights example, and Steel Wheels with one thing
//! broken at a time.

use ossie_model::parser;
use ossie_ontology::{Report, Severity};

const ONTOLOGY: &str = include_str!("../../ossie-model/tests/fixtures/steelwheels.ontology.yaml");
const MAPPING: &str = include_str!("../../ossie-model/tests/fixtures/steelwheels.mapping.yaml");
const MODEL: &str = include_str!("../../ossie-model/tests/fixtures/steelwheels.yaml");

fn check(ontology: &str, mapping: &str, model: &str) -> Report {
    let ontology = parser::ontology_from_yaml_str(ontology).expect("parse ontology");
    let mapping = parser::mapping_from_yaml_str(mapping).expect("parse mapping");
    let model = parser::from_yaml_str(model).expect("parse model");
    ossie_ontology::check(&mapping, &ontology, &model.model)
}

/// `src` with `from` replaced by `to`, which must occur exactly once.
fn edit(src: &str, from: &str, to: &str) -> String {
    assert_eq!(src.matches(from).count(), 1, "{from:?} must occur once");
    src.replacen(from, to, 1)
}

/// The one error, or a panic listing everything that was found.
fn only_error(report: &Report) -> String {
    let errors: Vec<_> = report.errors().collect();
    assert_eq!(errors.len(), 1, "{:#?}", report.issues);
    format!("{}: {}", errors[0].at, errors[0].message)
}

#[test]
fn steelwheels_maps_cleanly_but_for_one_unkeyed_claim() {
    let report = check(ONTOLOGY, MAPPING, MODEL);
    assert!(report.is_ok(), "{:#?}", report.issues);
    let warnings: Vec<_> = report.warnings().collect();
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert_eq!(
        warnings[0].at,
        "concept_mappings[1](Country).link_mappings[0].children[0]"
    );
    assert_eq!(
        warnings[0].message,
        "Country.in_territory is ManyToOne, but country is not a key of customer_w_ter, so the \
         dataset does not guarantee one \"Territory\" per country"
    );
}

/// Upstream's own example. Its two filtered flight mappings are not SQL, and
/// its semantic model declares no keys, so no many-to-one claim is backed.
#[test]
fn the_upstream_flights_example() {
    let report = check(
        include_str!("../../ossie-model/tests/fixtures/flights.ontology.yaml"),
        include_str!("../../ossie-model/tests/fixtures/flights.mapping.yaml"),
        include_str!("../../ossie-model/tests/fixtures/flights.semantic_model.yaml"),
    );
    let errors: Vec<String> = report.errors().map(|e| e.message.clone()).collect();
    assert_eq!(errors.len(), 2, "{errors:#?}");
    for (error, flag) in errors.iter().zip(["diverted", "cancelled"]) {
        assert!(
            error.contains(&format!("FLIGHT.{flag} == TRUE")) && error.contains("WHERE"),
            "{error}"
        );
    }
    let warnings: Vec<_> = report.warnings().collect();
    assert!(!warnings.is_empty());
    for w in warnings {
        assert!(
            w.message.contains("declares no primary or unique key"),
            "{w}"
        );
    }
}

#[test]
fn the_references_must_name_the_documents_checked() {
    let mapping = edit(MAPPING, "  name: SteelWheels\n", "  name: Steelwheels\n");
    assert!(
        only_error(&check(ONTOLOGY, &mapping, MODEL)).contains("names ontology \"Steelwheels\"")
    );
}

#[test]
fn a_concept_mapping_names_a_concept() {
    let mapping = edit(MAPPING, "  - concept: Territory\n", "  - concept: Region\n");
    assert_eq!(
        only_error(&check(ONTOLOGY, &mapping, MODEL)),
        "concept_mappings[0](Region): \"Region\" is not a concept of the ontology"
    );
}

#[test]
fn an_expression_reads_declared_fields() {
    let mapping = edit(
        MAPPING,
        "expression: customer_w_ter.state",
        "expression: customer_w_ter.province",
    );
    let error = only_error(&check(ONTOLOGY, &mapping, MODEL));
    assert!(error.contains("customer_w_ter.province"), "{error}");
}

/// Every mapping that reads the broken field says so: City's own, and the
/// City a customer is located in.
#[test]
fn an_expression_over_a_field_that_does_not_compile() {
    let model = edit(MODEL, "expression: city\n", "expression: \"city(\"\n");
    let report = check(ONTOLOGY, MAPPING, &model);
    let errors: Vec<&str> = report.errors().map(|e| e.at.as_str()).collect();
    assert_eq!(
        errors,
        [
            "concept_mappings[2](City).object_mappings[0].referent_mappings[0]",
            "concept_mappings[3](Customer).link_mappings[0].children[0].object_mapping.\
             referent_mappings[0]",
        ]
    );
    for e in report.errors() {
        assert!(
            e.message
                .starts_with("reads customer_w_ter.city, which does not compile"),
            "{e}"
        );
    }
}

#[test]
fn a_compound_identifier_is_mapped_whole() {
    // City is identified by name and country; drop the country.
    let mapping = edit(
        MAPPING,
        "          - relationship: country
            referent_mappings:
              - relationship: name
                expression: customer_w_ter.country
",
        "",
    );
    assert_eq!(
        only_error(&check(ONTOLOGY, &mapping, MODEL)),
        "concept_mappings[2](City).object_mappings[0]: does not map country, part of \"City\"'s \
         identifier"
    );
}

#[test]
fn an_expression_finds_only_simply_identified_entities() {
    let mapping = edit(
        MAPPING,
        "  - concept: City
    object_mappings:
      - referent_mappings:
          - relationship: name
            expression: customer_w_ter.city
          - relationship: country
            referent_mappings:
              - relationship: name
                expression: customer_w_ter.country
",
        "  - concept: City
    object_mappings:
      - expression: customer_w_ter.city
",
    );
    let error = only_error(&check(ONTOLOGY, &mapping, MODEL));
    assert!(
        error.contains("\"City\" has no simple identifier"),
        "{error}"
    );
}

/// A referent through a relationship that does not identify finds nothing,
/// and leaves the identifier itself unmapped.
#[test]
fn a_referent_names_an_identifying_relationship() {
    let mapping = edit(
        MAPPING,
        "          - relationship: number
            expression: orderfact.customernumber",
        "          - relationship: in_state
            expression: orderfact.customernumber",
    );
    let report = check(ONTOLOGY, &mapping, MODEL);
    let errors: Vec<String> = report
        .errors()
        .map(|e| format!("{}: {}", e.at, e.message))
        .collect();
    assert_eq!(
        errors,
        [
            "concept_mappings[3](Customer).object_mappings[1].referent_mappings[0]: \
             Customer.in_state is not one of \"Customer\"'s identifying relationships (number)",
            "concept_mappings[3](Customer).object_mappings[1]: does not map number, part of \
             \"Customer\"'s identifier",
        ]
    );
}

#[test]
fn a_link_mapping_names_a_relationship_of_its_depth() {
    // in_state is binary; at the top of the tree it would be unary.
    let mapping = edit(
        MAPPING,
        "    link_mappings:
      - object_mapping:
          referent_mappings:
            - relationship: number",
        "    link_mappings:
      - relationship: in_state
        object_mapping:
          referent_mappings:
            - relationship: number",
    );
    let error = only_error(&check(ONTOLOGY, &mapping, MODEL));
    assert!(
        error.contains("Customer.in_state has 2 roles, but this link mapping is at depth 1"),
        "{error}"
    );
}

#[test]
fn a_role_is_filled_by_its_concept() {
    let mapping = edit(
        MAPPING,
        "          - relationship: located_in
            object_mapping:
              concept: City",
        "          - relationship: located_in
            object_mapping:
              concept: Country",
    );
    let error = only_error(&check(ONTOLOGY, &mapping, MODEL));
    assert!(
        error.contains("maps \"Country\" where \"City\" is expected"),
        "{error}"
    );
}

#[test]
fn only_entity_types_are_populated_by_object_mappings() {
    let mapping = format!(
        "{MAPPING}
  - concept: Year
    object_mappings:
      - expression: dates.year_id
"
    );
    let error = only_error(&check(ONTOLOGY, &mapping, MODEL));
    assert!(error.contains("\"Year\" is a value type"), "{error}");
}

/// The ontology's own structural problems come back through the same report.
#[test]
fn structural_problems_are_reported_too() {
    let ontology = edit(
        ONTOLOGY,
        "    identify_by: [number]\n",
        "    identify_by: [nmber]\n",
    );
    let report = check(&ontology, MAPPING, MODEL);
    let errors: Vec<_> = report.errors().collect();
    assert!(
        errors
            .iter()
            .any(|e| e.at == "ontology" && e.message.contains("\"nmber\"")),
        "{errors:#?}"
    );
    assert!(errors.iter().all(|e| e.severity == Severity::Error));
}

/// Keying the dataset by what decides the territory settles the warning.
#[test]
fn a_key_backs_a_many_to_one_claim() {
    let model = edit(
        MODEL,
        "    unique_keys:\n      - [customernumber]\n",
        "    unique_keys:\n      - [customernumber]\n      - [country, territory]\n",
    );
    let report = check(ONTOLOGY, MAPPING, &model);
    // Not a key of country alone: still not guaranteed.
    assert_eq!(report.warnings().count(), 1, "{:#?}", report.issues);

    let model = edit(
        MODEL,
        "    unique_keys:\n      - [customernumber]\n",
        "    unique_keys:\n      - [customernumber]\n      - [country]\n",
    );
    let report = check(ONTOLOGY, MAPPING, &model);
    assert!(report.issues.is_empty(), "{:#?}", report.issues);
}
