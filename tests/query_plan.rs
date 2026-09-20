//! Bind + logical plan tests.

use ossie_substrait::emit::sql::to_sql;
#[cfg(feature = "substrait")]
use ossie_substrait::emit::substrait::to_plan;
use ossie_substrait::compiled::CompiledModel;
use ossie_substrait::model::OssieDocument;
use ossie_substrait::parser::from_yaml_str;
use ossie_substrait::catalog::Func;
use ossie_substrait::plan::expr::{Expr, Literal};
use ossie_substrait::plan::{LogicalPlan, NamedAggregate};
use ossie_substrait::planner::build_logical_plan;
use ossie_substrait::query::{FilterOperator, FilterSpec, SemanticQuery};
use ossie_substrait::resolver::{QueryError, bind_query};

fn minimal_doc() -> OssieDocument {
    let yaml = include_str!("fixtures/minimal.yaml");
    from_yaml_str(yaml).expect("parse fixture")
}

fn minimal_model() -> CompiledModel {
    CompiledModel::compile(&minimal_doc()).expect("compile minimal")
}

#[test]
fn bind_group_by_only() {
    let model = minimal_model();
    let spec = SemanticQuery {
        group_by: vec!["fact.id".into()],
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    assert_eq!(bound.root_dataset, "fact");
    assert_eq!(bound.group_by.len(), 1);
    assert!(bound.metrics.is_empty());
}

#[test]
fn bind_metrics_only_errors_without_dataset() {
    let model = minimal_model();
    let err = bind_query(&model, &SemanticQuery {
            metrics: vec!["row_count".into()],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::MissingBaseDataset));
}

#[test]
fn bind_metrics_only_with_dataset() {
    let model = minimal_model();
    let spec = SemanticQuery {
        metrics: vec!["row_count".into()],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    assert_eq!(bound.root_dataset, "fact");
}

#[test]
fn logical_plan_shape() {
    let model = minimal_model();
    let spec = SemanticQuery {
        group_by: vec!["fact.id".into()],
        metrics: vec!["row_count".into()],
        filters: vec![FilterSpec::new("fact.id", serde_json::json!(1))],
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");

    let LogicalPlan::Aggregate {
        input,
        group_by,
        aggregates,
    } = plan
    else {
        panic!("expected aggregate root");
    };
    assert_eq!(group_by, vec![Expr::column("", "id")]);
    assert_eq!(
        aggregates,
        vec![NamedAggregate {
            name: "row_count".into(),
            expr: Expr::Agg {
                func: Func::Count,
                distinct: false,
                args: vec![],
            },
        }]
    );
    let LogicalPlan::Filter {
        input: filter_input,
        predicate,
    } = *input
    else {
        panic!("expected filter");
    };
    assert_eq!(
        predicate,
        Expr::binary(
            Func::Equal,
            Expr::column("", "id"),
            Expr::Literal(Literal::Int64(1)),
        )
    );
    let LogicalPlan::Scan {
        source,
        dataset,
        columns,
    } = *filter_input
    else {
        panic!("expected scan");
    };
    assert_eq!(source, "warehouse.public.fact_table");
    assert_eq!(dataset, "fact");
    assert!(columns.iter().any(|c| c.name == "id"));
}

#[test]
fn eq_field_filter_sql() {
    let model = minimal_model();
    let spec = SemanticQuery {
        group_by: vec!["fact.id".into()],
        metrics: vec!["row_count".into()],
        filters: vec![FilterSpec {
            field: "fact.day".into(),
            operator: Some(FilterOperator::EqField),
            value: serde_json::json!("fact.month_days"),
        }],
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let sql = to_sql(&plan).expect("sql");
    assert!(
        sql.contains("day = month_days"),
        "expected column equality in SQL, got: {sql}"
    );
}

#[cfg(feature = "substrait")]
#[test]
fn minimal_substrait_plan() {
    let model = minimal_model();
    let spec = SemanticQuery {
        group_by: vec!["fact.id".into()],
        metrics: vec!["row_count".into()],
        filters: vec![FilterSpec::new("fact.id", serde_json::json!(1))],
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let p = to_plan(&plan).expect("substrait plan");
    assert_eq!(p.relations.len(), 1);
}

fn steelwheels_model() -> CompiledModel {
    CompiledModel::compile(&steelwheels_doc()).expect("compile steelwheels")
}

fn steelwheels_doc() -> OssieDocument {
    let yaml = include_str!("fixtures/steelwheels.yaml");
    from_yaml_str(yaml).expect("parse steelwheels")
}

#[test]
fn steelwheels_join_orderfact_and_dates() {
    let doc = steelwheels_doc();
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        group_by: vec!["dates.year_id".into()],
        metrics: vec!["sales".into()],
        dataset: Some("orderfact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    assert_eq!(bound.root_dataset, "orderfact");
    assert_eq!(bound.joins.len(), 1);
    assert_eq!(bound.joins[0].right_dataset, "dates");
    assert!(bound.joins[0].on[0].left_column.contains("time_id"));

    let plan = build_logical_plan(&bound, &model).expect("plan");
    let s = format!("{}", plan.display_indent());
    assert!(s.contains("Join:"), "{s}");
    assert!(s.contains("orderfact") || s.contains("steelwheels.orderfact"), "{s}");
    assert!(s.contains("dates") || s.contains("steelwheels.dates"), "{s}");

    let LogicalPlan::Aggregate { group_by, .. } = plan else {
        panic!("expected aggregate root");
    };
    assert_eq!(
        group_by,
        vec![Expr::column("dates", "year_id")]
    );
}

#[test]
fn disconnected_datasets_error() {
    let yaml = include_str!("fixtures/disconnected.yaml");
    let doc: OssieDocument = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let err = bind_query(&model, &SemanticQuery {
            group_by: vec!["alpha.id".into(), "beta.id".into()],
            dataset: Some("alpha".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::DisconnectedDatasets(_)), "{err:?}");
}

#[test]
fn multi_dataset_requires_root_dataset() {
    let model = steelwheels_model();
    let err = bind_query(&model, &SemanticQuery {
            group_by: vec!["orderfact.order_status".into(), "dates.year_id".into()],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::MissingRootDataset), "{err:?}");
}

#[test]
fn people_daily_tenant_model_validates_and_plans() {
    use ossie_substrait::validate::validate;
    let yaml = include_str!("fixtures/people_daily_tenant.yaml");
    let doc = from_yaml_str(yaml).expect("parse");
    validate(&doc).expect("validate");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["starters".into(), "turnover".into()],
        group_by: vec![
            "people_daily.year_mm".into(),
            "people_daily.month_desc".into(),
            "people_daily.employment_type_name".into(),
        ],
        dataset: Some("people_daily".into()),
        filters: vec![FilterSpec::new("people_daily.year", serde_json::json!([2024, 2025]))],
    };
    let bound = bind_query(&model, &spec).expect("bind");
    build_logical_plan(&bound, &model).expect("plan");
    let p = to_plan(&build_logical_plan(&bound, &model).unwrap()).expect("substrait");
    assert!(!p.relations.is_empty());
}

#[test]
fn people_daily_distinct_and_tenure_metrics() {
    use ossie_substrait::validate::validate;
    let yaml = include_str!("fixtures/people_daily_tenant.yaml");
    let doc = from_yaml_str(yaml).expect("parse");
    validate(&doc).expect("validate");
    let model = CompiledModel::compile(&doc).expect("compile");
    for name in ["num_days", "num_employees", "tenure"] {
        let spec = SemanticQuery {
            metrics: vec![name.into()],
            group_by: vec!["people_daily.year".into()],
            dataset: Some("people_daily".into()),
            ..Default::default()
        };
        let bound = bind_query(&model, &spec).expect("bind");
        let plan = build_logical_plan(&bound, &model).expect("plan");
        let sql = to_sql(&plan).expect("sql");
        if name == "tenure" {
            assert!(sql.contains("CURRENT_DATE"), "{sql}");
            assert!(sql.contains("employment_start_date"), "{sql}");
            let bq = ossie_substrait::emit::sql::to_sql_with(
                &plan,
                &ossie_substrait::sql_dialect::BigQuery,
            )
            .expect("bigquery sql");
            assert!(
                bq.contains("DATE_DIFF(CURRENT_DATE(), employment_start_date, DAY)"),
                "{bq}"
            );
        } else {
            assert!(sql.contains("COUNT(DISTINCT"), "{sql}");
        }
        to_plan(&plan).expect("substrait");
    }
}

#[test]
fn case_field_aggregates_into_a_metric() {
    let yaml = include_str!("fixtures/expressions.yaml");
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        group_by: vec!["fact.id".into()],
        metrics: vec!["starters".into()],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let sql = to_sql(&plan).expect("sql");
    assert!(
        sql.contains("CASE") && sql.contains("is_starter") && sql.contains("starters"),
        "expected a CASE aggregate, got: {sql}"
    );
    #[cfg(feature = "substrait")]
    {
        let p = to_plan(&plan).expect("substrait");
        assert_eq!(p.relations.len(), 1);
    }
}

#[test]
fn count_star_metric_is_structured() {
    let yaml = include_str!("fixtures/expressions.yaml");
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["row_count".into()],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let LogicalPlan::Aggregate { aggregates, .. } = plan else {
        panic!("aggregate root");
    };
    assert!(matches!(
        aggregates[0].expr,
        Expr::Agg {
            func: Func::Count,
            ..
        }
    ));
}

#[test]
fn semantic_query_accepts_group_by_camel_case_json() {
    let json = r#"{
        "groupBy": ["fact.id"],
        "metrics": [],
        "filters": [],
        "dataset": null
    }"#;
    let q: SemanticQuery = serde_json::from_str(json).expect("deserialize groupBy");
    assert_eq!(q.group_by, vec!["fact.id".to_string()]);
}

// ---------------------------------------------------------------------------
// Ossie SQL expressions (Phase 2a): composite metrics, computed fields.
// ---------------------------------------------------------------------------

fn steelwheels_plan(metrics: &[&str], group_by: &[&str]) -> LogicalPlan {
    let doc = steelwheels_doc();
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        group_by: group_by.iter().map(|s| (*s).to_string()).collect(),
        metrics: metrics.iter().map(|s| (*s).to_string()).collect(),
        dataset: Some("orderfact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    build_logical_plan(&bound, &model).expect("plan")
}

/// `SUM(orderfact.quantityordered * orderfact.priceeach)` — an aggregate over
/// arithmetic. The old prefix-matching parser produced `Expr::Sql` for this.
#[test]
fn aggregate_over_arithmetic_is_structured() {
    let plan = steelwheels_plan(&["revenue"], &[]);
    let LogicalPlan::Aggregate { aggregates, .. } = &plan else {
        panic!("expected aggregate root");
    };
    let Expr::Agg { func, args, .. } = &aggregates[0].expr else {
        panic!("expected a structured aggregate, got {:?}", aggregates[0].expr);
    };
    assert_eq!(*func, Func::Sum);
    assert!(
        matches!(&args[0], Expr::Call { func: Func::Multiply, .. }),
        "{:?}",
        args[0]
    );
    assert!(to_sql(&plan).expect("sql").contains("SUM((quantityordered * priceeach))"));
}

/// `SUM(CASE WHEN … END)` — an aggregate over a conditional.
#[test]
fn aggregate_over_case_is_structured() {
    let plan = steelwheels_plan(&["premium_sales"], &[]);
    let LogicalPlan::Aggregate { aggregates, .. } = &plan else {
        panic!("expected aggregate root");
    };
    let Expr::Agg { args, .. } = &aggregates[0].expr else {
        panic!("expected aggregate, got {:?}", aggregates[0].expr);
    };
    assert!(matches!(&args[0], Expr::Case { .. }), "{:?}", args[0]);
}

/// `SUM(a) / NULLIF(SUM(b), 0)` — the composite case that needs a projection
/// above the Substrait aggregate.
#[test]
fn composite_metric_plans_and_emits_sql() {
    let plan = steelwheels_plan(&["avg_unit_price"], &[]);
    let LogicalPlan::Aggregate { aggregates, .. } = &plan else {
        panic!("expected aggregate root");
    };
    assert!(
        matches!(&aggregates[0].expr, Expr::Call { func: Func::Divide, .. }),
        "expected a division at the metric root, got {:?}",
        aggregates[0].expr
    );
    // SQL needs no restructuring — a composite aggregate is legal in a SELECT list.
    let sql = to_sql(&plan).expect("sql");
    assert!(
        sql.contains("(SUM(totalprice) / NULLIF(SUM(quantityordered), 0)) AS avg_unit_price"),
        "{sql}"
    );
}

#[cfg(feature = "substrait")]
#[test]
fn composite_metric_emits_project_over_aggregate() {
    use substrait::proto::rel::RelType;

    let plan = steelwheels_plan(&["avg_unit_price"], &["dates.year_id"]);
    let substrait_plan = to_plan(&plan).expect("substrait");

    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };
    // Output is the grouping followed by the metric — not the intermediate measures.
    assert_eq!(root.names, vec!["dates.year_id", "avg_unit_price"]);

    let Some(RelType::Project(project)) = root.input.expect("input").rel_type else {
        panic!("expected a ProjectRel above the aggregate");
    };
    assert_eq!(project.expressions.len(), 1, "one expression per metric");

    let emit = match project.common.expect("common").emit_kind.expect("emit_kind") {
        substrait::proto::rel_common::EmitKind::Emit(e) => e.output_mapping,
        other => panic!("expected an explicit emit mapping, got {other:?}"),
    };
    // 1 grouping + 2 measures = 3 input columns; the metric lands at index 3.
    assert_eq!(emit, vec![0, 3]);

    let Some(RelType::Aggregate(agg)) = project.input.expect("project input").rel_type else {
        panic!("expected an AggregateRel under the projection");
    };
    assert_eq!(agg.measures.len(), 2, "SUM(totalprice) and SUM(quantityordered)");
}

/// A metric mentioning the same aggregate twice yields one measure, not two.
#[cfg(feature = "substrait")]
#[test]
fn repeated_aggregates_share_one_measure() {
    use substrait::proto::rel::RelType;

    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: t
    source: s.t
    fields:
      - name: v
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: v }]
metrics:
  - name: ratio
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(t.v) / NULLIF(SUM(t.v), 0)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["ratio".into()],
        dataset: Some("t".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let substrait_plan = to_plan(&plan).expect("substrait");

    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };
    let Some(RelType::Project(project)) = root.input.expect("input").rel_type else {
        panic!("expected a ProjectRel");
    };
    let Some(RelType::Aggregate(agg)) = project.input.expect("input").rel_type else {
        panic!("expected an AggregateRel");
    };
    assert_eq!(agg.measures.len(), 1, "SUM(t.v) should be emitted once");
}

/// A bare aggregate must still produce a plain AggregateRel with no projection.
#[cfg(feature = "substrait")]
#[test]
fn simple_metric_emits_no_projection() {
    use substrait::proto::rel::RelType;

    let plan = steelwheels_plan(&["sales"], &[]);
    let substrait_plan = to_plan(&plan).expect("substrait");
    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };
    assert!(
        matches!(root.input.expect("input").rel_type, Some(RelType::Aggregate(_))),
        "a bare aggregate needs no projection"
    );
}

#[test]
fn unsupported_expressions_are_reported_at_compile_time() {
    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: t
    source: s.t
    fields:
      - name: v
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: v }]
metrics:
  - name: windowed
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "RANK() OVER (ORDER BY t.v)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");

    // Compiling surfaces it immediately, naming the metric...
    let model = CompiledModel::compile(&doc).expect("the model still compiles");
    assert_eq!(model.issues.len(), 1);
    assert!(model.issues[0].target.contains("windowed"), "{:?}", model.issues[0]);
    assert!(model.issues[0].message.contains("window function"));

    // ...and querying it repeats the reason rather than claiming it is missing.
    let spec = SemanticQuery {
        metrics: vec!["windowed".into()],
        dataset: Some("t".into()),
        ..Default::default()
    };
    let err = bind_query(&model, &spec).expect_err("the metric is unusable");
    let msg = err.to_string();
    assert!(msg.contains("window function"), "{msg}");
    assert!(!msg.contains("not found"), "should not claim it is missing: {msg}");

    // Strict callers can refuse the whole model.
    assert!(CompiledModel::compile_strict(&doc).is_err());
}

