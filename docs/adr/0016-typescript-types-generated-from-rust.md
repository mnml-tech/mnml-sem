# 0016. TypeScript types, generated from the Rust types

- Status: Accepted
- Date: 2026-10-20

## Context

Web clients read and write the same three documents the crates do: semantic
models, ontologies and mappings. They want types for them.

There are two sources the types could come from. The vendored JSON Schemas
([ADR-0002](0002-mirror-the-schema.md)) are upstream's contract, but they `$ref`
each other by URL, and the Rust types are deliberately ahead of them in places
(dataset-scoped metrics, [ADR-0014](0014-dataset-scoped-metrics-ahead-of-the-schema.md)).
The Rust types are what the parser accepts, and what the crates write back.

## Decision

Generate the TypeScript from the Rust types with
[ts-rs](https://github.com/Aleph-Alpha/ts-rs), and commit the output as the
`@mnml-tech/sem` package in [`packages/sem`](../../packages/sem).

- `ossie-model` derives `ts_rs::TS` on every document type behind an optional
  `ts` feature, so no other user pays for it.
- ts-rs reads the serde attributes, so the TypeScript follows the wire format:
  a field that defaults when left out is optional, enums are their wire
  strings, and `OssieDocument` is the flat wire shape, not the Rust struct
  that nests the model.
- A drift test (`crates/ossie-model/tests/typescript.rs`) regenerates the
  package and fails when the committed files differ.
- The same test writes every Rust fixture, as the crate serializes it, as a
  TypeScript module that `satisfies` its document type. The package's
  `typecheck` then proves real documents fit the types. A file of documents
  that must be rejected, each marked `@ts-expect-error`, proves the types are
  not just permissive.
- The package carries types and the three spec version constants, and no
  runtime code. It is `private` until we decide how to publish it.

## Consequences

- Changing a document type means regenerating the package:
  `UPDATE_BINDINGS=1 cargo test -p ossie-model --features ts --test typescript`.
  Forgetting fails that test.
- A Rust type renamed only for TypeScript must not collide with another: the
  ontology's `Relationship` is `OntologyRelationship` there, as it is in the
  crate's re-exports.
- Validation, compilation and the ontology checker stay in Rust. Running them
  in a browser would mean WebAssembly bindings, which is a separate decision.
