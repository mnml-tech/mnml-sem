# 0012. SQL emission accumulates clauses

- Status: Accepted
- Date: 2026-09-20

## Context

A plan is a tree of relational operators that compose freely. SQL is a
clause-based language: one `SELECT` has exactly one `WHERE`, one `GROUP BY`, one
`ORDER BY`. Several plan nodes usually collapse into a single `SELECT`, and
sometimes they cannot — filtering the output of an aggregation cannot reuse that
aggregation's `WHERE`, because `WHERE` runs before `GROUP BY`.

An emitter can either recognise the shapes it expects and reject the rest, or
work out for itself where one `SELECT` has to stop. The first is smaller and
reads clearly for a fixed pipeline; it also means every new plan node extends a
pattern match, and no plan needing a subquery can be emitted at all.

## Decision

Accumulate clauses. Walk the plan building one block of `SELECT` clauses, and
open a subquery only at a real boundary — when a node wants a clause that SQL
evaluates *before* one the block has already occupied:

```
From < Where < GroupBy < Select < OrderBy < Fetch
```

A boundary is rendered as an **inline subquery**, not a common table
expression. The emitter builds bottom-up and a subquery is the shape that
falls out of it; hoisting to a `WITH` clause would mean accumulating fragments
separately and assembling them at the statement head. More to the point,
engines differ on whether a CTE is materialized — some always, some never,
some by heuristic — so choosing one would be choosing a performance
characteristic on the caller's behalf, invisibly.

Wrapping a block rewrites the references that cross it: a subquery preserves its
output names, so the wrapped block's columns move behind its alias. That
includes join keys, which otherwise keep pointing at a dataset that is no longer
directly in scope.

This is the design used by Calcite's `RelToSqlConverter` and DataFusion's
unparser.

## Consequences

- Any plan emits. There is no shape the emitter has to refuse for being
  unexpected.
- A projection over an aggregate stays in one block, because a select list can
  hold a composite of aggregates — which is what keeps the output readable.
- More machinery than a pattern match, and the conflict rule is the kind of
  thing that is subtly wrong rather than obviously broken. Calcite still files
  bugs against its equivalent. The boundaries need tests of their own, not just
  tests of the shapes the planner happens to build.
- Filtering after aggregating emits a subquery where `HAVING` would be
  idiomatic. Correct, but not what a person would write; folding that case is a
  later refinement on top of this structure.
- A deeply nested plan produces deeply nested SQL, which is harder to read than
  a sequence of named CTEs. If that becomes the complaint, hoisting the same
  boundaries into a `WITH` clause is a change to rendering, not to where the
  boundaries are.

