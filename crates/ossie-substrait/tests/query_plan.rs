//! Bind + logical plan tests.

use ossie_compiler::catalog::Func;
use ossie_compiler::compiled::{CompiledModel, IssueTarget};
use ossie_compiler::emit::sql::to_sql;
use ossie_compiler::plan::expr::{Expr, Literal};
use ossie_compiler::plan::{LogicalPlan, NamedExpr};
use ossie_compiler::planner::build_logical_plan;
use ossie_compiler::query::{
    Direction, FilterOperator, FilterSpec, NullOrder, OrderBy, SemanticQuery,
};
use ossie_compiler::resolver::{QueryError, bind_query};
use ossie_model::parser::from_yaml_str;
use ossie_model::{DataType, OssieDocument};
use ossie_substrait::to_plan;

fn minimal_doc() -> OssieDocument {
    let yaml = include_str!("../../ossie-model/tests/fixtures/minimal.yaml");
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
    let err = bind_query(
        &model,
        &SemanticQuery {
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
        vec![NamedExpr {
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
    let yaml = include_str!("../../ossie-model/tests/fixtures/steelwheels.yaml");
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
    assert!(
        s.contains("orderfact") || s.contains("steelwheels.orderfact"),
        "{s}"
    );
    assert!(
        s.contains("dates") || s.contains("steelwheels.dates"),
        "{s}"
    );

    let LogicalPlan::Aggregate { group_by, .. } = plan else {
        panic!("expected aggregate root");
    };
    assert_eq!(group_by, vec![Expr::column("dates", "year_id")]);
}

#[test]
fn disconnected_datasets_error() {
    let yaml = include_str!("../../ossie-model/tests/fixtures/disconnected.yaml");
    let doc: OssieDocument = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let err = bind_query(
        &model,
        &SemanticQuery {
            group_by: vec!["alpha.id".into(), "beta.id".into()],
            dataset: Some("alpha".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::DisconnectedDatasets(_)),
        "{err:?}"
    );
}

#[test]
fn multi_dataset_requires_root_dataset() {
    let model = steelwheels_model();
    let err = bind_query(
        &model,
        &SemanticQuery {
            group_by: vec!["orderfact.order_status".into(), "dates.year_id".into()],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::MissingRootDataset), "{err:?}");
}

#[test]
fn people_daily_tenant_model_validates_and_plans() {
    use ossie_model::validate::validate;
    let yaml = include_str!("../../ossie-model/tests/fixtures/people_daily_tenant.yaml");
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
        filters: vec![FilterSpec::new(
            "people_daily.year",
            serde_json::json!([2024, 2025]),
        )],
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    build_logical_plan(&bound, &model).expect("plan");
    let p = to_plan(&build_logical_plan(&bound, &model).unwrap()).expect("substrait");
    assert!(!p.relations.is_empty());
}

#[test]
fn people_daily_distinct_and_tenure_metrics() {
    use ossie_model::validate::validate;
    let yaml = include_str!("../../ossie-model/tests/fixtures/people_daily_tenant.yaml");
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
            let bq = ossie_compiler::emit::sql::to_sql_with(
                &plan,
                &ossie_compiler::sql_dialect::BigQuery,
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
    let yaml = include_str!("../../ossie-model/tests/fixtures/expressions.yaml");
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
    {
        let p = to_plan(&plan).expect("substrait");
        assert_eq!(p.relations.len(), 1);
    }
}

#[test]
fn count_star_metric_is_structured() {
    let yaml = include_str!("../../ossie-model/tests/fixtures/expressions.yaml");
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
        panic!(
            "expected a structured aggregate, got {:?}",
            aggregates[0].expr
        );
    };
    assert_eq!(*func, Func::Sum);
    assert!(
        matches!(
            &args[0],
            Expr::Call {
                func: Func::Multiply,
                ..
            }
        ),
        "{:?}",
        args[0]
    );
    assert!(
        to_sql(&plan)
            .expect("sql")
            .contains("SUM((quantityordered * priceeach))")
    );
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

/// `SUM(a) / NULLIF(SUM(b), 0)` — the composite case. The plan says what it
/// does: two measures, plus a projection that recombines them.
#[test]
fn composite_metric_plans_a_projection_over_the_aggregate() {
    let plan = steelwheels_plan(&["avg_unit_price"], &[]);

    let LogicalPlan::Project { input, exprs } = &plan else {
        panic!("expected a Project root, got {plan:?}");
    };
    assert_eq!(exprs.len(), 1);
    assert_eq!(exprs[0].name, "avg_unit_price");
    assert!(
        matches!(
            &exprs[0].expr,
            Expr::Call {
                func: Func::Divide,
                ..
            }
        ),
        "{:?}",
        exprs[0].expr
    );

    let LogicalPlan::Aggregate { aggregates, .. } = input.as_ref() else {
        panic!("expected an Aggregate under the projection");
    };
    assert_eq!(
        aggregates.len(),
        2,
        "SUM(totalprice) and SUM(quantityordered)"
    );
    assert!(
        aggregates
            .iter()
            .all(|a| matches!(a.expr, Expr::Agg { .. })),
        "every measure must be a bare aggregate: {aggregates:?}"
    );

    // The rendered plan shows the projection rather than hiding it.
    let shown = format!("{}", plan.display_indent());
    assert!(shown.contains("Project:"), "{shown}");
    assert!(shown.contains("Aggregate:"), "{shown}");

    // SQL flattens it back: a composite of aggregates is legal in a select list.
    let sql = to_sql(&plan).expect("sql");
    assert!(
        sql.contains("(SUM(totalprice) / NULLIF(SUM(quantityordered), 0)) AS avg_unit_price"),
        "{sql}"
    );
    assert!(!sql.contains("__agg"), "measure names must not leak: {sql}");
}

/// A bare aggregate needs no projection, so the plan keeps its simpler shape.
#[test]
fn simple_metric_plans_no_projection() {
    let plan = steelwheels_plan(&["sales"], &[]);
    assert!(
        matches!(plan, LogicalPlan::Aggregate { .. }),
        "got {plan:?}"
    );
}

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
    // One expression per output column: the grouping passes through
    // explicitly, then the metric.
    assert_eq!(project.expressions.len(), 2);

    let emit = match project
        .common
        .expect("common")
        .emit_kind
        .expect("emit_kind")
    {
        substrait::proto::rel_common::EmitKind::Emit(e) => e.output_mapping,
        other => panic!("expected an explicit emit mapping, got {other:?}"),
    };
    // 1 grouping + 2 measures = 3 input columns, and ProjectRel appends its
    // expressions after them, so the outputs are 3 and 4 — the intermediate
    // measures are dropped.
    assert_eq!(emit, vec![3, 4]);

    let Some(RelType::Aggregate(agg)) = project.input.expect("project input").rel_type else {
        panic!("expected an AggregateRel under the projection");
    };
    assert_eq!(
        agg.measures.len(),
        2,
        "SUM(totalprice) and SUM(quantityordered)"
    );
}

/// A metric mentioning the same aggregate twice yields one measure, not two.
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
        matches!(
            root.input.expect("input").rel_type,
            Some(RelType::Aggregate(_))
        ),
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
    assert_eq!(model.issues[0].target, IssueTarget::metric("windowed"));
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
    assert!(
        !msg.contains("not found"),
        "should not claim it is missing: {msg}"
    );

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

    let targets: Vec<&IssueTarget> = model.issues.iter().map(|i| &i.target).collect();
    assert_eq!(targets.len(), 3, "{targets:?}");
    assert!(
        targets.contains(&&IssueTarget::field("t", "bad_field")),
        "{targets:?}"
    );
    assert!(
        targets.contains(&&IssueTarget::metric("bad_metric_a")),
        "{targets:?}"
    );
    assert!(
        targets.contains(&&IssueTarget::metric("bad_metric_b")),
        "{targets:?}"
    );

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
    assert_eq!(
        bound.joins.len(),
        1,
        "metric should require the customers join"
    );
    assert_eq!(bound.joins[0].right_dataset, "customers");

    let plan = build_logical_plan(&bound, &model).expect("plan");
    let sql = to_sql(&plan).expect("sql");
    assert!(sql.contains("INNER JOIN"), "{sql}");
    assert!(
        sql.contains("(SUM(orders.amount) / COUNT(DISTINCT customers.id))"),
        "{sql}"
    );

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
        let f: FilterSpec =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("{spelling}: {e}"));
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
    let bq = ossie_compiler::emit::sql::to_sql_with(&plan, &ossie_compiler::sql_dialect::BigQuery)
        .expect("bigquery sql");
    assert!(bq.contains("'O\\'Brien\\\\'"), "{bq}");
}

// ---------------------------------------------------------------------------
// Scan schema types (P0 fix 1): declared DataType reaches the Substrait schema.
// ---------------------------------------------------------------------------

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

    let kind = |n: &str| {
        by_name
            .iter()
            .find(|(name, _)| name == n)
            .expect(n)
            .1
            .clone()
    };

    assert!(matches!(kind("name"), Kind::String(_)));
    assert!(matches!(kind("qty"), Kind::I64(_)));
    assert!(matches!(kind("ratio"), Kind::Fp64(_)));
    assert!(matches!(kind("active"), Kind::Bool(_)));
    assert!(matches!(kind("d"), Kind::Date(_)));
    assert!(matches!(kind("ts"), Kind::PrecisionTimestampTz(_)));

    // Under-specified in Ossie, so `unknown` rather than an invented parameterization.
    assert!(matches!(kind("amount"), Kind::UserDefined(_)), "Decimal");
    assert!(
        matches!(kind("untyped"), Kind::UserDefined(_)),
        "no datatype"
    );

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
#[test]
fn scan_schema_does_not_default_everything_to_string() {
    use substrait::proto::rel::RelType;
    use substrait::proto::r#type::Kind;

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

// ---------------------------------------------------------------------------
// Column scope: a planned query cannot fail emission on a lookup.
// ---------------------------------------------------------------------------

/// `output_schema` is the one definition of what a node produces, and the
/// Substrait emitter resolves field references against it.
#[test]
fn output_schema_describes_each_node() {
    let plan = steelwheels_plan(&["avg_unit_price"], &["dates.year_id"]);

    let LogicalPlan::Project { input, .. } = &plan else {
        panic!("expected a projection");
    };
    let aggregate = input.as_ref();

    // Aggregate: one column per grouping, then one per measure.
    let agg_schema = aggregate.output_schema();
    assert_eq!(
        agg_schema.len(),
        3,
        "1 grouping + 2 measures: {agg_schema:?}"
    );
    assert_eq!(agg_schema[0].dataset, "dates");
    assert_eq!(agg_schema[0].name, "year_id");
    assert!(agg_schema[1].name.starts_with("__agg"), "{agg_schema:?}");

    // Project: exactly its named outputs, measures dropped.
    let out = plan.output_schema();
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[1].name, "avg_unit_price");
    assert!(out[1].dataset.is_empty());
    assert!(!out.iter().any(|c| c.name.starts_with("__agg")));

    // The join under it concatenates its children.
    let LogicalPlan::Aggregate { input, .. } = aggregate else {
        panic!("expected an aggregate");
    };
    let join = input.as_ref();
    let LogicalPlan::Join { left, right, .. } = join else {
        panic!("expected a join");
    };
    assert_eq!(
        join.output_schema().len(),
        left.output_schema().len() + right.output_schema().len()
    );
}

/// A reference the plan cannot produce is caught while planning, with the
/// available columns listed — not at emission time.
#[test]
fn out_of_scope_columns_fail_at_plan_time() {
    use ossie_compiler::plan::{NamedExpr, ScanColumn};

    let scan = LogicalPlan::Scan {
        source: "s.t".into(),
        dataset: "t".into(),
        columns: vec![ScanColumn::new("id", None)],
    };
    let plan = LogicalPlan::Aggregate {
        input: Box::new(scan),
        group_by: vec![Expr::column("t", "nope")],
        aggregates: vec![NamedExpr::new(
            "n",
            Expr::Agg {
                func: Func::Count,
                distinct: false,
                args: vec![],
            },
        )],
    };

    let err = ossie_compiler::planner::check_plan_columns(&plan)
        .expect_err("a grouping on an unknown column must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("not in scope"), "{msg}");
    assert!(msg.contains("t.nope"), "{msg}");
    assert!(
        msg.contains("id"),
        "the available columns should be listed: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Types flow through the plan, not just the scan.
// ---------------------------------------------------------------------------

fn typed_model() -> CompiledModel {
    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: t
    source: s.t
    fields:
      - name: amount
        datatype: Decimal
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: qty
        datatype: Integer
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: qty }]
      - name: label
        datatype: String
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: label }]
      - name: untyped
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: untyped }]
metrics:
  - name: total
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(t.amount)" }]
  - name: n
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "COUNT(*)" }]
  - name: mean_qty
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "AVG(t.qty)" }]
  - name: ratio
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(t.amount) / COUNT(*)" }]
  - name: over_untyped
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(t.untyped)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    CompiledModel::compile(&doc).expect("compile")
}

