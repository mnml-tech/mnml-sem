//! Structural checks on ontology and mapping documents.
//!
//! These need nothing but the document itself. Whether a mapping fits its
//! ontology and its semantic model is a question across three documents, and
//! `ossie-ontology` answers it.

use std::collections::{HashMap, HashSet};

use crate::error::Error;
use crate::{
    ANY, BUILT_IN_CONCEPTS, BUILT_IN_VALUE_TYPES, ConceptMapping, ConceptType, LinkMapping,
    MAPPING_SPEC_VERSION, MappingDocument, Multiplicity, ONTOLOGY_SPEC_VERSION, ObjectMapping,
    OntologyComponent, OntologyDocument, ReferentMapping,
};

/// Check an ontology on its own: names, supertypes, roles, identifiers and
/// IRIs.
pub fn validate_ontology(doc: &OntologyDocument) -> Result<(), Error> {
    let mut issues = Vec::new();

    if doc.version != ONTOLOGY_SPEC_VERSION {
        issues.push(format!(
            "[Version] expected {ONTOLOGY_SPEC_VERSION:?}, got {:?}",
            doc.version
        ));
    }

    let mut declared: HashMap<&str, &OntologyComponent> = HashMap::new();
    for c in &doc.ontology {
        if BUILT_IN_CONCEPTS.contains(&c.concept.as_str()) {
            issues.push(format!(
                "[Concept] {:?} is a built-in concept and cannot be redeclared",
                c.concept
            ));
        } else if declared.insert(&c.concept, c).is_some() {
            issues.push(format!("[Concept] {:?} is declared twice", c.concept));
        }
    }
    let exists = |name: &str| declared.contains_key(name) || BUILT_IN_CONCEPTS.contains(&name);

    for c in &doc.ontology {
        let concept = &c.concept;

        for supertype in &c.extends {
            if !exists(supertype) {
                issues.push(format!(
                    "[Concept] {concept:?} extends {supertype:?}, which is not a concept"
                ));
            }
        }
        match c.concept_type {
            ConceptType::ValueType => {
                if !reaches_built_in_value_type(c, &declared) {
                    issues.push(format!(
                        "[Concept] value type {concept:?} must extend a built-in value type, \
                         directly or through others"
                    ));
                }
            }
            ConceptType::EntityType => {
                for supertype in &c.extends {
                    let is_entity = supertype == ANY
                        || declared
                            .get(supertype.as_str())
                            .is_some_and(|s| s.concept_type == ConceptType::EntityType);
                    if exists(supertype) && !is_entity {
                        issues.push(format!(
                            "[Concept] entity type {concept:?} extends {supertype:?}, which is \
                             not an entity type"
                        ));
                    }
                }
            }
        }

        let mut relationship_names = HashSet::new();
        for r in &c.relationships {
            let id = format!("{concept}.{}", r.name);
            if !relationship_names.insert(r.name.as_str()) {
                issues.push(format!("[Relationship] {id:?} is declared twice"));
            }
            // The first role is the concept itself, named after it.
            let mut role_names = HashSet::from([concept.as_str()]);
            for role in &r.roles {
                if !exists(&role.concept) {
                    issues.push(format!(
                        "[Relationship] {id:?} has a role played by {:?}, which is not a concept",
                        role.concept
                    ));
                }
                if !role_names.insert(role.role_name()) {
                    issues.push(format!(
                        "[Relationship] {id:?} has two roles named {:?}; give one a distinguishing \
                         name",
                        role.role_name()
                    ));
                }
            }
            if r.multiplicity == Some(Multiplicity::OneToOne) && r.arity() != 2 {
                issues.push(format!(
                    "[Relationship] {id:?} is OneToOne, which only a binary relationship can be"
                ));
            }
            check_iri(
                &format!("relationship {id:?}"),
                r.iri.as_deref(),
                doc,
                &mut issues,
            );
        }

        for name in &c.identify_by {
            match c.relationship(name) {
                None => issues.push(format!(
                    "[Concept] {concept:?} is identified by {name:?}, which is not one of its \
                     relationships"
                )),
                Some(r) if r.arity() != 2 => issues.push(format!(
                    "[Concept] {concept:?} is identified by {name:?}, which is not binary"
                )),
                Some(_) => {}
            }
        }
        if !c.identify_by.is_empty() && c.concept_type == ConceptType::ValueType {
            issues.push(format!(
                "[Concept] value type {concept:?} declares identify_by; only entity types are \
                 identified"
            ));
        }

        check_iri(
            &format!("concept {concept:?}"),
            c.iri.as_deref(),
            doc,
            &mut issues,
        );
    }

    finish(issues)
}

