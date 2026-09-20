# 0003. Compile expressions from their SQL text

- Status: Accepted
- Date: 2026-09-20

## Context

Ossie expresses field and metric logic as SQL strings, tagged by dialect:

```yaml
expression:
  dialects:
    - dialect: ANSI_SQL
      expression: SUM(orders.amount)
```

The schema fixes this shape: a dialect entry is `{ dialect, expression }` with
`additionalProperties: false`. A string is all we get.

Substrait is not text. It needs a structured expression — function references,
typed arguments, field offsets. So something has to turn the string into
structure. Options:

1. **Pass the string through.** Adequate for a SQL target; impossible for
   Substrait, which is the primary target. It also makes every expression
   opaque: we cannot tell whether a metric aggregates, which datasets it reads,
   or whether it references something that exists.
2. **Require a structured expression instead.** More tractable, but it is not
   what the spec defines, and a document using it would not be an Ossie
   document.
3. **Parse the SQL.**

## Decision

Parse the expression text and lower it to the plan IR.

- Parsing uses `sqlparser`, whose dialect set covers the SQL dialects in the
  Ossie enum (`ANSI_SQL`, `SNOWFLAKE`, `DATABRICKS`, `BIGQUERY`). The
  non-SQL ones (`MDX`, `TABLEAU`, `MAQL`, `SIGMA`, `THOUGHTSPOT`, `DAX`) are
  refused explicitly rather than mis-parsed.
- A SQL parser accepts far more than an Ossie expression may contain. The
  lowering pass is where that is enforced: `SELECT`, `FROM`, `JOIN`, subqueries
  and the rest are rejected with a reason.
- Identifiers resolve in one of two modes. A field's own definition reads
  physical columns of its dataset; a metric's expression resolves
  `dataset.field` through the model to that field's definition. Without the
  split, a field whose expression is `id` would resolve to itself forever.

## Consequences

- Expressions can be analysed, not just forwarded: we can tell an aggregate from
  a scalar, know which datasets a metric touches, and reject a reference to a
  field that does not exist.
- The spec's own rule that fields are scalar and metrics aggregate becomes
  checkable.
- We inherit a SQL parser's surface area and have to gate it. That gate is the
  main place this crate can be too strict or too lax.
- A model can only be compiled for a dialect this crate can parse.

