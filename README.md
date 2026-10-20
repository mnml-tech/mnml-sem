# ossie-substrait

> Compile [Apache Ossie](https://github.com/apache/ossie) semantic models into **Substrait** plans — portable IR for DataFusion, Velox, and other Substrait-capable engines.
>
> Targets core-spec **`0.2.0.dev0`**. Ossie was previously named Open Semantic Interchange (OSI).

## What is this?

**ossie-substrait** implements the path **Ossie document → validated model → bound query → logical plan → Substrait**. Load YAML/JSON, resolve a SemanticQuery against the document's model and compile an open **Substrait** plan.

It also emits **ANSI-style SQL** via [`emit::sql::to_sql`](crates/ossie-compiler/src/emit/sql.rs) — derived from the same structured plan (debugging, legacy engines).

## Crates

The repository is a Cargo workspace ([ADR-0015](docs/adr/0015-a-workspace-of-crates.md)). Depend on the lowest one that does what you need:

| Crate | What it does | Depends on |
|-------|--------------|------------|
| [`ossie-model`](crates/ossie-model) | Document types, parsing, structural validation | `serde` |
| [`ossie-compiler`](crates/ossie-compiler) | `CompiledModel`, `SemanticQuery`, binding, planning, SQL emission | `ossie-model`, `sqlparser` |
| [`ossie-substrait`](crates/ossie-substrait) | `LogicalPlan` → Substrait | `ossie-compiler`, `substrait` |

- **Spec-aligned** — Types follow `core-spec/ossie-schema.json`, and [`conformance.rs`](crates/ossie-model/tests/conformance.rs) checks them against a vendored copy of it ([`spec/`](crates/ossie-model/spec/)), so upstream drift shows up as a failing test.
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
| **I/O** | Parse Ossie documents from YAML or JSON (`ossie_model::parser`) |
| **Validation** | Check documents (`ossie_model::validate`); compile expressions (`CompiledModel`) |
| **Queries** | Bind [`SemanticQuery`](crates/ossie-compiler/src/query/spec.rs) (group by, metrics, filters, ordering, limit/offset, optional dataset anchor) |
| **Planning** | `LogicalPlan` for emission or inspection; refuses a metric a join would inflate rather than returning a wrong number |
| **Functions** | One [`catalog`](crates/ossie-compiler/src/catalog.rs) row per function carries its arity, SQL rendering, and Substrait coordinates |
| **Emit** | **`ossie_substrait::to_plan`** — primary |
| **Emit** | `emit::sql::to_sql` — secondary; same structured plan, coalescing a maximal `SELECT` and opening a subquery only at a clause conflict |
| **Expressions** | SQL expressions (`ANSI_SQL`, `SNOWFLAKE`, `DATABRICKS`, `BIGQUERY`) parsed with [`sqlparser`](https://github.com/apache/datafusion-sqlparser-rs) and lowered to the plan IR ([`plan::ossie_sql`](crates/ossie-compiler/src/plan/ossie_sql.rs)) |
| **Metrics** | Composite metrics (`SUM(a) / COUNT(DISTINCT b)`) split into Substrait measures plus a projection; cross-dataset metrics pull in their own joins |
| **Metrics** | Dataset-scoped metrics (`datasets[].metrics`, queried as `dataset.metric`, and built on by model-scoped metrics the same way) from the [proposal](https://github.com/apache/ossie/pull/591) not yet in the spec — see [ADR-0014](docs/adr/0014-dataset-scoped-metrics-ahead-of-the-schema.md) |

## Example

```rust
use ossie_substrait::prelude::*; // or ossie_compiler::prelude::* for SQL only

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

let _substrait = to_plan(&plan)?; // primary: feed to your Substrait executor

let _sql = to_sql(&plan)?; // secondary: inspect or send to a SQL engine
```

(`minimal_model` / `row_count` / `fact` match [`minimal.yaml`](crates/ossie-model/tests/fixtures/minimal.yaml).)

## Why it works this way

[`docs/adr/`](docs/adr/) records the decisions that are not obvious from the
code — where the spec is silent, where it under-specifies something a target
format requires, and where the obvious implementation would be wrong.

## Development

```sh
cargo test --workspace
```

Conformance: [`spec/README.md`](crates/ossie-model/spec/README.md) explains how to refresh the vendored Ossie schema; the tests then report exactly what moved.

Fixtures: [`crates/ossie-model/tests/fixtures/`](crates/ossie-model/tests/fixtures/), shared by every crate's tests. The end-to-end tests (parse → plan → SQL and Substrait) live in [`crates/ossie-substrait/tests/`](crates/ossie-substrait/tests/). Among them, [`datafusion.rs`](crates/ossie-substrait/tests/datafusion.rs) has [DataFusion](https://datafusion.apache.org) (a dev-dependency only) execute both the emitted Substrait plan and the emitted SQL against in-memory tables, and checks they return the same rows. The TPC-DS example test is **`#[ignore]`** unless you clone [Ossie](https://github.com/apache/ossie) next to this repo (`../ossie/examples/…`); then `cargo test --workspace -- --include-ignored`.

## License

Licensed under **MIT OR Apache-2.0** — see `LICENSE-MIT` and `LICENSE-APACHE`.
