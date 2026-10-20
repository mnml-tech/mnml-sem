//! A real Substrait consumer executes the plans this crate emits.
//!
//! The structural tests in `query_plan.rs` check the shape of a plan. These
//! check that an engine accepts it and computes the right numbers: DataFusion
//! decodes the plan from Protobuf bytes, binds it to in-memory tables, and runs
//! it. The same query also runs as the SQL `ossie-compiler` emits, and both
//! must agree with the expected rows.
//!
//! DataFusion is a dev-dependency only, so it never reaches consumers.

use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::catalog::{MemoryCatalogProvider, MemorySchemaProvider};
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use datafusion_substrait::logical_plan::consumer::from_substrait_plan;
use prost::Message;
use substrait::proto::Plan;

use ossie_compiler::compiled::CompiledModel;
use ossie_compiler::emit::sql::to_sql_with;
use ossie_compiler::plan::LogicalPlan;
use ossie_compiler::planner::build_logical_plan;
use ossie_compiler::query::{FilterOperator, FilterSpec, OrderBy, SemanticQuery};
use ossie_compiler::resolver::bind_query;
use ossie_compiler::sql_dialect::{Ansi, SqlDialect};
use ossie_model::parser::from_yaml_str;
use ossie_substrait::{encode_plan, to_plan};

fn plan(model_yaml: &str, query: &SemanticQuery) -> LogicalPlan {
    let doc = from_yaml_str(model_yaml).expect("parse model");
    let model = CompiledModel::compile(&doc).expect("compile model");
    assert!(model.issues.is_empty(), "{:?}", model.issues);
    let bound = bind_query(&model, query).expect("bind");
    build_logical_plan(&bound, &model).expect("plan")
}

/// Run the plan through Substrait: encode, decode, consume, execute.
async fn run_substrait(ctx: &SessionContext, plan: &LogicalPlan) -> Vec<String> {
    let bytes = encode_plan(&to_plan(plan).expect("emit substrait"));
    let decoded = Plan::decode(bytes.as_slice()).expect("decode substrait");
    let logical = from_substrait_plan(&ctx.state(), &decoded)
        .await
        .expect("DataFusion consumes the plan");
    let batches = ctx
        .execute_logical_plan(logical)
        .await
        .expect("plan is executable")
        .collect()
        .await
        .expect("execution produces rows");
    rows(&batches)
}

/// ANSI SQL, except for row limits: DataFusion's parser does not support the
/// standard `FETCH NEXT n ROWS ONLY` yet, only `LIMIT`/`OFFSET`.
struct DataFusionSql;

impl SqlDialect for DataFusionSql {
    fn quote_ident(&self, ident: &str) -> String {
        Ansi.quote_ident(ident)
    }

    fn quote_source(&self, path: &str) -> String {
        Ansi.quote_source(path)
    }

    fn render_fetch(&self, offset: u64, limit: Option<u64>) -> String {
        match (limit, offset) {
            (None, 0) => String::new(),
            (None, n) => format!("OFFSET {n}"),
            (Some(l), 0) => format!("LIMIT {l}"),
            (Some(l), n) => format!("LIMIT {l} OFFSET {n}"),
        }
    }
}

/// Run the plan as the SQL the compiler emits.
async fn run_sql(ctx: &SessionContext, plan: &LogicalPlan) -> Vec<String> {
    let sql = to_sql_with(plan, &DataFusionSql).expect("emit sql");
    let batches = ctx
        .sql(&sql)
        .await
        .unwrap_or_else(|e| panic!("DataFusion plans the SQL: {e}\n{sql}"))
        .collect()
        .await
        .unwrap_or_else(|e| panic!("the SQL executes: {e}\n{sql}"));
    rows(&batches)
}

/// Both paths, which must agree with each other and with `expected`.
async fn assert_rows(ctx: &SessionContext, plan: &LogicalPlan, expected: &[&str]) {
    let substrait = run_substrait(ctx, plan).await;
    let sql = run_sql(ctx, plan).await;
    assert_eq!(substrait, expected, "Substrait result");
    assert_eq!(sql, expected, "SQL result");
}

