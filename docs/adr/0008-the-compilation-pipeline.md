# 0008. The compilation pipeline

- Status: Accepted
- Date: 2026-09-20

## Context

Getting from a document to a plan takes several distinct steps: parse the YAML
or JSON, check it, turn its expression text into structure
([ADR-0003](0003-compile-expressions-from-sql-text.md)), resolve a query against
it, build a plan, emit. Two questions follow.

**How much of that does the caller see?** A single `document + query -> SQL`
entry point is the smallest API, and it decides for the caller that no
intermediate result is worth keeping. Since compiling a model's expressions is
the expensive part and models change far less often than queries are asked,
that decision costs the caller the only optimisation that matters, with no way
to opt out.

**When are expressions compiled?** Eagerly, so the whole model is structure
before any query runs; or lazily, resolving what each query happens to touch.
Lazily is what a query engine does, because its schema is external and mutable.
A semantic model is not that: it is an authored artifact, reviewed and
published, and the same for every query asked of it.

## Decision

Expose each phase, and the value it produces, as part of the API: `parse`,
`compile`, `bind`, `plan`, `emit`. The caller composes them and decides how long
each intermediate lives — whether a compiled model is held for the life of the
process, rebuilt when the file changes, kept per tenant, or dropped after one
query. This crate has no opinion and holds no state.

**A model compiles as a unit.** `CompiledModel::compile` lowers every field and
metric expression, not only those some query needs, and it is the one entry
point that performs every check this crate makes — structural problems are
fatal, because they leave the model ambiguous, while an expression that will not
compile is recorded against that field or metric and excluded, leaving the rest
of the model usable.

The reason compilation is whole-model is the error report, not speed. Reporting
every problem in a model at once only means something if every expression was
attempted; compile lazily and a caller learns about their broken metrics one
query at a time, from whichever user happens to reach one first.

## Consequences

- Every expression problem in a model is reported together, at a moment the
  caller chooses, rather than scattered across future queries.
- Queries parse nothing. Binding and planning read already-resolved data, which
  is also what makes a metric's referenced datasets known without re-parsing —
  the input to join discovery and to
  [ADR-0011](0011-refuse-a-metric-a-join-would-inflate.md).
- Errors surface at the earliest point they are knowable, which is not the same
  for all of them. A malformed expression is a property of the model. Whether a
  column is in scope, or whether a join inflates a metric, depends on the query
  and cannot be known before one exists.
- A caller pays to compile metrics their query never touches. Harmless when the
  compiled model is reused, real when it cannot be — a large model in an
  environment that recompiles per request.
- The type system carries the guarantee: a query can only be bound against a
  compiled model, so the checks cannot be skipped by accident.
- The API is wider than one call, and a caller who wants the simple thing has to
  compose four of them.
