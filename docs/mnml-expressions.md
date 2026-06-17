# MNML expression dialect

> **Scope.** This documents the `MNML` expression dialect implemented by
> `osi-substrait`. It is an **extension** to the OSI core expression model, not
> part of the OSI spec. The authoritative reference is the code — chiefly
> [`src/plan/mnml.rs`](../src/plan/mnml.rs) (node grammar),
> [`src/model/expression.rs`](../src/model/expression.rs) (parsing rules), and
> the emitters in [`src/emit/`](../src/emit/). This file is kept in sync with
> those; when they disagree, the code wins.

## Why it exists

OSI core defines a field/metric `expression` as a list of **dialect** entries,
each carrying a SQL string for a named dialect (`ANSI_SQL`, `SNOWFLAKE`, …):

```yaml
expression:
  dialects:
    - dialect: ANSI_SQL
      expression: COUNT(DISTINCT customer_id)
```

A SQL string is opaque to the planner: it can be passed through to a SQL engine
verbatim, but it cannot be compiled to a portable **Substrait** plan. The `MNML`
dialect replaces the SQL string with a **structured** JSON/YAML expression tree
that the planner compiles into the internal [`Expr`](../src/plan/expr.rs) IR,
and from there into **both** Substrait and ANSI SQL.

```yaml
expression:
  dialects:
    - dialect: MNML
      aggregation: count
      distinct: true
      field: orders.customer_id
```

## Surface form

MNML expressions live in a **dialect entry** with `dialect: MNML` whose body is
**structured keys** instead of an `expression` string:

```yaml
fields:
  - name: starter_flag
    expression:
      dialects:
        - dialect: MNML
          case:
            - when: { eq: [ { col: is_starter }, { lit: 1 } ] }
              then: { lit: 1 }
          else: { lit: 0 }
```

