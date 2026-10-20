//! The checks themselves, walking a mapping's concept mappings top-down.

use std::collections::BTreeSet;

use ossie_compiler::QueryError;
use ossie_compiler::compiled::CompiledModel;
use ossie_compiler::plan::Expr;
use ossie_compiler::plan::ossie_sql::lower_mapping_expression;
use ossie_model::validate::{validate_mapping, validate_ontology};
use ossie_model::{
    ConceptMapping, ConceptType, Dialect, Error, LinkMapping, MappingDocument, Multiplicity,
    ObjectMapping, OntologyDocument, OntologyRelationship, ReferentMapping, SemanticModel,
};

use crate::concepts::Concepts;
use crate::{Issue, Report, Severity};

/// The fields a mapping reads, as `(dataset, field)`.
type Fields = BTreeSet<(String, String)>;

pub(crate) struct Checker<'a> {
    ontology: &'a OntologyDocument,
    model: &'a SemanticModel,
    concepts: Concepts<'a>,
    /// `None` when the model is structurally broken and does not compile.
    compiled: Option<CompiledModel>,
    issues: Vec<Issue>,
}

/// What an object mapping maps to, and what it reads to do so.
struct Mapped {
    concept: String,
    fields: Fields,
}

impl<'a> Checker<'a> {
    pub(crate) fn new(ontology: &'a OntologyDocument, model: &'a SemanticModel) -> Self {
        Self {
            ontology,
            model,
            concepts: Concepts::new(ontology),
            compiled: None,
            issues: Vec::new(),
        }
    }

    pub(crate) fn run(mut self, mapping: &MappingDocument) -> Report {
        self.structural("ontology", validate_ontology(self.ontology));
        self.structural("mapping", validate_mapping(mapping));
        match CompiledModel::compile_model(self.model) {
            Ok(compiled) => self.compiled = Some(compiled),
            Err(e) => self.error("semantic model", format!("does not compile: {e}")),
        }

        if mapping.ontology_ref.name != self.ontology.name {
            self.error(
                "ontology_ref",
                format!(
                    "names ontology {:?}, but the ontology checked against is {:?}",
                    mapping.ontology_ref.name, self.ontology.name
                ),
            );
        }
        if mapping.semantic_model_ref.name != self.model.name {
            self.error(
                "semantic_model_ref",
                format!(
                    "names semantic model {:?}, but the model checked against is {:?}",
                    mapping.semantic_model_ref.name, self.model.name
                ),
            );
        }

        for (i, cm) in mapping.concept_mappings.iter().enumerate() {
            self.concept_mapping(&format!("concept_mappings[{i}]({})", cm.concept), cm);
        }
        Report {
            issues: self.issues,
        }
    }

    fn structural(&mut self, at: &str, result: Result<(), Error>) {
        match result {
            Ok(()) => {}
            Err(Error::Validation(issues)) => {
                for message in issues {
                    self.error(at, message);
                }
            }
            Err(e) => self.error(at, e.to_string()),
        }
    }

    fn concept_mapping(&mut self, at: &str, cm: &ConceptMapping) {
        let Some(kind) = self.concepts.kind(&cm.concept) else {
            self.error(
                at,
                format!("{:?} is not a concept of the ontology", cm.concept),
            );
            return;
        };
        if kind == ConceptType::ValueType && !cm.object_mappings.is_empty() {
            self.error(
                at,
                format!(
                    "{:?} is a value type; object_mappings populate entity types only",
                    cm.concept
                ),
            );
        }
        for (i, om) in cm.object_mappings.iter().enumerate() {
            self.object_mapping(&format!("{at}.object_mappings[{i}]"), om, Some(&cm.concept));
        }
        for (i, lm) in cm.link_mappings.iter().enumerate() {
            self.link_mapping(
                &format!("{at}.link_mappings[{i}]"),
                &cm.concept,
                lm,
                &mut Vec::new(),
            );
        }
    }