/// Each row as `a | b | c`, in the order the engine returned them.
fn rows(batches: &[RecordBatch]) -> Vec<String> {
    let mut out = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let cells: Vec<String> = batch
                .columns()
                .iter()
                .map(|c| array_value_to_string(c, row).expect("format cell"))
                .collect();
            out.push(cells.join(" | "));
        }
    }
    out
}

/// Register a table under a dotted `source` name, creating its catalog and
/// schema as needed: `table`, `schema.table`, or `catalog.schema.table`.
fn register(ctx: &SessionContext, source: &str, columns: Vec<(&str, ArrayRef)>) {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, array)| Field::new(*name, array.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        schema.clone(),
        columns.into_iter().map(|(_, array)| array).collect(),
    )
    .expect("fixture batch");
    let table = Arc::new(MemTable::try_new(schema, vec![vec![batch]]).expect("mem table"));

    let parts: Vec<&str> = source.split('.').collect();
    let (catalog, schema_name) = match parts.as_slice() {
        [_] => (None, None),
        [s, _] => (None, Some(*s)),
        [c, s, _] => (Some(*c), Some(*s)),
        _ => panic!("unsupported source {source}"),
    };
    if let Some(schema_name) = schema_name {
        let catalog_name = catalog.unwrap_or("datafusion");
        let provider = ctx.catalog(catalog_name).unwrap_or_else(|| {
            let fresh = Arc::new(MemoryCatalogProvider::new());
            ctx.register_catalog(catalog_name, fresh.clone());
            fresh
        });
        if provider.schema(schema_name).is_none() {
            provider
                .register_schema(schema_name, Arc::new(MemorySchemaProvider::new()))
                .expect("register schema");
        }
    }
    ctx.register_table(source, table).expect("register table");
}

fn bools(values: &[bool]) -> ArrayRef {
    Arc::new(BooleanArray::from(values.to_vec()))
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn ints(values: &[i64]) -> ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

fn floats(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

// ---------------------------------------------------------------------------
// A typed model: every field declares its datatype, so the plan's read schemas
// carry concrete types DataFusion can bind to.
// ---------------------------------------------------------------------------

const SHOP: &str = r#"
version: "0.2.0.dev0"
name: shop
datasets:
  - name: orders
    source: sales.public.orders
    primary_key: [order_id]
    fields:
      - name: order_id
        datatype: Integer
        expression: { dialects: [{ dialect: ANSI_SQL, expression: order_id }] }
      - name: customer_id
        datatype: Integer
        expression: { dialects: [{ dialect: ANSI_SQL, expression: customer_id }] }
      - name: status
        datatype: String
        expression: { dialects: [{ dialect: ANSI_SQL, expression: status }] }
      - name: amount
        datatype: Float
        expression: { dialects: [{ dialect: ANSI_SQL, expression: amount }] }
      - name: discount
        datatype: Float
        expression: { dialects: [{ dialect: ANSI_SQL, expression: discount }] }
      - name: quantity
        datatype: Integer
        expression: { dialects: [{ dialect: ANSI_SQL, expression: quantity }] }
      - name: net_amount
        datatype: Float
        expression: { dialects: [{ dialect: ANSI_SQL, expression: amount - discount }] }
    metrics:
      - name: total_net
        expression: { dialects: [{ dialect: ANSI_SQL, expression: SUM(net_amount) }] }
  - name: customers
    source: sales.public.customers
    primary_key: [id]
    fields:
      - name: id
        datatype: Integer
        expression: { dialects: [{ dialect: ANSI_SQL, expression: id }] }
      - name: region
        datatype: String
        expression: { dialects: [{ dialect: ANSI_SQL, expression: region }] }
    metrics:
      - name: customer_count
        expression: { dialects: [{ dialect: ANSI_SQL, expression: COUNT(DISTINCT id) }] }
relationships:
  - name: orders_to_customers
    from: orders
    to: customers
    from_columns: [customer_id]
    to_columns: [id]
metrics:
  - name: revenue
    expression: { dialects: [{ dialect: ANSI_SQL, expression: SUM(orders.amount) }] }
  - name: order_count
    expression: { dialects: [{ dialect: ANSI_SQL, expression: COUNT(*) }] }
  - name: avg_price
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: SUM(orders.amount) / NULLIF(SUM(orders.quantity), 0)
  - name: large_order_revenue
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: SUM(CASE WHEN orders.amount >= 100 THEN orders.amount ELSE 0 END)
  - name: revenue_per_customer
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: SUM(orders.amount) / COUNT(DISTINCT customers.id)
  - name: net_per_customer
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: orders.total_net / customers.customer_count
"#;

/// Five orders across three customers in two regions.
fn shop() -> SessionContext {
    let ctx = SessionContext::new();
    register(
        &ctx,
        "sales.public.orders",
        vec![
            ("order_id", ints(&[1, 2, 3, 4, 5])),
            ("customer_id", ints(&[10, 10, 20, 30, 30])),
            (
                "status",
                strings(&["paid", "paid", "paid", "pending", "refunded"]),
            ),
            ("amount", floats(&[100.0, 50.0, 200.0, 30.0, 20.0])),
            ("discount", floats(&[10.0, 0.0, 20.0, 0.0, 5.0])),
            ("quantity", ints(&[2, 1, 4, 3, 1])),
        ],
    );
    register(
        &ctx,
        "sales.public.customers",
        vec![
            ("id", ints(&[10, 20, 30])),
            ("region", strings(&["EMEA", "EMEA", "NA"])),
        ],
    );
    ctx
}

fn shop_plan(query: SemanticQuery) -> LogicalPlan {
    plan(SHOP, &query)
}

#[tokio::test]
async fn a_metric_over_one_dataset_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["revenue".into(), "order_count".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::asc("orders.status")],
        ..Default::default()
    });
    assert_rows(
        &shop(),
        &plan,
        &[
            "paid | 350.0 | 3",
            "pending | 30.0 | 1",
            "refunded | 20.0 | 1",
        ],
    )
    .await;
}