fn typed_plan(metrics: &[&str], group_by: &[&str]) -> LogicalPlan {
    let model = typed_model();
    let spec = SemanticQuery {
        metrics: metrics.iter().map(|s| (*s).to_string()).collect(),
        group_by: group_by.iter().map(|s| (*s).to_string()).collect(),
        dataset: Some("t".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    build_logical_plan(&bound, &model).expect("plan")
}

fn column_type(plan: &LogicalPlan, name: &str) -> Option<DataType> {
    plan.output_schema()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name:?}"))
        .datatype
}

/// An aggregate's type comes from the catalog row: SUM keeps its argument's
/// type, COUNT is always Integer, AVG is always Float.
#[test]
fn aggregate_output_types_follow_the_catalog() {
    let plan = typed_plan(&["total", "n", "mean_qty"], &[]);
    assert_eq!(column_type(&plan, "total"), Some(DataType::Decimal));
    assert_eq!(column_type(&plan, "n"), Some(DataType::Integer));
    assert_eq!(column_type(&plan, "mean_qty"), Some(DataType::Float));
}

/// A grouping column keeps the type the model declared for it, through the
/// aggregate.
#[test]
fn grouping_columns_keep_their_declared_type() {
    let plan = typed_plan(&["n"], &["t.label"]);
    assert_eq!(column_type(&plan, "label"), Some(DataType::String));
}

/// A composite metric is typed through the projection: Decimal / Integer
/// widens to Decimal.
#[test]
fn projected_types_are_inferred_through_the_projection() {
    let plan = typed_plan(&["ratio"], &[]);
    assert!(matches!(plan, LogicalPlan::Project { .. }));
    assert_eq!(column_type(&plan, "ratio"), Some(DataType::Decimal));
}

/// Ossie makes `datatype` optional, so an unknown input stays unknown rather
/// than becoming a guess.
#[test]
fn unknown_input_types_stay_unknown() {
    let plan = typed_plan(&["over_untyped"], &["t.untyped"]);
    assert_eq!(column_type(&plan, "over_untyped"), None);
    assert_eq!(column_type(&plan, "untyped"), None);
}

/// A known measure type reaches the Substrait plan; an unknown one is left
/// absent rather than declared wrongly.
#[test]
fn substrait_measures_declare_known_types_only() {
    use substrait::proto::rel::RelType;
    use substrait::proto::r#type::Kind;

    let measures = |metric: &str| {
        let plan = typed_plan(&[metric], &[]);
        let substrait_plan = to_plan(&plan).expect("substrait");
        let substrait::proto::plan_rel::RelType::Root(root) =
            substrait_plan.relations[0].rel_type.clone().expect("rel")
        else {
            panic!("expected a root rel");
        };
        let Some(RelType::Aggregate(agg)) = root.input.expect("input").rel_type else {
            panic!("expected an aggregate");
        };
        agg.measures
    };

    // COUNT is Integer whatever it counts.
    let out = measures("n")[0].measure.clone().expect("fn").output_type;
    assert!(matches!(out.and_then(|t| t.kind), Some(Kind::I64(_))));

    // AVG is Float.
    let out = measures("mean_qty")[0]
        .measure
        .clone()
        .expect("fn")
        .output_type;
    assert!(matches!(out.and_then(|t| t.kind), Some(Kind::Fp64(_))));

    // SUM over an undeclared column: no claim.
    let out = measures("over_untyped")[0]
        .measure
        .clone()
        .expect("fn")
        .output_type;
    assert!(out.is_none(), "an unknown type must not be declared");
}

// ---------------------------------------------------------------------------
// Ordering and row limits.
// ---------------------------------------------------------------------------

fn steelwheels_sorted(
    metrics: &[&str],
    group_by: &[&str],
    order_by: Vec<OrderBy>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> LogicalPlan {
    let model = steelwheels_model();
    let spec = SemanticQuery {
        group_by: group_by.iter().map(|s| (*s).to_string()).collect(),
        metrics: metrics.iter().map(|s| (*s).to_string()).collect(),
        dataset: Some("orderfact".into()),
        order_by,
        limit,
        offset,
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    build_logical_plan(&bound, &model).expect("plan")
}

#[test]
fn ordering_and_limit_shape_the_plan() {
    let plan = steelwheels_sorted(
        &["sales"],
        &["dates.year_id"],
        vec![OrderBy::desc("sales")],
        Some(10),
        Some(5),
    );

    let LogicalPlan::Fetch {
        input,
        offset,
        limit,
    } = &plan
    else {
        panic!("expected Fetch at the root, got {plan:?}");
    };
    assert_eq!((*offset, *limit), (5, Some(10)));

    let LogicalPlan::Sort { keys, .. } = input.as_ref() else {
        panic!("expected Sort under Fetch");
    };
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].direction, Direction::Desc);
    // Descending defaults nulls first: they sort as greater than non-nulls.
    assert_eq!(keys[0].nulls, NullOrder::First);

    // Neither node changes the columns.
    assert_eq!(plan.output_schema(), input.output_schema());
}

/// Ordering by a metric works because Sort sits above the projection.
#[test]
fn ordering_by_a_metric_output() {
    let sql = to_sql(&steelwheels_sorted(
        &["avg_unit_price"],
        &["dates.year_id"],
        vec![OrderBy::desc("avg_unit_price")],
        Some(3),
        None,
    ))
    .expect("sql");
    assert!(
        sql.contains("ORDER BY avg_unit_price DESC NULLS FIRST"),
        "{sql}"
    );
    assert!(sql.contains("FETCH NEXT 3 ROWS ONLY"), "{sql}");
}

#[test]
fn ordering_by_a_grouped_field_either_spelling() {
    for reference in ["dates.year_id", "year_id"] {
        let sql = to_sql(&steelwheels_sorted(
            &["sales"],
            &["dates.year_id"],
            vec![OrderBy::asc(reference)],
            None,
            None,
        ))
        .expect("sql");
        assert!(sql.contains("ORDER BY"), "{reference}: {sql}");
        assert!(sql.contains("ASC NULLS LAST"), "{reference}: {sql}");
    }
}

/// You can only order by what you selected — anything else is a query error,
/// with the available names listed.
#[test]
fn ordering_by_something_unselected_is_rejected() {
    let model = steelwheels_model();
    let spec = SemanticQuery {
        metrics: vec!["sales".into()],
        dataset: Some("orderfact".into()),
        order_by: vec![OrderBy::desc("quantity")],
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let err = build_logical_plan(&bound, &model).expect_err("quantity is not selected");
    let msg = err.to_string();
    assert!(msg.contains("not selected"), "{msg}");
    assert!(
        msg.contains("sales"),
        "should list what is available: {msg}"
    );
}

/// `LIMIT`/`OFFSET` is not ANSI; the standard spells it `FETCH NEXT`.
#[test]
fn row_limits_are_dialect_specific() {
    let plan = steelwheels_sorted(&["sales"], &[], vec![], Some(10), Some(5));

    let ansi = to_sql(&plan).expect("ansi");
    assert!(
        ansi.contains("OFFSET 5 ROWS FETCH NEXT 10 ROWS ONLY"),
        "{ansi}"
    );

    let bq = ossie_compiler::emit::sql::to_sql_with(&plan, &ossie_compiler::sql_dialect::BigQuery)
        .expect("bigquery");
    assert!(bq.contains("LIMIT 10 OFFSET 5"), "{bq}");
    assert!(!bq.contains("FETCH NEXT"), "{bq}");
}

#[test]
fn sort_and_fetch_reach_the_substrait_plan() {
    use substrait::proto::rel::RelType;
    use substrait::proto::sort_field::{SortDirection, SortKind};

    let plan = steelwheels_sorted(
        &["sales"],
        &["dates.year_id"],
        vec![OrderBy::desc("sales")],
        Some(10),
        Some(5),
    );
    let substrait_plan = to_plan(&plan).expect("substrait");
    let substrait::proto::plan_rel::RelType::Root(root) =
        substrait_plan.relations[0].rel_type.clone().expect("rel")
    else {
        panic!("expected a root rel");
    };

    let Some(RelType::Fetch(fetch)) = root.input.expect("input").rel_type else {
        panic!("expected a FetchRel at the top");
    };
    // The literal-expression forms, not the deprecated bare integers.
    assert!(matches!(
        fetch.offset_mode,
        Some(substrait::proto::fetch_rel::OffsetMode::OffsetExpr(_))
    ));
    assert!(matches!(
        fetch.count_mode,
        Some(substrait::proto::fetch_rel::CountMode::CountExpr(_))
    ));

    let Some(RelType::Sort(sort)) = fetch.input.expect("fetch input").rel_type else {
        panic!("expected a SortRel under the fetch");
    };
    assert_eq!(sort.sorts.len(), 1);
    // Substrait folds nulls placement into the direction.
    assert_eq!(
        sort.sorts[0].sort_kind,
        Some(SortKind::Direction(SortDirection::DescNullsFirst as i32))
    );
}

#[test]
fn explicit_nulls_ordering_overrides_the_default() {
    let plan = steelwheels_sorted(
        &["sales"],
        &[],
        vec![OrderBy {
            reference: "sales".into(),
            direction: Direction::Desc,
            nulls: Some(NullOrder::Last),
        }],
        None,
        None,
    );
    assert!(to_sql(&plan).expect("sql").contains("DESC NULLS LAST"));
}

#[test]
fn a_query_without_ordering_is_unchanged() {
    let plan = steelwheels_sorted(&["sales"], &[], vec![], None, None);
    assert!(
        matches!(plan, LogicalPlan::Aggregate { .. }),
        "no Sort or Fetch should appear: {plan:?}"
    );
}

// ---------------------------------------------------------------------------
// Fan-out: refuse a metric the join would inflate, rather than return it.
// ---------------------------------------------------------------------------

/// `orders` is the many side, `dim` the one side, so the join repeats each
/// `dim` row once per matching order.
const FAN_OUT_MODEL: &str = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: orders
    source: s.orders
    fields:
      - name: amount
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: dim_id
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: dim_id }]
  - name: dim
    source: s.dim
    primary_key: [id]
    fields:
      - name: id
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: id }]
      - name: headcount
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: headcount }]
relationships:
  - name: orders_to_dim
    from: orders
    to: dim
    from_columns: [dim_id]
    to_columns: [id]
