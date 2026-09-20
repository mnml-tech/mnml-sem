# 0011. Refuse a metric a join would inflate

- Status: Accepted
- Date: 2026-09-20

## Context

A metric may read columns from more than one dataset, which means the compiler
joins them. Joining repeats rows: on a one-to-many relationship, each row of the
one side appears once per matching row of the many side.

A `SUM` over the repeated side then counts each value once per match. The result
is not slightly off, it is multiplied by an arbitrary factor, and it looks like
a perfectly ordinary number.

The spec tells us this is happening. A relationship's `from` is defined as the
dataset on the **many** side and `to` as the one side, and `to_columns` names the
target's primary or unique key. Cardinality is declared, not guessed.

Three options:

1. **Emit it.** The compiler returns a wrong number with no indication.
2. **Compute the correct plan** by aggregating the repeated side before joining.
   Correct for an ungrouped metric. For a grouped one it requires deciding what
   the query even means — how a measure that does not vary along the grouping
   dimension should be aligned with one that does — and the spec does not say.
3. **Refuse.**

## Decision

Refuse. When a metric applies a duplicate-sensitive aggregate to a dataset the
join repeats, reject the query, naming the aggregate and the dataset and saying
what would work instead.

Aggregates that repeated rows cannot change are allowed: `MIN`, `MAX`, and any
`DISTINCT` aggregate.

## Consequences

- The compiler does not return a wrong number. This is the point.
- A model that is accepted elsewhere may be rejected here. Those numbers were
  wrong; that is a defensible position to be in.
- The check is conservative at the metric level: a metric is rejected if any
  part of it aggregates the repeated side, even where a narrower reading might
  be safe.
- The correct answer needs semantics the spec has not defined. See
  [ADR-0013](0013-cross-grain-metric-semantics.md).

