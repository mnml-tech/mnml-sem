# 0007. Unknown types stay unknown

- Status: Accepted
- Date: 2026-09-20

## Context

Substrait plans carry types: a scan declares a schema, and functions may declare
what they return. Ossie is more relaxed:

- `datatype` is **optional** on fields and metrics, and the spec says to omit it
  when the type is unknown.
- `Decimal` states neither precision nor scale. Substrait's decimal requires
  both.
- `Opaque` is defined as a type outside the portable vocabulary.

So for a large share of real columns we are asked to declare a type we do not
know. Declaring one anyway — `Decimal(38, 10)` is the usual guess — produces a
plan that looks complete and is wrong, and a consumer that trusts it will
mis-handle values. Getting a decimal's scale wrong changes numbers.

## Decision

Say nothing rather than guess.

- Absent `datatype`, `Opaque`, and `Decimal` map to Substrait's `unknown`
  extension type.
- Where the declaration does determine a type, map it concretely. `Integer`
  widens to `i64` — the spec leaves width unspecified, and `i64` holds every
  integral value Ossie can describe. `Time`, `DateTime` and `DateTimeTz` map to
  the `Precision*` types at microsecond precision, which is the meaning the
  types they replace carried.
- Unknown propagates. An expression over an unknown input is unknown, and an
  aggregate whose type is unknown is emitted with no declared output type rather
  than a guessed one.

## Consequences

- A consumer that trusts the plan's types is not misled by us.
- Plans over models that declare few types carry little type information. That
  is a faithful report of how much the model said.
- The two microsecond defaults are still choices; they are named constants so
  they can be found and argued with.

