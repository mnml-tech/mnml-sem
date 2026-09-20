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

**While the design is still moving, amend records in place.** These describe the
design as it stands, not how it arrived there. A decision reversed during design
was never really in force, and keeping a superseded record of it leaves readers
sifting archaeology to find what is true now. There is exactly one set of
records, and it is current.

That changes once the crate is released and callers depend on its behaviour. At
that point a reversal is a real event with real consequences for someone, and
the record supersedes rather than replaces.

## Consequences

- The reasoning survives the change that introduced it.
- Until release, git history is the only record of how a decision moved. That is
  the right place for it; the ADRs stay a description of the present.
- A decision worth recording is one more thing to write, and one more thing to
  keep true when the code moves.

