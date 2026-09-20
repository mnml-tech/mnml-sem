# 0009. The plan models the more constrained target

- Status: Accepted
- Date: 2026-09-20

## Context

One plan feeds two emitters with different capabilities. A Substrait
`AggregateRel` measure holds exactly one aggregate call; a SQL select list holds
any expression at all. So a metric like `SUM(a) / COUNT(DISTINCT b)` fits one
target directly and not the other.

Whichever way the plan is shaped, one emitter has to do work:

- Shape it for SQL — one measure holding the whole expression — and the
  Substrait emitter must split it into two measures plus a projection.
- Shape it for Substrait — two measures plus a projection — and the SQL emitter
  must fold the projection back into the select list.

The work is comparable either way. What differs is where the *transformation*
hides, and whether the plan tells the truth about what will be emitted.

## Decision

The plan carries the shape the **more constrained** target needs: aggregates are
lifted into separate measures and a `Project` above the aggregate recombines
them. Substrait maps that one-to-one. The SQL emitter, which is free to do
otherwise, folds it back.

## Consequences

- The plan is a faithful model of the Substrait output. Rendering it shows the
  projection that will exist, rather than concealing a transformation inside an
  emitter.
- Folding is always safe in the permissive direction — a select list can express
  what a measure cannot, never the reverse — so this is the direction that
  cannot fail.
- SQL output is unchanged by the lifting: the synthesized measure names never
  appear.
- When a third target appears that is more constrained still, this rule says the
  plan should move toward it, and both existing emitters absorb the difference.

