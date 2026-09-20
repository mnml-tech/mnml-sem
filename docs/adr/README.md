# Architecture decisions

Why this crate is built the way it is — see
[ADR-0001](0001-record-architecture-decisions.md) for what belongs here.

These describe the design as it stands. While it is still moving they are
amended in place rather than superseded, so there is one set of records and it
is current.

| # | Decision | Status |
|---|----------|--------|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-mirror-the-schema.md) | Mirror the schema, and check against a vendored copy | Accepted |
| [0003](0003-compile-expressions-from-sql-text.md) | Compile expressions from their SQL text | Accepted |
| [0004](0004-interpret-only-the-core-spec.md) | Interpret only the core spec | Accepted |
| [0005](0005-one-catalog-describes-every-function.md) | One catalog describes every function | Accepted |
| [0006](0006-expression-ir-mirrors-substrait.md) | The expression IR mirrors Substrait's expression types | Accepted |
| [0007](0007-unknown-types-stay-unknown.md) | Unknown types stay unknown | Accepted |
| [0008](0008-the-compilation-pipeline.md) | The compilation pipeline | Accepted |
| [0009](0009-plan-models-the-constrained-target.md) | The plan models the more constrained target | Accepted |
| [0010](0010-the-query-contract-is-ours.md) | The query contract is ours | Accepted |
| [0011](0011-refuse-a-metric-a-join-would-inflate.md) | Refuse a metric a join would inflate | Accepted |
| [0012](0012-sql-emission-accumulates-clauses.md) | SQL emission accumulates clauses | Accepted |
| [0013](0013-cross-grain-metric-semantics.md) | Cross-grain metric semantics | **Proposed** |