/// Whether `c` reaches a built-in value type through `extends`. Cycle-safe.
fn reaches_built_in_value_type(
    c: &OntologyComponent,
    declared: &HashMap<&str, &OntologyComponent>,
) -> bool {
    let mut seen = HashSet::new();
    let mut pending: Vec<&str> = c.extends.iter().map(String::as_str).collect();
    while let Some(name) = pending.pop() {
        if BUILT_IN_VALUE_TYPES.contains(&name) {
            return true;
        }
        if !seen.insert(name) {
            continue;
        }
        if let Some(s) = declared.get(name) {
            pending.extend(s.extends.iter().map(String::as_str));
        }
    }
    false
}

/// A full IRI passes; a QName `prefix:local` needs its prefix declared.
fn check_iri(what: &str, iri: Option<&str>, doc: &OntologyDocument, issues: &mut Vec<String>) {
    let Some(iri) = iri else { return };
    // `scheme://...` and `urn:...` are full IRIs, whatever their scheme.
    if iri.contains("://") || iri.starts_with("urn:") {
        return;
    }
    match iri.split_once(':') {
        Some((prefix, _)) if doc.prefixes.contains_key(prefix) => {}
        Some((prefix, _)) => issues.push(format!(
            "[IRI] {what} uses prefix {prefix:?}, which `prefixes` does not declare"
        )),
        None => issues.push(format!(
            "[IRI] {what} has {iri:?}, which is neither a full IRI nor a prefix:local QName"
        )),
    }
}

/// Check a mapping on its own: the constraints its schema states with `anyOf`,
/// which the types cannot express.
pub fn validate_mapping(doc: &MappingDocument) -> Result<(), Error> {
    let mut issues = Vec::new();
    if doc.version != MAPPING_SPEC_VERSION {
        issues.push(format!(
            "[Version] expected {MAPPING_SPEC_VERSION:?}, got {:?}",
            doc.version
        ));
    }
    if doc.name.is_empty() {
        issues.push("[Mapping] name must not be empty".into());
    }
    for (what, r) in [
        ("ontology_ref", &doc.ontology_ref),
        ("semantic_model_ref", &doc.semantic_model_ref),
    ] {
        if r.name.is_empty() {
            issues.push(format!("[Mapping] {what}.name must not be empty"));
        }
    }
    for cm in &doc.concept_mappings {
        check_concept_mapping(cm, &mut issues);
    }
    finish(issues)
}

fn check_concept_mapping(cm: &ConceptMapping, issues: &mut Vec<String>) {
    let at = format!("concept mapping {:?}", cm.concept);
    if cm.object_mappings.is_empty() && cm.link_mappings.is_empty() {
        issues.push(format!(
            "[Mapping] {at} has neither object_mappings nor link_mappings"
        ));
    }
    for om in &cm.object_mappings {
        check_object_mapping(&at, om, issues);
    }
    for lm in &cm.link_mappings {
        check_link_mapping(&at, lm, issues);
    }
}

fn check_link_mapping(at: &str, lm: &LinkMapping, issues: &mut Vec<String>) {
    check_object_mapping(at, &lm.object_mapping, issues);
    for child in &lm.children {
        check_link_mapping(at, child, issues);
    }
}

