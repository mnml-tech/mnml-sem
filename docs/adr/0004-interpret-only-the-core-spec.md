# 0004. Interpret only Ossie specifications

- Status: Accepted
- Date: 2026-09-20, amended 2026-10-20 by [ADR-0015](0015-a-workspace-of-crates.md)

## Context

Ossie provides `custom_extensions` at every level — model, dataset, field,
metric — each a `vendor_name` and an opaque JSON `data` string. The vendor name
is free-form, so anyone may write anything there.

That is an inviting place to put what the core spec cannot express: where a
dataset's files physically live, engine hints, join cardinality, grain. A
compiler that read its own extensions could do more than one that does not.

It could also stop being a compiler for Ossie and become a compiler for Ossie
plus a private dialect, where the interesting behaviour is only reachable
through metadata no other tool understands.

Ossie also specifies more than the core: an ontology, and mapping documents
that tie one to a semantic model. Those are not vendor payloads. Every tool
that implements them reads the same thing.

## Decision

Interpret what the Ossie specifications define — the core spec, the ontology
and mapping documents — and nothing else. Carry `custom_extensions` through
parsing and serialization unchanged, and interpret none of them. No behaviour
in these crates depends on any vendor payload.

## Consequences

- A document is compiled on the strength of its specified content alone. Two
  tools reading the same model see the same thing.
- `Dataset.source` is the only physical locator, and resolving it — to a table,
  a view, a file — belongs to the engine executing the plan. This crate never
  opens data.
- A payload that is not valid JSON, or that means nothing to us, is not an
  error. It is not ours to judge.
- Where the specs genuinely cannot express something we need, the answer is
  to raise it upstream, not to route around it here. Following a proposal
  before it lands ([ADR-0014](0014-dataset-scoped-metrics-ahead-of-the-schema.md))
  is raising it upstream; inventing a payload is not.

