# Vendored Ossie schemas

These are verbatim copies of Apache Ossie's schemas. They are **not** authored
here — they are vendored so the conformance tests can run offline and
deterministically, and so that a change upstream shows up as a diff rather than
as a surprise.

| File | Upstream path | Upstream commit | Committed upstream | Checked by |
|---|---|---|---|---|
| `ossie-schema.json` | `core-spec/ossie-schema.json` | `32c293c79a80b0c11e01bd76357950284aeaf3f8` | 2026-10-13 (*Require a column in primary_key and in each unique key*, #574) | [`tests/conformance.rs`](../tests/conformance.rs) |
| `ontology.json` | `ontology/ontology.json` | `c91b5decf3f4c120640f493bf3532dcd1625f6e4` | 2026-10-14 (*Require a mapping to say how it populates anything*, #582) | [`tests/ontology_conformance.rs`](../tests/ontology_conformance.rs) |
| `mapping.json` | `ontology/mapping.json` | `c91b5decf3f4c120640f493bf3532dcd1625f6e4` | 2026-10-14 (*Require a mapping to say how it populates anything*, #582) | [`tests/ontology_conformance.rs`](../tests/ontology_conformance.rs) |

All are at spec version `0.2.0.dev0`, Apache-2.0, © the Apache Software
Foundation. The ontology and mapping schemas `$ref` the core schema, and each
other, by their raw GitHub URLs; the tests resolve those URLs to the vendored
copies and refuse any other, so nothing is fetched.

The schemas are the authority on the document shapes; this crate's types are an
implementation of them. The conformance tests assert the two agree — on the
spec versions, on every enumeration, on every property the schemas declare, and
on every fixture being a legal document.

The fixtures `flights.ontology.yaml`, `flights.mapping.yaml` and
`flights.semantic_model.yaml` are upstream's example, vendored from
`examples/` at `1ea545c2a76211cbdc8d3460109957de5abf3b5d` with their licence
headers.

## Refreshing them

```sh
base=https://raw.githubusercontent.com/apache/ossie/main
curl -o crates/ossie-model/spec/ossie-schema.json $base/core-spec/ossie-schema.json
curl -o crates/ossie-model/spec/ontology.json $base/ontology/ontology.json
curl -o crates/ossie-model/spec/mapping.json $base/ontology/mapping.json
cargo test -p ossie-model
```

Failures are the point: they tell you exactly what moved upstream. Update the
table above with the new commits when you refresh.
