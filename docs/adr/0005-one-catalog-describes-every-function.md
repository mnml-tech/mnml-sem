# 0005. One catalog describes every function

- Status: Accepted
- Date: 2026-09-20

## Context

Supporting a function means knowing several unrelated-looking things about it:

- the SQL spellings that parse to it,
- how many arguments it takes,
- how it renders in each target dialect,
- its Substrait extension group and function anchor,
- what type it returns, given its arguments,
- for an aggregate, whether repeating an input row changes its result.

Each of those belongs to a different module — the expression lowering, the SQL
emitter, the Substrait emitter, type inference, the fan-out check. The natural
thing is for each module to grow its own `match` over function names. That is
also how the two emitters come to disagree about what `SUM` is, and how a
function ends up half-supported: parseable but unrenderable, or emitted to SQL
but not to Substrait.

## Decision

One table. Every function is a row carrying all of it, and every consumer reads
that row rather than matching on the function itself.

Adding a function is adding a row. There is no second place to update, and the
row cannot be written without answering every question — including the ones
easy to forget, like whether an aggregate is sensitive to duplicated rows.

## Consequences

- The emitters cannot drift, because neither owns the vocabulary.
- A new function is immediately reachable from a model expression, since
  lowering resolves names through the same table.
- The table carries some fields that are meaningless for some rows — arity for a
  niladic keyword, for instance. That is the price of one row per function.
- Anything genuinely irregular has to be special-cased somewhere, and the
  pressure is to distort the table rather than admit the exception.