#[tokio::test]
async fn a_metric_without_grouping_executes() {
    let plan = shop_plan(SemanticQuery {
        metrics: vec!["revenue".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    });
    assert_rows(&shop(), &plan, &["400.0"]).await;
}

#[tokio::test]
async fn grouping_through_a_join_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["customers.region".into()],
        metrics: vec!["revenue".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::asc("customers.region")],
        ..Default::default()
    });
    assert_rows(&shop(), &plan, &["EMEA | 350.0", "NA | 50.0"]).await;
}

#[tokio::test]
async fn a_composite_metric_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["avg_price".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::asc("orders.status")],
        ..Default::default()
    });
    // paid: 350 / 7; pending: 30 / 3; refunded: 20 / 1.
    assert_rows(
        &shop(),
        &plan,
        &["paid | 50.0", "pending | 10.0", "refunded | 20.0"],
    )
    .await;
}

#[tokio::test]
async fn a_case_inside_an_aggregate_executes() {
    let plan = shop_plan(SemanticQuery {
        metrics: vec!["large_order_revenue".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    });
    assert_rows(&shop(), &plan, &["300.0"]).await;
}

#[tokio::test]
async fn a_metric_spanning_a_join_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["customers.region".into()],
        metrics: vec!["revenue_per_customer".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::asc("customers.region")],
        ..Default::default()
    });
    // EMEA: 350 over customers 10 and 20; NA: 50 over customer 30.
    assert_rows(&shop(), &plan, &["EMEA | 175.0", "NA | 50.0"]).await;
}

#[tokio::test]
async fn filters_execute() {
    let ctx = shop();
    let by_status = |filter: FilterSpec| {
        shop_plan(SemanticQuery {
            group_by: vec!["orders.status".into()],
            metrics: vec!["revenue".into()],
            dataset: Some("orders".into()),
            filters: vec![filter],
            order_by: vec![OrderBy::asc("orders.status")],
            ..Default::default()
        })
    };

    let eq = by_status(FilterSpec::new("orders.status", serde_json::json!("paid")));
    assert_rows(&ctx, &eq, &["paid | 350.0"]).await;

    let one_of = by_status(FilterSpec::new(
        "orders.status",
        serde_json::json!(["pending", "refunded"]),
    ));
    assert_rows(&ctx, &one_of, &["pending | 30.0", "refunded | 20.0"]).await;

    let mut at_least = FilterSpec::new("orders.amount", serde_json::json!(50));
    at_least.operator = Some(FilterOperator::Gte);
    assert_rows(&ctx, &by_status(at_least), &["paid | 350.0"]).await;
}

