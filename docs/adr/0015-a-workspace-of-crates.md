# 0015. A workspace of crates, covering the ontology

- Status: Accepted
- Date: 2026-10-20

## Context

This is one crate, `ossie-substrait`, and everything is in it: the document
types, parsing and validation, expression lowering, the compiled model, the
query contract, binding, planning, and SQL and Substrait emission. Substrait
sits behind a default feature, `substrait`.

Three things make that a poor fit.

**Most users of the document types need none of the rest.** A converter, a
linter or an editor integration reads, checks and writes Ossie documents. To
get the types today it takes on `sqlparser`, and unless it turns off a default
feature, `substrait` and `prost` as well. The model layer does not need any of
them: `src/model` imports nothing from the rest of the crate except doc links.

**The Substrait boundary is a feature flag, and it is not holding.**
`cargo test --no-default-features` does not build, because a test calls
`to_plan` without the `cfg` that guards it. A feature boundary is checked only
in the configurations someone remembers to build. A crate boundary is checked
every time.

**Ossie is now more than the core spec.** Next to the semantic model, the
project specifies an ontology (`ontology/ontology.json`): concepts,
relationships, multiplicities, rules. It also specifies a mapping document
(`ontology/mapping.json`) that names one ontology and one semantic model, and
says which dataset fields populate which concepts and relationships. Since
apache/ossie#458 these are three separate documents. A mapping's expressions
(`customer_w_ter.customernumber`) are `dataset.field` references into the
semantic model: the same resolution a model-scoped metric performs, without
the aggregate. Checking a mapping therefore needs the compiled model.

[ADR-0004](0004-interpret-only-the-core-spec.md) limits this crate to the core
spec. It was written against vendor `custom_extensions`, so that nothing here
depends on metadata other tools cannot read. The ontology is not a vendor
extension; it is an Ossie specification. Covering it does not create the
private dialect ADR-0004 guards against, but it does change what the project
is for, and the name says Substrait.

The ontology spec is early. It is at `0.2.0.dev0`. Upstream says parser and
converter support for mapping documents "is not yet available". The rule
language for `derived_by` and `requires` is undecided, and nothing defines how
an ontology is queried.

## Decision

**Make the repository a Cargo workspace of four crates**, each depending only
on those above it:

| Crate | Holds | Depends on |
|---|---|---|
| `ossie-model` | Types for the semantic model, ontology and mapping documents; parsing; structural validation; conformance against all three vendored schemas | `serde`, `serde_json`, `serde_yaml` |
| `ossie-compiler` | Expression lowering, function catalog, `CompiledModel`, `SemanticQuery`, binding, planning, `LogicalPlan`, SQL emission | `ossie-model`, `sqlparser`, `serde` (the query contract deserializes) |
| `ossie-substrait` | `LogicalPlan` → Substrait | `ossie-compiler`, `substrait`, `prost` |
| `ossie-ontology` | Checking a mapping against its ontology and a `CompiledModel` | `ossie-model`, `ossie-compiler` |

- **The ontology and mapping types live in `ossie-model`**, next to the
  core-spec types, and follow [ADR-0002](0002-mirror-the-schema.md): one type
  per `$def`, `deny_unknown_fields`, a vendored schema, conformance tests that
  fail on upstream drift.
- **SQL emission stays in the compiler.** It is small, shares the catalog
  ([ADR-0005](0005-one-catalog-describes-every-function.md)) with lowering, and
  nothing needs it without the planner. The catalog keeps Substrait's
  coordinates as plain strings, so it needs no Substrait dependency.
- **The `substrait` feature goes away.** Depending on `ossie-substrait` is how
  a caller opts in.
- **`QueryError` stays one type, in `ossie-compiler`.** Lowering, binding and
  planning all return it. Splitting it would only matter if lowering became a
  crate of its own, which nothing here needs. `ossie_model::Error` therefore
  loses its `Query` variant, which nothing constructed.
- **Fixtures live with the conformance tests, in `ossie-model`.** They are
  Ossie documents first, and conformance checks they are legal ones. Every
  crate's tests read them from there.
- **The end-to-end tests live in `ossie-substrait`.** They take a document
  through planning to both SQL and Substrait, and only the top crate can see
  both. They moved whole rather than being split by layer, so the move adds and
  drops no assertion; sorting SQL-only tests down into `ossie-compiler` is
  later work.
- **The ontology crate checks; it does not evaluate.** It checks that every
  mapping expression names a declared field. It checks that the concepts and
  relationships named exist in the ontology, and that a relationship's
  mappings have the right arity. It checks that an entity's identifying fields
  match the mapped dataset's keys. It interprets no `derived_by` or `requires`
  rule and answers no query, because the spec defines neither. Where those are
  silent we refuse rather than guess, as in ADR-0004.
- **ADR-0004 is amended on acceptance.** Its rule becomes "interpret only Ossie
  specifications": the core spec, the ontology and mappings, and no vendor
  payload.
- **The restructure is mechanical, and it lands on its own.** It is one change
  made of `git mv`, path rewrites and manifest changes, with no behavior
  change, so history follows the files and the review stays readable. The same
  tests pass before and after. `ossie-ontology` and the ontology types
  followed separately, the crate created with its first code: checking the
  Steel Wheels mapping.

**The crate names are the ones above, for now.** The repository is renamed
`mnml-sem`, from `osi-substrait`, which carried the pre-rename OSI and
described one crate rather than the project. The ASF's trademark policy does
not allow a project's name in a third party's product branding, so the
`ossie-` prefix is settled before anything is published: either upstream
approves it, or the crates take the repository's prefix. Nothing is published,
so renaming is free now and costly later.

## Consequences

- A caller that only reads and writes documents depends on `ossie-model` and
  nothing heavier. One that wants SQL takes the compiler. Substrait is opt-in
  by dependency, not by feature.
- Layering is enforced by the compiler. A model type cannot reach into the
  planner, and a doc link across crates becomes a link to another crate's docs.
- The ontology gets the same discipline as the core spec from its first
  commit: mirrored types and a vendored schema that flags upstream changes.
- Four crates to version and, eventually, publish, in dependency order. Before
  release one shared version is enough. After it, a change to `ossie-model`
  ripples to every crate.
- Each crate's public API is now a contract with the others. Items that are
  `pub` today only for tests or neighboring modules need sorting into public
  API and `pub(crate)`.
- Ontology support is limited to what the spec makes checkable. Evaluating
  rules, or compiling queries over an ontology, waits for the spec to define
  them.
- The open cardinality and grain questions ([ADR-0011](0011-refuse-a-metric-a-join-would-inflate.md),
  [ADR-0013](0013-cross-grain-metric-semantics.md)) gain a possible source:
  ontology multiplicities and entity identity carry exactly what the core spec
  lacks. Nothing here depends on that. It is a reason the two belong in one
  workspace, not a decision.