/// Every bad expression is reported together, and the rest stays usable.
#[test]
fn compile_reports_all_expression_errors_at_once() {
    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: t
    source: s.t
    fields:
      - name: ok
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: ok }]
      - name: bad_field
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: "SUM(ok)" }]
metrics:
  - name: bad_metric_a
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "RANK() OVER (ORDER BY t.ok)" }]
  - name: bad_metric_b
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(t.nonexistent)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");

    let targets: Vec<&str> = model.issues.iter().map(|i| i.target.as_str()).collect();
    assert_eq!(targets.len(), 3, "{targets:?}");
    assert!(targets.iter().any(|t| t.contains("bad_field")), "{targets:?}");
    assert!(targets.iter().any(|t| t.contains("bad_metric_a")), "{targets:?}");
    assert!(targets.iter().any(|t| t.contains("bad_metric_b")), "{targets:?}");

    // The good field survives, so the rest of the model is still queryable.
    assert!(model.dataset("t").expect("dataset").field("ok").is_some());
}

/// A cross-dataset metric pulls in its own join: nothing is grouped or
/// filtered on `dates`, but the metric expression references it.
#[test]
fn cross_dataset_metric_discovers_its_join() {
    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: orders
    source: s.orders
    fields:
      - name: amount
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: customer_id
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: customer_id }]
  - name: customers
    source: s.customers
    primary_key: [id]
    fields:
      - name: id
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: id }]
relationships:
  - name: orders_to_customers
    from: orders
    to: customers
    from_columns: [customer_id]
    to_columns: [id]