metrics:
  - name: inflated
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.amount) / SUM(dim.headcount)" }]
  - name: safe_distinct
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.amount) / COUNT(DISTINCT dim.id)" }]
  - name: safe_max
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "MAX(dim.headcount)" }]
  - name: safe_many_side
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.amount)" }]
"#;

fn fan_out_plan(metric: &str) -> Result<LogicalPlan, QueryError> {
    let doc = from_yaml_str(FAN_OUT_MODEL).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec![metric.to_string()],
        dataset: Some("orders".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    build_logical_plan(&bound, &model)
}

/// The bug this exists for: summing the one side counts each of its rows once
/// per matching row on the many side.
#[test]
fn summing_the_one_side_of_a_join_is_refused() {
    let err = fan_out_plan("inflated").expect_err("SUM over the repeated side is inflated");
    let msg = err.to_string();
    assert!(msg.contains("would be inflated"), "{msg}");
    assert!(msg.contains("\"dim\""), "should name the dataset: {msg}");
    assert!(msg.contains("SUM"), "should name the aggregate: {msg}");
    assert!(
        msg.contains("COUNT(DISTINCT"),
        "should say what would work: {msg}"
    );
}

/// Repeated rows do not change these, so they stay allowed.
#[test]
fn duplicate_insensitive_aggregates_are_allowed() {
    fan_out_plan("safe_distinct").expect("COUNT(DISTINCT ...) collapses the repeats");
    fan_out_plan("safe_max").expect("MAX is unchanged by repeated rows");
}

/// The many side is not repeated, so aggregating it is fine.
#[test]
fn aggregating_the_many_side_is_unaffected() {
    fan_out_plan("safe_many_side").expect("orders rows appear once each");
}

/// Nothing is repeated without a join, so nothing is refused.
#[test]
fn a_single_dataset_query_never_fans_out() {
    let doc = from_yaml_str(FAN_OUT_MODEL).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["safe_many_side".into()],
        group_by: vec!["orders.dim_id".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    assert!(
        bound.fanned_datasets.is_empty(),
        "{:?}",
        bound.fanned_datasets
    );
    build_logical_plan(&bound, &model).expect("plan");
}

/// The binder names which side the join repeats, per the spec's reading of
/// `from` as the many side and `to` as the one side.
#[test]
fn the_binder_identifies_the_repeated_side() {
    let doc = from_yaml_str(FAN_OUT_MODEL).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["safe_distinct".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    assert_eq!(
        bound.fanned_datasets.iter().cloned().collect::<Vec<_>>(),
        vec!["dim".to_string()],
        "the one side is the repeated one"
    );
}

// ---------------------------------------------------------------------------
// Identifiers: matched case-insensitively, emitted as written.
// ---------------------------------------------------------------------------

/// Two fields spelling one underlying column differently name one column, and
/// the SQL keeps the spelling each field was written with.
#[test]
fn column_spellings_differing_only_by_case_are_one_column() {
    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: t
    source: s.t
    fields:
      - name: lower
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: upper
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: AMOUNT }]
metrics:
  - name: total
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(t.upper)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");

    // One scan column, not two.
    let cols = model.dataset("t").expect("dataset").scan_columns();
    assert_eq!(cols.len(), 1, "{cols:?}");
    assert_eq!(cols[0].0, "amount", "the first spelling is the one exposed");

    // The reference resolves against it despite the different case, and the
    // emitted SQL keeps what the field was written with.
    let spec = SemanticQuery {
        metrics: vec!["total".into()],
        dataset: Some("t".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    let sql = to_sql(&plan).expect("sql");
    assert!(
        sql.contains("SUM(AMOUNT)"),
        "identifier text is preserved: {sql}"
    );

    to_plan(&plan).expect("the reference must resolve to a scan column");
}

/// Entity names are not SQL identifiers, so they are matched as written.
#[test]
fn entity_names_are_still_matched_exactly() {
    let yaml = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: Orders
    source: s.orders
    fields:
      - name: Amount
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
metrics:
  - name: as_declared
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(Orders.Amount)" }]
  - name: wrong_case
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.Amount)" }]
"#;
    let doc = from_yaml_str(yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");

    assert!(model.metric("as_declared").is_some());
    assert!(model.metric("wrong_case").is_none());
    let issue = &model.issues[0];
    assert!(issue.message.contains("declares \"Orders\""), "{issue:?}");
}

// ---------------------------------------------------------------------------
// Metric composition: a metric may reference another by bare name.
// ---------------------------------------------------------------------------

const COMPOSED: &str = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: orders
    source: s.orders
    fields:
      - name: amount
        datatype: Decimal
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: qty
        datatype: Integer
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: qty }]
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
  - name: total_revenue
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.amount)" }]
  - name: total_units
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.qty)" }]
  - name: customer_count
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "COUNT(DISTINCT customers.id)" }]
  - name: revenue_per_unit
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "total_revenue / total_units" }]
  - name: revenue_in_thousands
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "total_revenue / 1000" }]
  - name: revenue_per_customer
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "total_revenue / customer_count" }]
  - name: doubly_derived
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "revenue_per_unit * 100" }]
"#;

