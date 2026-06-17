# osi-substrait

> Compile [Open Semantic Interchange](https://github.com/open-semantic-interchange/OSI) (OSI) semantic models into **Substrait** plans — portable IR for DataFusion, Velox, and other Substrait-capable engines.

## What is this?

**osi-substrait** implements the path **OSI document → validated model → bound query → logical plan → Substrait**. Load YAML/JSON, resolve a SemanticQuery against a named model and compile an open **Substrait** plan.

The crate also exposes **ANSI-style SQL** via [`emit::sql::to_sql`](src/emit/sql.rs) — derived from the same structured plan (debugging, legacy engines).

- **Spec-aligned** — Types follow `osi-schema.json`; validation runs before planning.
- **Join-aware binding** — Relationships drive join paths; `dataset.field` references resolve through the model.
- **Engine-agnostic execution** — Substrait decouples the semantic layer from any one runtime.
- **Composable and lightweight** — Pure Rust library with no runtime server. Embed it in an API, CLI tool, or compute engine

```
OSI ──► validate ──► bind_query(model, SemanticQuery) ──► LogicalPlan ──► Substrait
                                                                     └──► SQL        
```

## Features

| Area | What you get |
|------|----------------|
| **I/O** | Parse OSI from YAML or JSON (`parser`) |
| **Validation** | Check documents (`validate`) |
| **Queries** | Bind [`SemanticQuery`](src/query/spec.rs) (group by, metrics, filters, optional dataset anchor) |
| **Planning** | `LogicalPlan` for emission or inspection |
| **Emit** | **`emit::substrait::to_plan`** — primary (default feature `substrait`) |
| **Emit** | `emit::sql::to_sql` — secondary; same structured plan |
| **Expressions** | Structured **MNML** dialect for fields & metrics — see [`docs/mnml-expressions.md`](docs/mnml-expressions.md) |

## Example

```rust
use osi_substrait::prelude::*;

let doc = from_yaml_str(yaml)?;
validate(&doc)?;

let query = SemanticQuery {
    metrics: vec!["row_count".into()],
    dataset: Some("fact".into()),
    ..Default::default()
};

let bound = bind_query(&doc, "minimal_model", &query)?;
let model = doc.semantic_model.iter().find(|m| m.name == "minimal_model").unwrap();
let plan = build_logical_plan(&bound, model)?;

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

Fixtures: [`tests/fixtures/`](tests/fixtures/). The TPC-DS example test is **`#[ignore]`** unless you clone [OSI](https://github.com/open-semantic-interchange/OSI) next to this repo (`../OSI/examples/…`); then `cargo test -- --include-ignored`.

## License

Licensed under **MIT OR Apache-2.0** — see `LICENSE-MIT` and `LICENSE-APACHE`.