metrics:
  - name: revenue_per_customer
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: "SUM(orders.amount) / COUNT(DISTINCT customers.id)"
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["revenue_per_customer".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    assert_eq!(bound.joins.len(), 1, "metric should require the customers join");
    assert_eq!(bound.joins[0].right_dataset, "customers");

    let plan = build_logical_plan(&bound, &model).expect("plan");
    let sql = to_sql(&plan).expect("sql");
    assert!(sql.contains("INNER JOIN"), "{sql}");
    assert!(
        sql.contains("(SUM(orders.amount) / COUNT(DISTINCT customers.id))"),
        "{sql}"
    );

    #[cfg(feature = "substrait")]
    to_plan(&plan).expect("substrait");
}

// ---------------------------------------------------------------------------
// Filter operators (P0 fix 3): typed, exhaustive, no silent fallback.
// ---------------------------------------------------------------------------

fn minimal_filter_sql(op: Option<FilterOperator>, value: serde_json::Value) -> String {
    let model = minimal_model();
    let spec = SemanticQuery {
        metrics: vec!["row_count".into()],
        filters: vec![FilterSpec {
            field: "fact.id".into(),
            operator: op,
            value,
        }],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    to_sql(&plan).expect("sql")
}

#[test]
fn comparison_operators_reach_the_plan() {
    for (op, sql) in [
        (FilterOperator::Eq, "id = 1"),
        (FilterOperator::Ne, "id <> 1"),
        (FilterOperator::Lt, "id < 1"),
        (FilterOperator::Lte, "id <= 1"),
        (FilterOperator::Gt, "id > 1"),
        (FilterOperator::Gte, "id >= 1"),
    ] {
        let out = minimal_filter_sql(Some(op), serde_json::json!(1));
        assert!(out.contains(sql), "{op:?} -> {out}");
    }
}

#[test]
fn set_and_null_operators_reach_the_plan() {
    let out = minimal_filter_sql(Some(FilterOperator::In), serde_json::json!([1, 2]));
    assert!(out.contains("id IN (1, 2)"), "{out}");

    let out = minimal_filter_sql(Some(FilterOperator::NotIn), serde_json::json!([1, 2]));
    assert!(out.contains("NOT id IN (1, 2)"), "{out}");

    let out = minimal_filter_sql(Some(FilterOperator::IsNull), serde_json::Value::Null);
    assert!(out.contains("id IS NULL"), "{out}");

    let out = minimal_filter_sql(Some(FilterOperator::IsNotNull), serde_json::Value::Null);
    assert!(out.contains("id IS NOT NULL"), "{out}");
}

#[test]
fn omitted_operator_defaults_on_value_shape() {
    assert!(minimal_filter_sql(None, serde_json::json!(1)).contains("id = 1"));
    assert!(minimal_filter_sql(None, serde_json::json!([1, 2])).contains("id IN (1, 2)"));
}

/// The bug this fix exists for: an unrecognized operator used to become `=`.
#[test]
fn unknown_operators_are_rejected_at_deserialization() {
    let json = r#"{"metrics":["row_count"],"dataset":"fact",
                   "filters":[{"field":"fact.id","operator":"approximately","value":1}]}"#;
    let err = serde_json::from_str::<SemanticQuery>(json).expect_err("must not deserialize");
    assert!(err.to_string().contains("unknown variant"), "{err}");
}

