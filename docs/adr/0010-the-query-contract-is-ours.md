# 0010. The query contract is ours

- Status: Accepted
- Date: 2026-09-20

## Context

The Ossie core spec describes a *model*: datasets, fields, relationships,
metrics. It does not describe a query. There is no standard way to say "these
metrics, grouped by these fields, filtered like this".

Something has to say it. A compiler that only accepts a model has nothing to
compile.

## Decision

Define the query shape in this crate, as `SemanticQuery`, and treat it as ours
to design — not as an extrapolation of the spec.

It references model entities by name and never restates them: metric names,
`dataset.field` references, an optional anchor dataset for the join root. On top
of that it carries what a caller needs and the model cannot provide: typed
filter operators, ordering with explicit nulls placement, and a row limit.

Two rules keep it from drifting into the model's territory:

- It names things the model defines; it does not define things itself. There is
  no way to declare a metric inline in a query.
- Ordering references what the query *selects* — a grouped field or a metric —
  rather than an arbitrary expression, so the output columns are the vocabulary.

## Consequences

- We can shape it freely and change it without any claim about Ossie.
- It is not interoperable. A query written against this crate is not portable to
  another Ossie tool, and should not be presented as if it were.
- If the spec later defines a query format, this becomes a compatibility problem
  to solve deliberately, and this ADR is superseded rather than quietly amended.