#[tokio::test]
async fn ordering_and_limit_execute() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["orders.customer_id".into()],
        metrics: vec!["revenue".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::desc("revenue")],
        limit: Some(2),
        ..Default::default()
    });
    assert_rows(&shop(), &plan, &["20 | 200.0", "10 | 150.0"]).await;
}

#[tokio::test]
async fn an_offset_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["orders.customer_id".into()],
        metrics: vec!["revenue".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::desc("revenue")],
        limit: Some(1),
        offset: Some(1),
        ..Default::default()
    });
    assert_rows(&shop(), &plan, &["10 | 150.0"]).await;
}

#[tokio::test]
async fn a_dataset_metric_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["orders.total_net".into()],
        order_by: vec![OrderBy::asc("orders.status")],
        ..Default::default()
    });
    // net_amount is a computed field: amount - discount.
    assert_rows(
        &shop(),
        &plan,
        &["paid | 320.0", "pending | 30.0", "refunded | 15.0"],
    )
    .await;
}

#[tokio::test]
async fn a_model_metric_built_on_dataset_metrics_executes() {
    let plan = shop_plan(SemanticQuery {
        group_by: vec!["customers.region".into()],
        metrics: vec!["net_per_customer".into()],
        dataset: Some("orders".into()),
        order_by: vec![OrderBy::asc("customers.region")],
        ..Default::default()
    });
    // EMEA: (90 + 50 + 180) / 2; NA: (30 + 15) / 1.
    assert_rows(&shop(), &plan, &["EMEA | 160.0", "NA | 45.0"]).await;
}

// ---------------------------------------------------------------------------
// Steel Wheels: a fact with two dimensions, every field typed.
// ---------------------------------------------------------------------------

const STEELWHEELS: &str = include_str!("../../ossie-model/tests/fixtures/steelwheels.yaml");

fn steelwheels() -> SessionContext {
    let ctx = SessionContext::new();
    register(
        &ctx,
        "steelwheels.orderfact",
        vec![
            ("time_id", ints(&[1, 1, 2, 3])),
            ("customernumber", ints(&[10, 20, 10, 20])),
            ("quantityordered", ints(&[2, 1, 4, 5])),
            ("totalprice", floats(&[200.0, 50.0, 480.0, 100.0])),
            ("priceeach", floats(&[100.0, 50.0, 120.0, 20.0])),
            ("is_premium_order", bools(&[false, false, true, false])),
            (
                "order_status",
                strings(&["Shipped", "Shipped", "Shipped", "Cancelled"]),
            ),
        ],
    );
    register(
        &ctx,
        "steelwheels.dates",
        vec![
            ("time_id", ints(&[1, 2, 3])),
            ("year_id", ints(&[2024, 2024, 2025])),
            ("qtr_id", ints(&[1, 2, 1])),
            ("month_name", strings(&["Jan", "Apr", "Jan"])),
        ],
    );
    register(
        &ctx,
        "steelwheels.customer_w_ter",
        vec![
            ("customernumber", ints(&[10, 20])),
            ("territory", strings(&["EMEA", "NA"])),
            ("country", strings(&["Sweden", "USA"])),
            ("state", strings(&["", "CA"])),
            ("city", strings(&["Stockholm", "San Rafael"])),
        ],
    );
    ctx
}

fn steelwheels_plan(query: SemanticQuery) -> LogicalPlan {
    plan(
        STEELWHEELS,
        &SemanticQuery {
            dataset: Some("orderfact".into()),
            ..query
        },
    )
}

