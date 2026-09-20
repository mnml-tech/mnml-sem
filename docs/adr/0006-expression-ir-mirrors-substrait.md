# 0006. The expression IR mirrors Substrait's expression types

- Status: Accepted
- Date: 2026-09-20

## Context

Between the parsed SQL and the emitters sits an intermediate representation.
Its shape is a choice.

The obvious approach is a variant per operation — `Eq`, `Add`, `Coalesce`,
`Lower`, and so on. It reads well at the use site and the compiler checks
exhaustiveness. It also grows without bound: each new function is a variant plus
an arm in every traversal, every emitter and every rewriter, and nothing about
the enum tells you which of those you forgot.

## Decision

Six variants, one per Substrait expression type:

| IR        | Substrait           |
|-----------|---------------------|
| `Column`  | `Selection`         |
| `Literal` | `Literal`           |
| `Call`    | `ScalarFunction`    |
| `Agg`     | `AggregateFunction` |
| `Case`    | `IfThen`            |
| `InList`  | `SingularOrList`    |

Functions are not variants; they are rows in the catalog
([ADR-0005](0005-one-catalog-describes-every-function.md)), named by a `Func`
inside `Call` and `Agg`.

`Case` and `InList` stay structural rather than becoming calls because Substrait
models them as dedicated expression types too. The IR follows the target rather
than inventing a shape and translating.

## Consequences

- Substrait emission is translation rather than decision: each variant has
  exactly one destination.
- Traversals — type inference, dataset collection, aggregate extraction,
  reference rewriting — are written once over six cases instead of once per
  operation.
- The IR is less self-describing at a glance: `Call { func: Func::Divide, .. }`
  reads worse than `Div(a, b)`.
- It ties the IR's shape to Substrait's. A second structured target that
  disagreed would make this a poor fit.