    /// A node at depth `path.len() + 1`, under ancestors that mapped `path`.
    fn link_mapping(&mut self, at: &str, owner: &str, lm: &LinkMapping, path: &mut Vec<Mapped>) {
        let depth = path.len() + 1;

        // The concept expected here is the role this node fills.
        let mut expected: Option<String> = (depth == 1).then(|| owner.to_string());
        let mut relationship: Option<(String, &'a OntologyRelationship, Vec<&'a str>)> = None;
        if let Some(name) = &lm.relationship {
            match self.concepts.relationship(owner, name) {
                None => self.error(at, format!("{owner:?} has no relationship {name:?}")),
                Some((declared_on, r)) if r.arity() != depth => self.error(
                    at,
                    format!(
                        "{declared_on}.{} has {} roles, but this link mapping is at depth {depth}; \
                         a link mapping at depth n names a relationship of n roles",
                        r.name,
                        r.arity()
                    ),
                ),
                Some((declared_on, r)) => {
                    let players = role_players(declared_on, r);
                    for (i, ancestor) in path.iter().enumerate() {
                        if !self.concepts.is_a(&ancestor.concept, players[i]) {
                            self.error(
                                at,
                                format!(
                                    "role {} of {declared_on}.{} is played by {:?}, but the link \
                                     mapping above maps {:?}",
                                    i + 1,
                                    r.name,
                                    players[i],
                                    ancestor.concept
                                ),
                            );
                        }
                    }
                    expected = Some(players[depth - 1].to_string());
                    relationship = Some((format!("{declared_on}.{}", r.name), r, players));
                }
            }
        }

        let Some(mapped) = self.object_mapping(
            &format!("{at}.object_mapping"),
            &lm.object_mapping,
            expected.as_deref(),
        ) else {
            return;
        };

        if let Some((id, r, players)) = relationship
            && let Some(multiplicity) = r.multiplicity
        {
            // The roles before the last decide the last; a OneToOne also
            // decides the first from the second.
            let determinant: Fields = path.iter().flat_map(|m| m.fields.clone()).collect();
            let last = players[players.len() - 1];
            self.keyed_by(at, &id, multiplicity, last, &determinant);
            if multiplicity == Multiplicity::OneToOne {
                self.keyed_by(at, &id, multiplicity, players[0], &mapped.fields);
            }
        }

        path.push(mapped);
        for (i, child) in lm.children.iter().enumerate() {
            self.link_mapping(&format!("{at}.children[{i}]"), owner, child, path);
        }
        path.pop();
    }

    /// Check one object mapping. `expected` is the concept its position calls
    /// for, when the position says.
    fn object_mapping(
        &mut self,
        at: &str,
        om: &ObjectMapping,
        expected: Option<&str>,
    ) -> Option<Mapped> {
        let concept = match (om.concept.as_deref(), expected) {
            (Some(c), _) if !self.concepts.exists(c) => {
                self.error(at, format!("{c:?} is not a concept of the ontology"));
                return None;
            }
            (Some(c), Some(e)) if !self.concepts.is_a(c, e) => {
                self.error(
                    at,
                    format!("maps {c:?} where {e:?} is expected, and {c:?} is not a kind of {e:?}"),
                );
                return None;
            }
            (Some(c), _) => c.to_string(),
            (None, Some(e)) => e.to_string(),
            (None, None) => {
                self.error(
                    at,
                    "nothing says which concept this maps to; name it with `concept`".into(),
                );
                return None;
            }
        };
        let kind = self.concepts.kind(&concept);

        let mut fields = Fields::new();
        if let Some(sql) = &om.expression {
            self.expression(at, sql, &mut fields);
            if kind == Some(ConceptType::EntityType)
                && self.concepts.simple_identifier(&concept).is_none()
            {
                self.error(
                    at,
                    format!(
                        "entity type {concept:?} has no simple identifier, so an expression \
                         cannot find its objects; use referent_mappings"
                    ),
                );
            }
        }
        if !om.referent_mappings.is_empty() {
            if kind == Some(ConceptType::EntityType) {
                self.referents(at, &concept, &om.referent_mappings, &mut fields);
            } else {
                self.error(
                    at,
                    format!(
                        "{concept:?} is a value type; referent_mappings find entity objects only"
                    ),
                );
            }
        }
        Some(Mapped { concept, fields })
    }