fn composed_sql(metric: &str) -> String {
    let doc = from_yaml_str(COMPOSED).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    assert!(model.issues.is_empty(), "{:?}", model.issues);
    let spec = SemanticQuery {
        metrics: vec![metric.to_string()],
        dataset: Some("orders".into()),
        ..Default::default()
    };
    let bound = bind_query(&model, &spec).expect("bind");
    let plan = build_logical_plan(&bound, &model).expect("plan");
    to_plan(&plan).expect("substrait");
    to_sql(&plan).expect("sql")
}

/// A reference is substituted with what the referenced metric compiled to.
#[test]
fn a_metric_may_be_built_from_other_metrics() {
    let sql = composed_sql("revenue_per_unit");
    assert!(
        sql.contains("(SUM(amount) / SUM(qty)) AS revenue_per_unit"),
        "{sql}"
    );
}

#[test]
fn a_metric_may_be_scaled_by_a_constant() {
    assert!(composed_sql("revenue_in_thousands").contains("(SUM(amount) / 1000)"));
}

/// Composition is transitive.
#[test]
fn a_derived_metric_may_itself_be_referenced() {
    let sql = composed_sql("doubly_derived");
    assert!(sql.contains("((SUM(amount) / SUM(qty)) * 100)"), "{sql}");
}

