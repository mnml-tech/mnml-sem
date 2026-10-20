//! Ontology documents (`ontology/ontology.json`) and the concept mappings they
//! share with mapping documents.
//!
//! An ontology describes the enterprise in concepts and relationships, grouping
//! each relationship under the concept that plays its first role. These types
//! mirror the schema's `$defs` one for one (ADR-0002).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ai_context::AiContext;
use crate::document::OssieDocument;

/// The `version` value ontology and mapping documents carry (`ontology.json`
/// and `mapping.json` root const).
pub const ONTOLOGY_SPEC_VERSION: &str = "0.2.0.dev0";

/// Concepts every ontology includes without declaring them.
pub const BUILT_IN_CONCEPTS: [&str; 8] = [
    "Any", "Boolean", "Date", "DateTime", "Decimal", "Float", "Integer", "String",
];

/// The built-in value types, every one of which a value type must extend.
pub const BUILT_IN_VALUE_TYPES: [&str; 7] = [
    "Boolean", "Date", "DateTime", "Decimal", "Float", "Integer", "String",
];

/// The built-in entity type every entity type implicitly extends.
pub const ANY: &str = "Any";

/// Root of an ontology document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct OntologyDocument {
    pub version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_context: Option<AiContext>,
    /// Constraints on the population of the whole ontology.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// One component per concept.
    pub ontology: Vec<OntologyComponent>,
    /// Deprecated embedded mappings; write a [`MappingDocument`] instead.
    ///
    /// [`MappingDocument`]: crate::MappingDocument
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ontology_mappings: Vec<OntologyMap>,
    /// Namespace prefixes that QName IRIs expand against.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prefixes: BTreeMap<String, String>,
}

impl OntologyDocument {
    /// The component declaring `concept`, if this ontology declares it.
    pub fn concept(&self, concept: &str) -> Option<&OntologyComponent> {
        self.ontology.iter().find(|c| c.concept == concept)
    }
}

/// One concept, with the relationships in which it plays the first role
/// (`$defs.OntologyComponent`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct OntologyComponent {
    pub concept: String,
    #[serde(rename = "type")]
    pub concept_type: ConceptType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Supertypes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_by: Vec<String>,
    /// Names of this concept's relationships that form its preferred
    /// identifier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identify_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}

impl OntologyComponent {
    /// The relationship declared under this concept as `name`.
    pub fn relationship(&self, name: &str) -> Option<&Relationship> {
        self.relationships.iter().find(|r| r.name == name)
    }
}

/// Entity type or value type (`$defs.ConceptType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ConceptType {
    /// A real-world concept, referenced through other information.
    EntityType,
    /// A datatype with additional semantics.
    ValueType,
}

/// A relationship among concepts (`$defs.Relationship`).
///
/// Its first role is played by the concept it is declared under; `roles` lists
/// the rest, in order. It is identified as `Concept.name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
// Renamed so it does not collide with the semantic model's `Relationship`.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "OntologyRelationship"))]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The roles after the first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplicity: Option<Multiplicity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    pub verbalizes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}

impl Relationship {
    /// Number of roles, counting the implicit first one.
    pub fn arity(&self) -> usize {
        self.roles.len() + 1
    }
}

/// A role after the first (`$defs.Role`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub concept: String,
    /// Distinguishes the role when its concept plays more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Role {
    /// How expressions refer to this role: its name, defaulting to its concept.
    pub fn role_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.concept)
    }
}

/// Relationship multiplicity (`$defs.Multiplicity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Multiplicity {
    /// The last role is determined by the others.
    ManyToOne,
    /// Many-to-one in both directions; binary relationships only.
    OneToOne,
}

/// How one concept, and the relationships grouped under it, are populated
/// (`$defs.ConceptMapping`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ConceptMapping {
    pub concept: String,
    /// Populate the concept. Valid only for an entity type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object_mappings: Vec<ObjectMapping>,
    /// Populate the relationships grouped under the concept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_mappings: Vec<LinkMapping>,
}

/// Expressions that find objects of a concept (`$defs.ObjectMapping`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ObjectMapping {
    /// The concept mapped to; otherwise implied by where the mapping sits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    /// Find entity objects through their identifying relationships.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referent_mappings: Vec<ReferentMapping>,
    /// Compute a value, or a simply identified entity's identifying value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

/// One identifying relationship of a referenced entity
/// (`$defs.ReferentMapping`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ReferentMapping {
    pub relationship: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// For an identifying relationship whose other role is an entity type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referent_mappings: Vec<ReferentMapping>,
}

/// A node in a tree of link mappings (`$defs.LinkMapping`).
///
/// A node at depth `n` maps tuples of `n` objects: its ancestors' objects, then
/// its own. When it names a relationship, that relationship has `n` roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LinkMapping {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    /// The object in the last position of this node's tuples.
    pub object_mapping: ObjectMapping,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<LinkMapping>,
}

/// A mapping embedded in an ontology (`$defs.OntologyMap`). Deprecated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct OntologyMap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub semantic_model: OssieDocument,
    pub concept_mappings: Vec<ConceptMapping>,
}