    /// Referent mappings of entity type `concept`: each names one of its
    /// identifying relationships, and together they name all of them.
    fn referents(
        &mut self,
        at: &str,
        concept: &str,
        referents: &[ReferentMapping],
        fields: &mut Fields,
    ) {
        let identifiers = self.concepts.identifiers(concept);
        if identifiers.is_empty() {
            self.error(
                at,
                format!(
                    "{concept:?} declares no identify_by, so referent_mappings cannot find its \
                     objects"
                ),
            );
            return;
        }

        let mut seen = BTreeSet::new();
        for (i, rm) in referents.iter().enumerate() {
            let at = format!("{at}.referent_mappings[{i}]");
            let Some((declared_on, r)) = self.concepts.relationship(concept, &rm.relationship)
            else {
                self.error(
                    &at,
                    format!("{concept:?} has no relationship {:?}", rm.relationship),
                );
                continue;
            };
            if !identifiers.contains(&r.name) {
                self.error(
                    &at,
                    format!(
                        "{declared_on}.{} is not one of {concept:?}'s identifying relationships \
                         ({})",
                        r.name,
                        identifiers.join(", ")
                    ),
                );
                continue;
            }
            if !seen.insert(r.name.as_str()) {
                self.error(&at, format!("maps {declared_on}.{} twice", r.name));
            }
            let [role] = r.roles.as_slice() else {
                // Not binary; the ontology's own checks report it.
                continue;
            };
            let player = role.concept.as_str();
            let player_kind = self.concepts.kind(player);

            if let Some(sql) = &rm.expression {
                self.expression(&at, sql, fields);
                if player_kind == Some(ConceptType::EntityType)
                    && self.concepts.simple_identifier(player).is_none()
                {
                    self.error(
                        &at,
                        format!(
                            "{declared_on}.{} is played by entity type {player:?}, which has no \
                             simple identifier; use nested referent_mappings",
                            r.name
                        ),
                    );
                }
            }
            if !rm.referent_mappings.is_empty() {
                if player_kind == Some(ConceptType::EntityType) {
                    self.referents(&at, player, &rm.referent_mappings, fields);
                } else {
                    self.error(
                        &at,
                        format!(
                            "{declared_on}.{} is played by value type {player:?}; nested \
                             referent_mappings find entity objects only",
                            r.name
                        ),
                    );
                }
            }
        }

        let missing: Vec<&str> = identifiers
            .iter()
            .map(String::as_str)
            .filter(|id| !seen.contains(id))
            .collect();
        if !missing.is_empty() {
            self.error(
                at,
                format!(
                    "does not map {}, part of {concept:?}'s identifier",
                    missing.join(", ")
                ),
            );
        }
    }

    /// An expression must be one the compiler can read over declared fields.
    fn expression(&mut self, at: &str, sql: &str, fields: &mut Fields) {
        let expr = match lower_mapping_expression(sql, Dialect::AnsiSql, self.model) {
            Ok(expr) => expr,
            // A parse error already quotes the expression.
            Err(e @ QueryError::InvalidExpression(_)) => {
                self.error(at, e.to_string());
                return;
            }
            Err(e) => {
                self.error(at, format!("expression {sql:?}: {e}"));
                return;
            }
        };
        let mut read = Fields::new();
        columns(&expr, &mut read);
        for (dataset, field) in &read {
            if let Some(issue) = self.compile_issue(dataset, field) {
                self.error(
                    at,
                    format!("reads {dataset}.{field}, which does not compile: {issue}"),
                );
            }
        }
        fields.extend(read);
    }