/// Substitution is closed under the IR, so a reference that reaches another
/// dataset still pulls in its join.
#[test]
fn composition_carries_the_referenced_metrics_datasets() {
    let sql = composed_sql("revenue_per_customer");
    assert!(
        sql.contains("INNER JOIN"),
        "the join must follow the reference: {sql}"
    );
    assert!(
        sql.contains("(SUM(orders.amount) / COUNT(DISTINCT customers.id))"),
        "{sql}"
    );
}

/// And the composed metric is still split into measures for Substrait.
#[test]
fn a_composed_metric_still_splits_into_measures() {
    use substrait::proto::rel::RelType;

    let doc = from_yaml_str(COMPOSED).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    let spec = SemanticQuery {
        metrics: vec!["revenue_per_unit".into()],
        dataset: Some("orders".into()),
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
        panic!("expected a projection over the aggregate");
    };
    let Some(RelType::Aggregate(agg)) = project.input.expect("input").rel_type else {
        panic!("expected an aggregate");
    };
    assert_eq!(agg.measures.len(), 2, "SUM(amount) and SUM(qty)");
}

fn compile_issue(metrics: &str) -> String {
    let yaml = format!(
        r#"version: "0.2.0.dev0"
name: m
datasets:
  - name: orders
    source: s.orders
    fields:
      - name: amount
        expression:
          dialects: [{{ dialect: ANSI_SQL, expression: amount }}]
metrics:
{metrics}"#
    );
    let doc = from_yaml_str(&yaml).expect("parse");
    let model = CompiledModel::compile(&doc).expect("compile");
    model
        .issues
        .iter()
        .map(|i| format!("{i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_reference_cycle_is_reported_with_its_path() {
    let issues = compile_issue(
        r#"  - name: a
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "b + 1" }]
  - name: b
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "a + 1" }]
"#,
    );
    assert!(issues.contains("cycle"), "{issues}");
    assert!(
        issues.contains("a -> b -> a"),
        "the path should be named: {issues}"
    );
}

