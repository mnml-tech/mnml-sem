//! Apache Ossie documents — types, parsing and structural validation.
//!
//! Three kinds of document, each mirroring its [Apache Ossie](https://github.com/apache/ossie)
//! schema (ADR-0002):
//!
//! - a semantic model ([`OssieDocument`], `core-spec/ossie-schema.json`, at
//!   [`OSSIE_SPEC_VERSION`]);
//! - an ontology ([`OntologyDocument`], `ontology/ontology.json`);
//! - a mapping of one onto the other ([`MappingDocument`],
//!   `ontology/mapping.json`).
//!
//! Ossie was previously named Open Semantic Interchange (OSI).
//!
//! Nothing here compiles an expression: that needs the model as context and
//! lives in `ossie-compiler`.

#![forbid(unsafe_code)]

mod ai_context;
mod dataset;
mod document;
mod enums;
mod expression;
mod extensions;
mod field;
mod mapping;
mod metric;
mod ontology;
mod relationship;
mod semantic_model;

pub mod error;
pub mod parser;
pub mod validate;

pub use ai_context::{AiContext, AiContextStructured};
pub use dataset::Dataset;
pub use document::{OSSIE_SPEC_VERSION, OssieDocument};
pub use enums::{DataType, Dialect};
pub use expression::{DialectExpression, Expression};
pub use extensions::CustomExtension;
pub use field::{Field, FieldDimension};
pub use mapping::{DocumentReference, MAPPING_SPEC_VERSION, MappingDocument};
pub use metric::Metric;
pub use ontology::{
    ANY, BUILT_IN_CONCEPTS, BUILT_IN_VALUE_TYPES, ConceptMapping, ConceptType, LinkMapping,
    Multiplicity, ONTOLOGY_SPEC_VERSION, ObjectMapping, OntologyComponent, OntologyDocument,
    OntologyMap, ReferentMapping, Relationship as OntologyRelationship, Role,
};
pub use relationship::Relationship;
pub use semantic_model::SemanticModel;

pub use error::Error;
