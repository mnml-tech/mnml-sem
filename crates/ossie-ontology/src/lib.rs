//! Check an Apache Ossie mapping against its ontology and its semantic model.
//!
//! A mapping document says which fields of a semantic model populate which
//! concepts and relationships of an ontology. Each of the three documents can
//! be valid on its own while the mapping still names a concept the ontology
//! does not have, reads a field the model does not declare, or claims a
//! many-to-one relationship the data cannot promise. [`check`] finds those.
//!
//! It checks; it does not evaluate. No `derived_by` or `requires` rule is
//! interpreted and no query is answered, because the spec defines neither
//! (ADR-0015). Where the spec is silent, nothing is guessed.
//!
//! ```no_run
//! # fn main() -> Result<(), ossie_model::Error> {
//! use ossie_model::parser;
//!
//! let ontology = parser::ontology_from_file("steelwheels.ontology.yaml")?;
//! let mapping = parser::mapping_from_file("steelwheels.mapping.yaml")?;
//! let model = parser::from_file("steelwheels.yaml")?;
//!
//! let report = ossie_ontology::check(&mapping, &ontology, &model.model);
//! for issue in &report.issues {
//!     println!("{issue}");
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod check;
mod concepts;

use std::fmt;

use ossie_model::{MappingDocument, OntologyDocument, SemanticModel};

/// Check `mapping` against the ontology and semantic model it references.
///
/// Runs the structural checks on the ontology and the mapping first, and
/// compiles the model, so one call reports everything.
pub fn check(
    mapping: &MappingDocument,
    ontology: &OntologyDocument,
    model: &SemanticModel,
) -> Report {
    check::Checker::new(ontology, model).run(mapping)
}

/// Everything [`check`] found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub issues: Vec<Issue>,
}

impl Report {
    /// Whether nothing is wrong. Warnings do not count.
    pub fn is_ok(&self) -> bool {
        self.errors().next().is_none()
    }

    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Issue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
    }
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Where in the mapping, as a path: `concept_mappings[3](Customer).link_mappings[0]`.
    pub at: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}: {}", self.severity, self.at, self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The mapping is wrong: it names something that does not exist, or
    /// something of the wrong kind.
    Error,
    /// The mapping may be right, but nothing in the documents confirms it.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        })
    }
}