#[test]
fn symbol_aliases_are_accepted() {
    for (spelling, expected) in [
        ("=", FilterOperator::Eq),
        ("!=", FilterOperator::Ne),
        ("<>", FilterOperator::Ne),
        (">=", FilterOperator::Gte),
        ("eq_column", FilterOperator::EqField),
    ] {
        let json = format!(r#"{{"field":"fact.id","operator":"{spelling}","value":1}}"#);
        let f: FilterSpec = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{spelling}: {e}"));
        assert_eq!(f.operator, Some(expected), "{spelling}");
    }
}

#[test]
fn set_operators_require_an_array_value() {
    let model = minimal_model();
    let spec = SemanticQuery {
        metrics: vec!["row_count".into()],
        filters: vec![FilterSpec {
            field: "fact.id".into(),
            operator: Some(FilterOperator::In),
            value: serde_json::json!(1),
        }],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let err = build_logical_plan(&bound, &model).expect_err("scalar value for IN");
    assert!(err.to_string().contains("expects an array"), "{err}");
}

// ---------------------------------------------------------------------------
// Dataset.source (P0 fix 2): one meaning across both emitters.
// ---------------------------------------------------------------------------

/// `source` is a dot-separated qualified name, so the SQL and Substrait
/// emitters must decompose it the same way.
#[cfg(feature = "substrait")]
#[test]
fn both_emitters_read_source_as_a_qualified_name() {
    use substrait::proto::{read_rel::ReadType, rel::RelType};

    let model = minimal_model();
    let spec = SemanticQuery {
        metrics: vec!["row_count".into()],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");

    // SQL: three identifiers joined by dots, not one identifier containing dots.
    let sql = to_sql(&plan).expect("sql");
    assert!(sql.contains("FROM warehouse.public.fact_table"), "{sql}");
    assert!(!sql.contains("\"warehouse.public.fact_table\""), "{sql}");

    // Substrait: the same three parts.
    let substrait_plan = to_plan(&plan).expect("substrait");
    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };
    let names = find_table_names(&root.input.expect("input")).expect("a read rel");
    assert_eq!(names, vec!["warehouse", "public", "fact_table"]);

    fn find_table_names(rel: &substrait::proto::Rel) -> Option<Vec<String>> {
        match rel.rel_type.as_ref()? {
            RelType::Read(r) => match r.read_type.as_ref()? {
                ReadType::NamedTable(t) => Some(t.names.clone()),
                _ => None,
            },
            RelType::Aggregate(a) => find_table_names(a.input.as_ref()?),
            RelType::Filter(f) => find_table_names(f.input.as_ref()?),
            RelType::Project(p) => find_table_names(p.input.as_ref()?),
            RelType::Join(j) => find_table_names(j.left.as_ref()?),
            _ => None,
        }
    }
}

/// String literals must be escaped for the target dialect, not just ANSI.
#[test]
fn string_literals_are_escaped_per_dialect() {
    let model = minimal_model();
    let spec = SemanticQuery {
        metrics: vec!["row_count".into()],
        filters: vec![FilterSpec::new("fact.id", serde_json::json!("O'Brien\\"))],
        dataset: Some("fact".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");

    // ANSI: double the single quote; backslash is an ordinary character.
    assert!(to_sql(&plan).expect("sql").contains("'O''Brien\\'"));

    // BigQuery: backslash is an escape character, so it must be escaped too —
    // otherwise the trailing backslash would escape the closing quote.
    let bq = ossie_substrait::emit::sql::to_sql_with(
        &plan,
        &ossie_substrait::sql_dialect::BigQuery,
    )
    .expect("bigquery sql");
    assert!(bq.contains("'O\\'Brien\\\\'"), "{bq}");
}

// ---------------------------------------------------------------------------
// Scan schema types (P0 fix 1): declared DataType reaches the Substrait schema.
// ---------------------------------------------------------------------------

#[cfg(feature = "substrait")]
#[test]
fn scan_schema_carries_declared_types() {
    use substrait::proto::r#type::Kind;
    use substrait::proto::{read_rel::ReadType, rel::RelType};

    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: t
    source: s.t
    fields:
      - name: name
        datatype: String
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: name }]
      - name: qty
        datatype: Integer
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: qty }]
      - name: ratio
        datatype: Float
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: ratio }]
      - name: active
        datatype: Boolean
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: active }]
      - name: d
        datatype: Date
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: d }]
      - name: ts
        datatype: DateTimeTz
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: ts }]
      - name: amount
        datatype: Decimal
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: untyped
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: untyped }]
metrics:
  - name: n
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "COUNT(*)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["n".into()],
        dataset: Some("t".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let substrait_plan = to_plan(&plan).expect("substrait");

    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };
    let Some(RelType::Aggregate(agg)) = root.input.expect("input").rel_type else {
        panic!("expected an aggregate");
    };
    let Some(RelType::Read(read)) = agg.input.expect("agg input").rel_type else {
        panic!("expected a read");
    };
    assert!(matches!(read.read_type, Some(ReadType::NamedTable(_))));

    let schema = read.base_schema.expect("base_schema");
    let types = schema.r#struct.expect("struct").types;
    let by_name: Vec<(String, Kind)> = schema
        .names
        .iter()
        .cloned()
        .zip(types.into_iter().map(|t| t.kind.expect("kind")))
        .collect();

    let kind = |n: &str| by_name.iter().find(|(name, _)| name == n).expect(n).1.clone();

    assert!(matches!(kind("name"), Kind::String(_)));
    assert!(matches!(kind("qty"), Kind::I64(_)));
    assert!(matches!(kind("ratio"), Kind::Fp64(_)));
    assert!(matches!(kind("active"), Kind::Bool(_)));
    assert!(matches!(kind("d"), Kind::Date(_)));
    assert!(matches!(kind("ts"), Kind::PrecisionTimestampTz(_)));

    // Under-specified in Ossie, so `unknown` rather than an invented parameterization.
    assert!(matches!(kind("amount"), Kind::UserDefined(_)), "Decimal");
    assert!(matches!(kind("untyped"), Kind::UserDefined(_)), "no datatype");

    // The `unknown` type must be declared for those references to resolve.
    assert!(
        substrait_plan
            .extension_urns
            .iter()
            .any(|u| u.urn == "extension:io.substrait:unknown"),
        "unknown type extension URN missing"
    );
}

/// The regression this fix exists for: columns were all typed String.
#[cfg(feature = "substrait")]
#[test]
fn scan_schema_does_not_default_everything_to_string() {
    use substrait::proto::r#type::Kind;
    use substrait::proto::rel::RelType;

    let plan = steelwheels_plan(&["sales"], &[]);
    let substrait_plan = to_plan(&plan).expect("substrait");
    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };
    let Some(RelType::Aggregate(agg)) = root.input.expect("input").rel_type else {
        panic!("expected an aggregate");
    };
    let Some(RelType::Read(read)) = agg.input.expect("agg input").rel_type else {
        panic!("expected a read");
    };
    let types = read
        .base_schema
        .expect("base_schema")
        .r#struct
        .expect("struct")
        .types;
    // The steelwheels fixture declares no datatypes, so nothing should claim String.
    assert!(
        types
            .iter()
            .all(|t| matches!(t.kind, Some(Kind::UserDefined(_)))),
        "undeclared columns must be `unknown`, not String"
    );
}
