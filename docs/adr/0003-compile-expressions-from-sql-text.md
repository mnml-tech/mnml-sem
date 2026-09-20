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
- **Identifier text is preserved; matching is what varies.** The expression
  language says an unquoted identifier is case-insensitive and that the
  normalized form used for matching is upper-cased. Normalizing for *matching*
  is not the same as normalizing for *emission*: folding an identifier's case
  on the way out rewrites the SQL handed to the engine and cannot be undone. So
  the text is kept exactly as written, and only the comparisons fold —
  `amount` and `AMOUNT` name one column, and two fields spelling it differently
  produce one scan column rather than two.
- **Entity names are matched as written.** A `dataset.field` reference is not a
  SQL identifier; it names a `name` in the document, which the schema
  constrains in no way. The expression language routes how such a reference is
  looked up to a semantics document that is not published, so there is no rule
  to follow — and folding both sides would make two entities collide, since a
  model may legitimately declare `Orders` and `orders` as distinct. When a
  reference differs from a declared name only by case, the error says which
  name was declared, so matching as written is strict without being unhelpful.
- **Which dialect entry is compiled is chosen, not asked for.** An expression
  may carry several. The `ANSI_SQL` entry is preferred, then any other entry in
  a dialect that can be parsed, then the first entry — which then fails if it
  is not SQL. Preferring ANSI means the compiled expression is the portable
  one, so the plan can be emitted to any target rather than to the vendor the
  entry was written for.
- **An expression may reference any dataset; the joins follow from it.** A
  metric naming `customers.id` causes the binder to find a join path, rather
  than requiring the query to ask for one. This is what makes
  [ADR-0011](0011-refuse-a-metric-a-join-would-inflate.md) necessary: a
  reference is enough to create a join, so a reference is enough to create a
  fan-out.
- **A metric may not reference another metric.** The spec anticipates this —
  its namespace table has a row for metric references — and then defers the
  rules to the same unpublished semantics document. Simple substitution
  (`revenue_per_unit = total_revenue / total_units`) would be a small change;
  what is undecided is how a bare name resolves when a metric and a dataset
  share it, and what a reference means when the two aggregate at different
  grains. Until there is a rule, a metric referencing a metric fails as an
  unresolved reference.

## Consequences

- Expressions can be analysed, not just forwarded: we can tell an aggregate from
  a scalar, know which datasets a metric touches, and reject a reference to a
  field that does not exist.
- The spec's own rule that fields are scalar and metrics aggregate becomes
  checkable.
- We inherit a SQL parser's surface area and have to gate it. That gate is the
  main place this crate can be too strict or too lax.
- A model can only be compiled for a dialect this crate can parse.
- A model whose entity names are not lowercase works, and one whose references
  disagree with its declarations about case does not.
- Emitted SQL says what the model said. A field written `AMOUNT` is emitted as
  `AMOUNT`, so a reader can match the output against the model, and an engine
  with case-sensitive quoted columns is not broken by a normalization we chose.
- The caller cannot say which dialect entry to compile. A model carrying both
  `ANSI_SQL` and `BIGQUERY` is always compiled from the ANSI entry, even when
  BigQuery is the target and the vendor entry was written to say something the
  portable one cannot. The plan IR is dialect-neutral, so this is usually
  invisible; where it is not, there is currently no way to ask.
- Reuse across metrics is not available. A model repeating
  `SUM(orders.amount)` in five metrics repeats it five times, and a change has
  to be made five times.

