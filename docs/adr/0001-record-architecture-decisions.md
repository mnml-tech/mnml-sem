# 0001. Record architecture decisions

- Status: Accepted
- Date: 2026-09-20

## Context

This crate compiles Apache Ossie semantic models into Substrait plans and SQL.
Much of its behaviour follows directly from the spec and needs no explanation.
Some of it does not:

- places where the spec is silent and we had to choose,
- places where the spec under-specifies something a target format requires,
- places where the obvious implementation would be wrong.

Those choices are invisible in the code — the code shows what was decided, not
what else was considered or why it was rejected. Without a record they get
re-litigated, or worse, quietly reversed by someone who reads the code as
arbitrary.

## Decision

Record them here as numbered Architecture Decision Records, in the format of
[`template.md`](template.md): Context, Decision, Consequences.

Write one when a decision is *contested* — when a competent reader could
reasonably expect a different answer. Not for decisions the spec makes for us.

Where a decision has to be made before the code, the ADR comes first, with
`Status: Proposed`.

## Consequences

- The reasoning survives the change that introduced it.
- An ADR is superseded, never edited away; the history of a reversal is part of
  the record.
- There is a cost: a decision worth recording is one more thing to write.

