# 0002. Mirror the schema, and check against a vendored copy

- Status: Accepted
- Date: 2026-09-20

## Context

This crate implements a specification it does not own. The core spec is at
`0.2.0.dev0`, which its own text describes as mutable and not to be depended on
in production. The model types here are an *implementation* of
`core-spec/ossie-schema.json`, and nothing in the language keeps them honest.

The failure mode is silent drift: upstream adds a dialect, changes which fields
are required, or moves the version, and nothing notices until a real document
fails to parse — or worse, parses and means something different.

There is a prior question. The types could **mirror** the schema — one type per
`$def`, the same field names, optional where the schema says optional — or they
could be a **normalized internal model** the document is translated into, with
the spec's awkward corners smoothed out on the way in. Translation gives nicer
types to work with and insulates the rest of the crate from the spec. It also
removes the correspondence that makes drift detectable: with nothing to compare
type-for-type, there is no assertion to write beyond "it still parses".

Three options for the drift itself:

1. Trust the types and find out when a document breaks.
2. Fetch the schema during tests. Accurate, but the test suite then needs the
   network and can fail for reasons unrelated to the change under test.
3. Vendor the schema and assert the types agree with it.

## Decision

**The model types mirror the schema.** One type per `$def`, field names as the
schema spells them, `deny_unknown_fields` throughout, optionality following the
schema's `required`. Nothing is smoothed over, renamed, or dropped, and the
crate's own conveniences are methods on those types rather than a second shape.

Vendor `ossie-schema.json` under [`crates/ossie-model/spec/`](../../crates/ossie-model/spec/), recording the upstream
commit it came from, and assert in the test suite that:

- the spec version this crate targets is the version the schema pins,
- the `Dialect` and `DataType` enumerations match by **wire form**, reporting
  both what is missing here and what is not in the schema,
- the root's required fields are the ones the types make non-optional,
- every fixture is a valid Ossie document,
- everything this crate writes round-trips to a valid document,
- a document using every optional field the schema permits still parses, so the
  parser cannot become stricter than the spec.

Refreshing the vendored file is what surfaces an upstream change. The JSON
Schema validator is a dev-dependency: the serde types remain the runtime
contract, and the schema is how we check they are still right.

## Consequences

- An upstream change arrives as a reviewable diff and a set of failing tests
  that name what moved, rather than as a mystery.
- Tests are offline and deterministic.
- The vendored copy can go stale. Nothing detects that automatically; refreshing
  it is a deliberate act.
- The types are exactly as awkward as the spec is. `ai_context` is a string or
  an object; a dialect entry has to be searched for rather than looked up;
  `datatype` is optional everywhere it appears. Callers see all of that.
- A spec change is a breaking change here. There is no insulating layer to
  absorb it, which is the same property that makes the change visible.
- Parts of the document are carried without being used — labels, descriptions,
  AI context — because a document must round-trip to itself.

