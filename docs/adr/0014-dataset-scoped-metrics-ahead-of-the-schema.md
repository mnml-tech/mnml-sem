# 0014. Dataset-scoped metrics, ahead of the schema

- Status: Accepted
- Date: 2026-10-20

## Context

[apache/ossie#591](https://github.com/apache/ossie/pull/591) proposes letting a
dataset declare its own metrics, `datasets[].metrics`, alongside the model-root
`metrics`. It is additive, reflects a dev@ consensus, and is open but not
merged. The vendored schema
([ADR-0002](0002-mirror-the-schema.md)) does not have the property, so a
document using it fails to parse here today.

The proposal's rules:

1. A dataset-scoped metric references only the **declared fields** of its own
   dataset, by bare name: `SUM(net_amount)`. A qualified reference is invalid,
   and a metric that needs another dataset is model-scoped.
2. Within a dataset, field names and metric names share one namespace.
3. A model-scoped metric references fields as `dataset.field`.

A consumer refers to a dataset-scoped metric as `dataset.metric`. Reaching a
source column no field declares is called out as a future extension.

Waiting for the merge means a model written against the next spec does not load
on release day. Implementing early means accepting documents the vendored
schema rejects, and a rule that might still move.

## Decision

Implement the proposal now, as written, without a feature flag: it is additive,
and a document that does not use it is unaffected.

- `Dataset` gains `metrics`, the same `Metric` type as the root.
- Validation enforces the shared namespace and unique metric names per dataset,
  matched as written like every other name
  ([ADR-0003](0003-compile-expressions-from-sql-text.md)).
- A dataset-scoped metric compiles in its own resolution mode: a bare name is a
  declared field of its dataset; a qualified name, a sibling metric, and an
  undeclared name are each a compile issue with its own message.
- A query names a dataset-scoped metric `dataset.metric`, and that reference is
  the metric's output column, so it never collides with a model-scoped metric
  of the same bare name. The bare name stays the model-scoped namespace and
  never falls back to a dataset.
- A model-scoped metric named like a dataset-scoped reference (`orders.total`
  beside `orders` declaring `total`) is a validation error, since a query
  could not tell them apart.
- Fan-out ([ADR-0011](0011-refuse-a-metric-a-join-would-inflate.md)) applies
  unchanged. A dataset-scoped metric reads one dataset, which a join can still
  repeat.
- **A model-scoped metric may build on dataset-scoped ones**, written
  `dataset.metric`: `orders.total_net_amount / customers.customer_count`.
  In a model-scoped metric `dataset.name` is the field when one is declared
  and otherwise the dataset-scoped metric; the shared namespace means it is
  never both. The reference resolves by substitution, as a model-scoped
  metric reference does ([ADR-0003](0003-compile-expressions-from-sql-text.md)),
  so the result is the metric written out over fields, its datasets decide the
  joins, the fan-out check sees through it, and wrapping it in an aggregate is
  refused because it is already aggregated.

  This one is ahead of the proposal as well as the schema. Neither #591 nor
  the dev@ thread says whether it is allowed: metric references are deferred
  to expression-composition work that has no owner yet, and
  [#486](https://github.com/apache/ossie/pull/486) refuses them for this
  version. It is implemented because the direction is clear enough to bet on —
  the spelling is the one the proposal gives for referencing a dataset-scoped
  metric, every system the proposal cites as prior art (Snowflake semantic
  views, dbt MetricFlow, AtScale SML) builds cross-table metrics on per-table
  ones, and the dev@ discussion of constraints (references resolve against
  declared names only, cycles are invalid) is already what this crate does.

Where the proposal is silent and the direction is not clear, we refuse rather
than guess ([ADR-0004](0004-interpret-only-the-core-spec.md)):

- **A dataset-scoped metric referencing a sibling metric** is refused; the
  proposal's rule 1 allows only declared fields, though the thread expects it
  "eventually".

## Consequences

- A model using dataset-scoped metrics loads, compiles and queries now.
- This crate is laxer than the vendored schema for one property, which
  ADR-0002's "never stricter" test tolerates. The `dataset_metrics.yaml`
  fixture is kept out of the schema-conformance list, and a conformance test
  fails once a refresh brings the property in. That is when this record is
  checked against what merged.
- If the merged rules differ — case folding for the shared namespace is the
  most likely change, as it is still unresolved in review — this is a breaking
  change here, as any spec change is.
- Grouping a dataset-scoped metric by another dataset's dimension raises the
  same question as [ADR-0013](0013-cross-grain-metric-semantics.md). The
  proposal does not answer it, so neither does this.
- Composition is the bet most likely to need revisiting. If upstream defines a
  dataset-scoped metric as aggregated over its own dataset before any join, as
  Snowflake does, every number substitution returns is still right; the
  composed metrics the fan-out check refuses become answerable, which is the
  work ADR-0013 describes. A different spelling would be a breaking change.
