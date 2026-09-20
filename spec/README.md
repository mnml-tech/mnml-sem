# Vendored Ossie schema

`ossie-schema.json` is a verbatim copy of the Apache Ossie core metadata
schema. It is **not** authored here — it is vendored so the conformance tests
in [`tests/conformance.rs`](../tests/conformance.rs) can run offline and
deterministically, and so that a change upstream shows up as a diff rather than
as a surprise.

| | |
|---|---|
| Source | <https://github.com/apache/ossie/blob/main/core-spec/ossie-schema.json> |
| Upstream commit | `cc919e9e3818f5e0b798aa081345630d5af57b64` |
| Committed upstream | 2026-09-16 (*Define one semantic model per document without a wrapper*, #383) |
| Spec version | `0.2.0.dev0` |
| Licence | Apache-2.0, © the Apache Software Foundation |

The schema is the authority on the document shape; this crate's `model` types
are an implementation of it. The conformance tests assert the two agree — on
the spec version, on the `Dialect` and `DataType` enumerations, and on every
fixture being a legal Ossie document.

## Refreshing it

```sh
curl -o spec/ossie-schema.json \
  https://raw.githubusercontent.com/apache/ossie/main/core-spec/ossie-schema.json
cargo test --test conformance
```

Failures are the point: they tell you exactly what moved upstream. Update the
table above with the new commit when you refresh.