#[tokio::test]
async fn steelwheels_sales_by_year_executes() {
    let plan = steelwheels_plan(SemanticQuery {
        group_by: vec!["dates.year_id".into()],
        metrics: vec!["sales".into(), "quantity".into()],
        order_by: vec![OrderBy::asc("dates.year_id")],
        ..Default::default()
    });
    assert_rows(
        &steelwheels(),
        &plan,
        &["2024 | 730.0 | 7", "2025 | 100.0 | 5"],
    )
    .await;
}

#[tokio::test]
async fn steelwheels_joins_both_dimensions() {
    let plan = steelwheels_plan(SemanticQuery {
        group_by: vec!["dates.year_id".into(), "customer_w_ter.territory".into()],
        metrics: vec!["sales".into()],
        order_by: vec![
            OrderBy::asc("dates.year_id"),
            OrderBy::asc("customer_w_ter.territory"),
        ],
        ..Default::default()
    });
    assert_rows(
        &steelwheels(),
        &plan,
        &[
            "2024 | EMEA | 680.0",
            "2024 | NA | 50.0",
            "2025 | NA | 100.0",
        ],
    )
    .await;
}

#[tokio::test]
async fn steelwheels_metrics_over_expressions_execute() {
    let plan = steelwheels_plan(SemanticQuery {
        group_by: vec!["dates.year_id".into()],
        metrics: vec![
            "revenue".into(),
            "premium_sales".into(),
            "avg_unit_price".into(),
        ],
        order_by: vec![OrderBy::asc("dates.year_id")],
        ..Default::default()
    });
    // 2024: 730 over 7 units, 480 of it on the one line priced over 100.
    assert_rows(
        &steelwheels(),
        &plan,
        &[
            "2024 | 730.0 | 480.0 | 104.28571428571429",
            "2025 | 100.0 | 0.0 | 20.0",
        ],
    )
    .await;
}

#[tokio::test]
async fn steelwheels_filters_on_a_joined_dimension_and_a_flag() {
    let ctx = steelwheels();

    let emea = steelwheels_plan(SemanticQuery {
        group_by: vec!["dates.year_id".into()],
        metrics: vec!["sales".into()],
        filters: vec![FilterSpec::new(
            "customer_w_ter.territory",
            serde_json::json!("EMEA"),
        )],
        order_by: vec![OrderBy::asc("dates.year_id")],
        ..Default::default()
    });
    assert_rows(&ctx, &emea, &["2024 | 680.0"]).await;

    let premium = steelwheels_plan(SemanticQuery {
        metrics: vec!["sales".into()],
        filters: vec![FilterSpec::new(
            "orderfact.is_premium_order",
            serde_json::json!(true),
        )],
        ..Default::default()
    });
    assert_rows(&ctx, &premium, &["480.0"]).await;
}

/// A field without a `datatype` is read as Substrait's `unknown` extension type
/// (ADR-0007), which DataFusion does not implement. The SQL path needs no
/// types and runs. This pins the gap: when it closes, this test fails.
#[tokio::test]
async fn untyped_fields_run_as_sql_but_not_as_substrait() {
    const UNTYPED: &str = r#"
version: "0.2.0.dev0"
name: untyped
datasets:
  - name: orders
    source: sales.public.orders
    fields:
      - name: status
        expression: { dialects: [{ dialect: ANSI_SQL, expression: status }] }
      - name: amount
        expression: { dialects: [{ dialect: ANSI_SQL, expression: amount }] }
metrics:
  - name: revenue
    expression: { dialects: [{ dialect: ANSI_SQL, expression: SUM(orders.amount) }] }
"#;
    let plan = plan(
        UNTYPED,
        &SemanticQuery {
            metrics: vec!["revenue".into()],
            dataset: Some("orders".into()),
            ..Default::default()
        },
    );
    let ctx = shop();
    assert_eq!(run_sql(&ctx, &plan).await, ["400.0"]);

    let decoded = Plan::decode(encode_plan(&to_plan(&plan).expect("emit")).as_slice())
        .expect("decode substrait");
    let err = from_substrait_plan(&ctx.state(), &decoded)
        .await
        .expect_err("DataFusion has no `unknown` type");
    assert!(
        err.to_string().contains("user defined type"),
        "unexpected error: {err}"
    );
}
