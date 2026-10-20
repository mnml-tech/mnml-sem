//! Lookups over an ontology's concepts: kinds, supertypes, relationships and
//! identifiers, with the built-in concepts included.

use std::collections::{HashMap, HashSet};

use ossie_model::{
    ANY, BUILT_IN_CONCEPTS, ConceptType, OntologyComponent, OntologyDocument, OntologyRelationship,
};

pub(crate) struct Concepts<'a> {
    declared: HashMap<&'a str, &'a OntologyComponent>,
}

impl<'a> Concepts<'a> {
    pub(crate) fn new(ontology: &'a OntologyDocument) -> Self {
        Self {
            declared: ontology
                .ontology
                .iter()
                .map(|c| (c.concept.as_str(), c))
                .collect(),
        }
    }

    /// Entity or value type; `None` when no such concept exists.
    pub(crate) fn kind(&self, concept: &str) -> Option<ConceptType> {
        match self.declared.get(concept) {
            Some(c) => Some(c.concept_type),
            None if concept == ANY => Some(ConceptType::EntityType),
            None if BUILT_IN_CONCEPTS.contains(&concept) => Some(ConceptType::ValueType),
            None => None,
        }
    }

    pub(crate) fn exists(&self, concept: &str) -> bool {
        self.kind(concept).is_some()
    }

    /// `concept` followed by its supertypes, nearest first. Cycle-safe.
    fn lineage(&self, concept: &'a str) -> Vec<&'a str> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut pending = vec![concept];
        while let Some(name) = pending.pop() {
            if !seen.insert(name) {
                continue;
            }
            order.push(name);
            if let Some(c) = self.declared.get(name) {
                pending.extend(c.extends.iter().rev().map(String::as_str));
            }
        }
        order
    }

    /// Whether every object of `sub` is an object of `sup`.
    pub(crate) fn is_a(&self, sub: &str, sup: &str) -> bool {
        if sub == sup {
            return true;
        }
        if sup == ANY {
            return self.kind(sub) == Some(ConceptType::EntityType);
        }
        self.declared
            .get_key_value(sub)
            .is_some_and(|(name, _)| self.lineage(name).contains(&sup))
    }

    /// The relationship `name` of `owner`: `local` or `Concept.local`,
    /// declared on `owner` or inherited from a supertype.
    pub(crate) fn relationship(
        &self,
        owner: &str,
        name: &str,
    ) -> Option<(&'a str, &'a OntologyRelationship)> {
        let (qualifier, local) = match name.split_once('.') {
            Some((q, l)) => (Some(q), l),
            None => (None, name),
        };
        let (owner, _) = self.declared.get_key_value(owner)?;
        self.lineage(owner)
            .into_iter()
            .filter(|c| qualifier.is_none_or(|q| q == *c))
            .find_map(|c| {
                let component = self.declared.get(c)?;
                Some((component.concept.as_str(), component.relationship(local)?))
            })
    }

    /// The relationships identifying `concept`: its own `identify_by`, or the
    /// nearest supertype's. Empty when it has none.
    pub(crate) fn identifiers(&self, concept: &str) -> &'a [String] {
        let Some((name, _)) = self.declared.get_key_value(concept) else {
            return &[];
        };
        self.lineage(name)
            .into_iter()
            .filter_map(|c| self.declared.get(c))
            .map(|c| c.identify_by.as_slice())
            .find(|ids| !ids.is_empty())
            .unwrap_or(&[])
    }

    /// The value type identifying `concept` on its own, when it has a simple
    /// identifier: one binary relationship to a value type.
    pub(crate) fn simple_identifier(&self, concept: &str) -> Option<&'a str> {
        let [only] = self.identifiers(concept) else {
            return None;
        };
        let (_, r) = self.relationship(concept, only)?;
        match r.roles.as_slice() {
            [role] if self.kind(&role.concept) == Some(ConceptType::ValueType) => {
                Some(role.concept.as_str())
            }
            _ => None,
        }
    }
}