#[test]
fn a_metric_cannot_reference_itself() {
    let issues = compile_issue(
        r#"  - name: loop
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "loop * 2" }]
"#,
    );
    assert!(issues.contains("cycle"), "{issues}");
}

/// A metric is already aggregated, so it cannot be wrapped in an aggregate.
#[test]
fn a_metric_reference_cannot_sit_inside_an_aggregate() {
    let issues = compile_issue(
        r#"  - name: total
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(orders.amount)" }]
  - name: nested
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(total)" }]
"#,
    );
    assert!(issues.contains("already aggregated"), "{issues}");
}

/// Whose failure it is should be clear.
#[test]
fn depending_on_a_broken_metric_says_so() {
    let issues = compile_issue(
        r#"  - name: windowed
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "RANK() OVER (ORDER BY orders.amount)" }]
  - name: depends_on_it
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "windowed / 2" }]
"#,
    );
    assert!(
        issues.contains("references metric \"windowed\", which did not compile"),
        "{issues}"
    );
}

#[test]
fn an_unknown_bare_name_explains_the_two_spellings() {
    let issues = compile_issue(
        r#"  - name: oops
    expression:
      dialects: [{ dialect: ANSI_SQL, expression: "SUM(amount)" }]
"#,
    );
    assert!(issues.contains("unknown metric"), "{issues}");
    assert!(
        issues.contains("dataset.field"),
        "should say how to write a column: {issues}"
    );
}