fn check_object_mapping(at: &str, om: &ObjectMapping, issues: &mut Vec<String>) {
    if om.expression.is_none() && om.referent_mappings.is_empty() {
        issues.push(format!(
            "[Mapping] {at} has an object mapping with neither expression nor referent_mappings"
        ));
    }
    for rm in &om.referent_mappings {
        check_referent_mapping(at, rm, issues);
    }
}

fn check_referent_mapping(at: &str, rm: &ReferentMapping, issues: &mut Vec<String>) {
    if rm.expression.is_none() && rm.referent_mappings.is_empty() {
        issues.push(format!(
            "[Mapping] {at} maps referent {:?} with neither expression nor referent_mappings",
            rm.relationship
        ));
    }
    for nested in &rm.referent_mappings {
        check_referent_mapping(at, nested, issues);
    }
}

fn finish(issues: Vec<String>) -> Result<(), Error> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(issues))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ontology_from_yaml_str;

    fn issues(yaml: &str) -> Vec<String> {
        let doc = ontology_from_yaml_str(yaml).expect("parse");
        match validate_ontology(&doc) {
            Ok(()) => Vec::new(),
            Err(Error::Validation(issues)) => issues,
            Err(e) => panic!("{e}"),
        }
    }

    fn ontology(components: &str) -> String {
        format!("version: \"0.2.0.dev0\"\nname: o\nontology:\n{components}")
    }

    #[test]
    fn a_well_formed_ontology_passes() {
        let yaml = ontology(
            r#"
  - concept: Nr
    type: ValueType
    extends: [Integer]
  - concept: Person
    type: EntityType
    identify_by: [nr]
    relationships:
      - name: nr
        roles: [{ concept: Nr }]
        multiplicity: OneToOne
        verbalizes: ["{Person} has {Nr}"]
"#,
        );
        assert_eq!(issues(&yaml), Vec::<String>::new());
    }

    #[test]
    fn a_value_type_must_reach_a_built_in() {
        let yaml = ontology(
            r#"
  - concept: A
    type: ValueType
    extends: [B]
  - concept: B
    type: ValueType
    extends: [A]
"#,
        );
        let issues = issues(&yaml);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues[0].contains("must extend a built-in value type"));
    }

    #[test]
    fn an_identifier_must_be_a_binary_relationship_of_the_concept() {
        let yaml = ontology(
            r#"
  - concept: Person
    type: EntityType
    identify_by: [nr, missing]
    relationships:
      - name: nr
        verbalizes: ["{Person} is numbered"]
"#,
        );
        let issues = issues(&yaml);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("\"nr\", which is not binary"))
        );
        assert!(
            issues
                .iter()
                .any(|i| i.contains("\"missing\", which is not one of"))
        );
    }

    #[test]
    fn a_repeated_role_needs_a_name() {
        let yaml = ontology(
            r#"
  - concept: Store
    type: EntityType
    relationships:
      - name: ships_to
        roles: [{ concept: Store }]
        verbalizes: ["{Store} ships to {Store}"]
      - name: supplies
        roles: [{ concept: Store, name: destination }]
        verbalizes: ["{Store} supplies {Store:destination}"]
"#,
        );
        let issues = issues(&yaml);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("Store.ships_to"));
    }

    #[test]
    fn a_qname_needs_its_prefix() {
        let yaml = format!(
            "{}prefixes:\n  foaf: http://xmlns.com/foaf/0.1/\n",
            ontology(
                r#"
  - concept: Agent
    type: EntityType
    iri: foaf:Agent
  - concept: Org
    type: EntityType
    iri: org:Organization
  - concept: Place
    type: EntityType
    iri: http://schema.org/Place
"#,
            )
        );
        let issues = issues(&yaml);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("\"org\""));
    }
}