    fn compile_issue(&self, dataset: &str, field: &str) -> Option<String> {
        let compiled = self.compiled.as_ref()?;
        compiled
            .dataset(dataset)
            .and_then(|d| d.field(field))
            .is_none()
            .then(|| {
                compiled
                    .issues
                    .iter()
                    .find(|i| {
                        matches!(&i.target, ossie_compiler::compiled::IssueTarget::Field {
                            dataset: d, field: f } if d == dataset && f == field)
                    })
                    .map_or_else(|| "unknown reason".to_string(), |i| i.message.clone())
            })
    }

    /// A multiplicity promises that the fields in `determinant` decide one
    /// `decided` object. The dataset guarantees that only when they cover one
    /// of its keys.
    fn keyed_by(
        &mut self,
        at: &str,
        relationship: &str,
        multiplicity: Multiplicity,
        decided: &str,
        determinant: &Fields,
    ) {
        let datasets: BTreeSet<&str> = determinant.iter().map(|(d, _)| d.as_str()).collect();
        let dataset = match datasets.into_iter().collect::<Vec<_>>().as_slice() {
            [] => return,
            [one] => *one,
            many => {
                self.warning(
                    at,
                    format!(
                        "{relationship} is {multiplicity:?}, but the roles deciding {decided:?} \
                         read {}; a key across datasets cannot be checked",
                        many.join(", ")
                    ),
                );
                return;
            }
        };
        let Some(ds) = self.model.datasets.iter().find(|d| d.name == dataset) else {
            return;
        };

        let keys: Vec<&Vec<String>> = std::iter::once(&ds.primary_key)
            .chain(&ds.unique_keys)
            .filter(|k| !k.is_empty())
            .collect();
        let names: Vec<&str> = determinant.iter().map(|(_, f)| f.as_str()).collect();
        if keys.is_empty() {
            self.warning(
                at,
                format!(
                    "{relationship} is {multiplicity:?}, but {dataset} declares no primary or \
                     unique key, so nothing confirms one {decided:?} per {}",
                    names.join(", ")
                ),
            );
            return;
        }

        // A key names fields; a field reading a column by the same name counts.
        let covered = |key: &&Vec<String>| {
            key.iter().all(|k| {
                determinant.iter().any(|(_, f)| {
                    f == k
                        || self
                            .compiled
                            .as_ref()
                            .and_then(|c| c.dataset(dataset)?.field(f)?.physical_column.clone())
                            .is_some_and(|col| &col == k)
                })
            })
        };
        if !keys.iter().any(covered) {
            self.warning(
                at,
                format!(
                    "{relationship} is {multiplicity:?}, but {} is not a key of {dataset}, so \
                     the dataset does not guarantee one {decided:?} per {}",
                    names.join(", "),
                    names.join(", ")
                ),
            );
        }
    }

    fn error(&mut self, at: &str, message: String) {
        self.push(Severity::Error, at, message);
    }

    fn warning(&mut self, at: &str, message: String) {
        self.push(Severity::Warning, at, message);
    }

    fn push(&mut self, severity: Severity, at: &str, message: String) {
        self.issues.push(Issue {
            severity,
            at: at.to_string(),
            message,
        });
    }
}

/// The concepts playing each role, the declaring concept first.
fn role_players<'r>(declared_on: &'r str, r: &'r OntologyRelationship) -> Vec<&'r str> {
    std::iter::once(declared_on)
        .chain(r.roles.iter().map(|role| role.concept.as_str()))
        .collect()
}

/// The `(dataset, field)` references in a lowered mapping expression.
fn columns(expr: &Expr, out: &mut Fields) {
    if let Expr::Column(c) = expr {
        out.insert((c.dataset.clone(), c.column.clone()));
    }
    for child in expr.children() {
        columns(child, out);
    }
}
