# 0013. Cross-grain metric semantics

- Status: Proposed
- Date: 2026-09-20

## Context

[ADR-0011](0011-refuse-a-metric-a-join-would-inflate.md) refuses a metric whose
aggregate reads a dataset the join repeats. That is the safe answer, not the
useful one. The useful one requires knowing what such a metric means.

Take a metric dividing a measure of a many-side dataset by a measure of a
one-side dataset — sales over headcount, say. Ungrouped, it is unambiguous:
aggregate each dataset independently and combine the two scalars.

```sql
SELECT s.total / NULLIF(e.staff, 0)
FROM (SELECT SUM(amount) AS total FROM sales) s
CROSS JOIN (SELECT SUM(headcount) AS staff FROM store) e
```

Grouped by a dimension of the many side — by year, say — it is not. Sales vary
by year; headcount does not. At least three answers are defensible:

1. the denominator is the total across all stores, repeated for every year;
2. the denominator covers only stores that had sales in that year;
3. the denominator is as of that year, which the model has no way to express.

They give different numbers. The core spec does not choose between them, and the
question is live upstream — entity and grain as first-class concepts, and
cross-dataset dimensions against single-dataset measures, are both open
discussions rather than settled parts of the spec.

## Decision

*Proposed — not decided.*

The options, in the order they cost:

- **A. Keep refusing.** No semantics invented. The compiler stays honest and
  stays limited.
- **B. Support only the unambiguous case** — metrics-only, no grouping — and
  refuse the rest. Half the value, and a metric that works for one query and
  fails for another, which is its own kind of confusing.
- **C. Choose a rule** for the grouped case and document it. Most useful, and it
  means this crate defines something the spec does not — so two conformant tools
  could disagree about the same model and both be right.

The argument for waiting is that C is not a decision an implementation should
make alone; the argument against is that refusing a real metric in a published
example model is a poor advertisement.

Whatever is chosen, the machinery exists: the plan can express aggregation below
a join, and the SQL emitter can emit it
([ADR-0012](0012-sql-emission-accumulates-clauses.md)). What is missing is the
rule, not the capability.

## Consequences

Unresolved. Until this is decided, ADR-0011 stands and these metrics are
refused.

