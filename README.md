# ossie-substrait

> Compile [Apache Ossie](https://github.com/apache/ossie) semantic models into **Substrait** plans — portable IR for DataFusion, Velox, and other Substrait-capable engines.
>
> Targets core-spec **`0.2.0.dev0`**. Ossie was previously named Open Semantic Interchange (OSI).

## What is this?

**ossie-substrait** implements the path **Ossie document → validated model → bound query → logical plan → Substrait**. Load YAML/JSON, resolve a SemanticQuery against the document's model and compile an open **Substrait** plan.

The crate also exposes **ANSI-style SQL** via [`emit::sql::to_sql`](src/emit/sql.rs) — derived from the same structured plan (debugging, legacy engines).

- **Spec-aligned** — Types follow `core-spec/ossie-schema.json`, and [`tests/conformance.rs`](tests/conformance.rs) checks them against a vendored copy of it ([`spec/`](spec/)), so upstream drift shows up as a failing test.
- **Join-aware binding** — Relationships drive join paths; `dataset.field` references resolve through the model.
- **Engine-agnostic execution** — Substrait decouples the semantic layer from any one runtime.
- **Composable and lightweight** — Pure Rust library with no runtime server. Embed it in an API, CLI tool, or compute engine

```
Ossie ──► validate ──► CompiledModel ──► bind_query(SemanticQuery) ──► LogicalPlan ──► Substrait
                       (expressions                                                └──► SQL
                        lowered once)
```

## Features

| Area | What you get |
|------|----------------|
| **I/O** | Parse Ossie documents from YAML or JSON (`parser`) |
| **Validation** | Check documents (`validate`); compile expressions (`CompiledModel`) |
| **Queries** | Bind [`SemanticQuery`](src/query/spec.rs) (group by, metrics, filters, ordering, limit/offset, optional dataset anchor) |
| **Planning** | `LogicalPlan` for emission or inspection |
| **Functions** | One [`catalog`](src/catalog.rs) row per function carries its arity, SQL rendering, and Substrait coordinates |
| **Emit** | **`emit::substrait::to_plan`** — primary (default feature `substrait`) |
| **Emit** | `emit::sql::to_sql` — secondary; same structured plan |
| **Expressions** | SQL expressions (`ANSI_SQL`, `SNOWFLAKE`, `DATABRICKS`, `BIGQUERY`) parsed with [`sqlparser`](https://github.com/apache/datafusion-sqlparser-rs) and lowered to the plan IR ([`plan::ossie_sql`](src/plan/ossie_sql.rs)) |
| **Metrics** | Composite metrics (`SUM(a) / COUNT(DISTINCT b)`) split into Substrait measures plus a projection; cross-dataset metrics pull in their own joins |

## Example

```rust
use ossie_substrait::prelude::*;

let doc = from_yaml_str(yaml)?;
validate(&doc)?;

// Compile once: every field and metric expression is parsed and resolved here,
// and every error is reported together.
let model = CompiledModel::compile(&doc)?;

let query = SemanticQuery {
    metrics: vec!["row_count".into()],
    dataset: Some("fact".into()),
    ..Default::default()
};

let bound = bind_query(&model, &query)?;
let plan = build_logical_plan(&bound, &model)?;

#[cfg(feature = "substrait")]
{
    let _substrait = to_plan(&plan)?; // primary: feed to your Substrait executor
}

let _sql = to_sql(&plan)?; // secondary: inspect or send to a SQL engine
```

(`minimal_model` / `row_count` / `fact` match [`tests/fixtures/minimal.yaml`](tests/fixtures/minimal.yaml).)

## Development

```sh
cargo test   # default feature `substrait` includes Substrait + SQL tests
```

Conformance: [`spec/README.md`](spec/README.md) explains how to refresh the vendored Ossie schema; the tests then report exactly what moved.

Fixtures: [`tests/fixtures/`](tests/fixtures/). The TPC-DS example test is **`#[ignore]`** unless you clone [Ossie](https://github.com/apache/ossie) next to this repo (`../ossie/examples/…`); then `cargo test -- --include-ignored`.

## License

Licensed under **MIT OR Apache-2.0** — see `LICENSE-MIT` and `LICENSE-APACHE`.