Parsing rules (enforced in `DialectExpression`'s deserializer):

- An `MNML` entry **must** use structured keys; supplying `expression:` is an
  error (*"MNML dialect must use structured keys … not expression string"*).
- An `MNML` entry **must not** be empty.
- Conversely, a non-MNML entry (`ANSI_SQL`, etc.) must use **only**
  `expression:`; extra keys are an error.
- The dialect token is `MNML` (the `Dialect` enum is `SCREAMING_SNAKE_CASE`).
- When multiple dialects are present, the planner uses the **first** `MNML`
  entry (`pick_mnml_dialect`).

> **Not to be confused with MNML storage extensions.** A dataset may carry a
> `custom_extensions` entry with `vendor_name: MNML` and a JSON payload like
> `{"version":1,"format":"parquet","location":"…"}` — that describes physical
> storage (`is_mnml_storage_json`), not an expression, and is handled
> separately. MNML *expressions* only ever appear as the `dialect: MNML`
> entry described above.

## Expression nodes

Every node is a single-key object. These are the **only** recognized nodes
(`mnml_value_to_expr`); anything else is rejected with
`InvalidMnmlExpression`.

| Node                         | Shape                                             | Meaning / `Expr`        |
|------------------------------|---------------------------------------------------|-------------------------|
| `col`                        | `{ col: "dataset.field" }` or `{ col: "field" }`  | column reference        |
| `lit`                        | `{ lit: <scalar> }`                               | literal                 |
| `eq`                         | `{ eq: [a, b] }`                                  | `a = b`                 |
| `ne`                         | `{ ne: [a, b] }`                                  | `a <> b`                |
| `and`                        | `{ and: [n1, n2, …] }`                            | conjunction            |
| `in`                         | `{ in: { col: "f", values: [v1, v2] } }`          | membership (literals)   |
| `sub`                        | `{ sub: [a, b] }`                                 | `a - b`                 |
| `current_date` / `today`     | `{ current_date: true }`                          | `CURRENT_DATE`          |
| `case`                       | see below                                         | `CASE WHEN … ELSE … END`|

### `lit` scalars

Literals are taken from the JSON value (`Literal::from_json`):

| JSON          | Literal     | SQL render          |
|---------------|-------------|---------------------|
| `null`        | `Null`      | `NULL`              |
| `true`/`false`| `Bool`      | `TRUE` / `FALSE`    |
| integer       | `Int64`     | `123`               |
| float         | `Float64`   | `1.5`               |
| string        | `String`    | `'…'` (`'`→`''`)    |

### `case`

```yaml
case:
  - when: <node>
    then: <node>
  - when: <node>
    then: <node>
else: <node>        # optional; defaults to lit null
```

`else` is read from the **enclosing** object (sibling of `case`), not from
inside a branch. If omitted, the else result is `NULL`.

### `in`

`in` takes a **column** and a list of **literals** only — not arbitrary
sub-expressions:

```yaml
in:
  col: status
  values: [ "active", "trial" ]
```

## Metric definitions

A metric's `dialect: MNML` body is a `MnmlMetricDef` (`deny_unknown_fields`):

| Key                          | Required | Meaning                                            |
|------------------------------|----------|----------------------------------------------------|
| `aggregation` (alias `agg`)  | yes      | `count` \| `sum` \| `avg` \| `min` \| `max`        |
| `distinct`                   | no       | `DISTINCT` flag (default `false`)                  |
| `field`                      | one of   | semantic field reference (`dataset.field`)         |
| `expr` (alias `arg`)         | one of   | inline expression node, aggregated directly        |

Normalization (`metric_mnml_to_def_value`):

- `aggregation` → `agg`.
- When `field` is absent and `agg` is present, `expr` → `arg`.
- Using `expr` without an `aggregation` is an error.
- A metric with neither `aggregation` nor `agg` is an error.

`field` vs `expr`/`arg`:

- **`field`** resolves the referenced field's **full** definition — if that
  field is itself an MNML expression (e.g. a `CASE` flag), the whole tree is
  aggregated (`resolve_field_ref`). This is how
  `SUM(CASE WHEN … THEN 1 ELSE 0 END)` is expressed as `sum` over a flag field.
- **`expr`/`arg`** aggregates an inline node directly, e.g.
  `MAX(CURRENT_DATE - employment_start_date)`.

```yaml
metrics:
  # COUNT(DISTINCT order_date)
  - name: active_days
    expression:
      dialects:
        - dialect: MNML
          aggregation: count
          distinct: true
          field: orders.order_date

  # MAX(CURRENT_DATE - employment_start_date)
  - name: tenure
    expression:
      dialects:
        - dialect: MNML
          aggregation: max
          expr:
            sub:
              - current_date: true
              - col: emp.employment_start_date
```

## Column resolution

`dataset.field` paths are resolved against the model (`parse_col_path`):

- A qualified `dataset.field` is used as-is.
- An unqualified `field` uses the **default dataset**: for a field expression,
  the dataset the field lives on; for a metric, there is **no** default, so a
  metric must qualify its references.

Two resolution modes:

- **`col` nodes** (filters, comparisons) resolve to the **physical scan
  column** (`physical_column_for_field`). A field used as a `col` must map to a
  physical column — a field whose MNML body is itself a non-column expression
  (e.g. a `CASE`) cannot be used as a bare `col`.
- **`field` in a metric** resolves the field's full logical expression
  (`resolve_field_ref`), following its MNML body when present.

## Emission coverage

The `Expr` IR is emitted to Substrait (`emit_expr`) and to ANSI SQL
(`format_expr`). Coverage is **not symmetric** — SQL is strictly more permissive.

| `Expr`            | Substrait                                | ANSI SQL              |
|-------------------|------------------------------------------|-----------------------|
| `Column`          | ✅ field reference                        | ✅                    |
| `Literal`         | ✅                                        | ✅                    |
| `Eq` / `Ne`       | ✅ `equal` / `not_equal`                  | ✅ `=` / `<>`         |
| `And`             | ✅ `and`                                  | ✅ `AND`              |
| `In`              | ✅ desugared to OR-of-`equal`             | ✅ `IN (…)`           |
| `Case`            | ✅ `IfThen`                               | ✅ `CASE … END`       |
| `Sub`             | ✅ `subtract`                             | ✅ `(a - b)`          |
| `CurrentDate`     | ✅ `current_date`                         | ✅ `CURRENT_DATE`     |
| `Agg`             | ⚠️ only as a top-level metric (`NamedAggregate`); **rejected inline** | ✅ |
| `Sql(string)`     | ❌ `UnsupportedExpression`                | ✅ passthrough        |

Notes:

- **`Sql` is Substrait's hard wall.** Any field/metric the planner cannot
  structure (e.g. an `ANSI_SQL`-only expression, or a metric SQL string
  `parse_metric_sql` cannot recognize) falls back to `Expr::Sql`, which SQL
  emits verbatim but Substrait refuses.
- **Aggregates are top-level only.** `Agg` is emitted through the aggregate
  relation; a nested `Agg` inside another expression errors in both the planner
  and `emit_expr`.
- The Substrait emitter declares a **fixed** function set across five extension
  URNs (`functions_aggregate_generic`, `functions_comparison`,
  `functions_boolean`, `functions_arithmetic`, `functions_datetime`):
  `sum, avg, count, min, max, equal, not_equal, and, or, subtract,
  current_date`. Adding an operator means wiring a new anchor in
  [`src/emit/substrait.rs`](../src/emit/substrait.rs).

## Known limitations

The dialect is deliberately narrow. Not currently supported:

- Arithmetic beyond `sub` (no add / multiply / divide).
- Comparisons beyond `eq` / `ne` (no `<`, `>`, `>=`, `<=`).
- First-class `or` / `not` nodes (`in` synthesizes `or` internally, but it is
  not exposed as a node).
- `like`, string functions, or date functions beyond `current_date`.
- `in` over expressions or sub-queries (literals only).

When a needed construct is missing, the escape hatch is an `ANSI_SQL` dialect
entry — but that produces an `Expr::Sql` and therefore **SQL-only** output (no
Substrait plan).

## Worked example

See [`tests/fixtures/mnml_expressions.yaml`](../tests/fixtures/mnml_expressions.yaml)
for a complete model: a `starter_flag` `CASE` field and a `starters` metric that
sums it (`SUM(CASE WHEN is_starter = 1 THEN 1 ELSE 0 END)`), exercised end to end
(MNML → `Expr` → Substrait + SQL) in `tests/query_plan.rs`.
