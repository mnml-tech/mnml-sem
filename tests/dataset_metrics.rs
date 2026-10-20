//! Dataset-scoped metrics (`datasets[].metrics`), ahead of the schema — see
//! ADR-0014 and apache/ossie#591.

use ossie_substrait::compiled::{CompiledModel, IssueTarget};
use ossie_substrait::emit::sql::to_sql;
#[cfg(feature = "substrait")]
use ossie_substrait::emit::substrait::to_plan;
use ossie_substrait::model::OssieDocument;
use ossie_substrait::parser::from_yaml_str;
use ossie_substrait::plan::LogicalPlan;
use ossie_substrait::planner::build_logical_plan;
use ossie_substrait::query::{OrderBy, SemanticQuery};
use ossie_substrait::resolver::{QueryError, bind_query};

fn doc() -> OssieDocument {
    from_yaml_str(include_str!("fixtures/dataset_metrics.yaml")).expect("parse fixture")
}

fn model() -> CompiledModel {
    CompiledModel::compile_strict(&doc()).expect("compile fixture")
}

fn plan(spec: &SemanticQuery) -> Result<LogicalPlan, QueryError> {
    let model = model();
    let bound = bind_query(&model, spec)?;
    build_logical_plan(&bound, &model)
}

fn sql(spec: &SemanticQuery) -> String {
    to_sql(&plan(spec).expect("plan")).expect("sql")
}

/// A model with one dataset `orders` whose metric is `expression`.
fn one_metric(expression: &str) -> String {
    format!(
        r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: orders
    source: s.orders
    fields:
      - name: amount
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: amount
      - name: customer_id
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: customer_id
    metrics:
      - name: total
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: SUM(amount)
      - name: probe
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: "{expression}"
  - name: customers
    source: s.customers
    fields:
      - name: id
        expression:
          dialects:
            - dialect: ANSI_SQL
              expression: id
relationships:
  - name: r
    from: orders
    to: customers
    from_columns: [customer_id]
    to_columns: [id]
"#
    )
}

fn probe_issue(expression: &str) -> String {
    let doc = from_yaml_str(&one_metric(expression)).expect("parse");
    let model = CompiledModel::compile(&doc).expect("structurally valid");
    model
        .issue_for(&IssueTarget::dataset_metric("orders", "probe"))
        .unwrap_or_else(|| panic!("{expression:?} should not compile"))
        .message
        .clone()
}

#[test]
fn dataset_metrics_parse_and_round_trip() {
    let doc = doc();
    let orders = &doc.model.datasets[0];
    assert_eq!(orders.metrics.len(), 2);

    let yaml = serde_yaml::to_string(&doc).expect("serialize");
    let again = from_yaml_str(&yaml).expect("reparse");
    assert_eq!(again.model.datasets[0].metrics.len(), 2);
    assert_eq!(again.model.datasets[1].metrics.len(), 2);
}

#[test]
fn a_dataset_metric_compiles_against_its_own_fields() {
    let model = model();
    let m = model
        .dataset_metric("orders", "total_net_amount")
        .expect("compiled");
    assert_eq!(m.dataset.as_deref(), Some("orders"));
    assert_eq!(m.reference(), "orders.total_net_amount");
    assert_eq!(m.datasets.iter().collect::<Vec<_>>(), ["orders"]);
}

#[test]
fn a_query_names_a_dataset_metric_qualified() {
    let out = sql(&SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["orders.total_net_amount".into()],
        ..Default::default()
    });
    assert_eq!(
        out,
        "SELECT status, SUM((amount - discount)) AS \"orders.total_net_amount\" \
         FROM sales.public.orders GROUP BY status"
    );
}

/// The spec gives the two scopes separate namespaces, so the same bare name
/// can mean one metric of each, and a query can ask for both.
#[test]
fn a_dataset_metric_and_a_model_metric_may_share_a_bare_name() {
    let out = sql(&SemanticQuery {
        metrics: vec!["orders.total_net_amount".into(), "total_net_amount".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    });
    assert!(
        out.contains("AS \"orders.total_net_amount\"") && out.contains("AS total_net_amount"),
        "{out}"
    );
}

#[test]
fn ordering_matches_a_dataset_metric_by_its_reference() {
    let out = sql(&SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["orders.order_count".into()],
        order_by: vec![OrderBy::desc("orders.order_count")],
        ..Default::default()
    });
    assert!(
        out.ends_with("ORDER BY \"orders.order_count\" DESC NULLS FIRST"),
        "{out}"
    );
}

/// Grouping by another dataset pulls in the join, as a model metric would.
#[test]
fn a_dataset_metric_grouped_across_a_relationship_joins() {
    let out = sql(&SemanticQuery {
        group_by: vec!["customers.region".into()],
        metrics: vec!["orders.order_count".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    });
    assert!(
        out.contains("INNER JOIN sales.public.customers ON orders.customer_id = customers.id"),
        "{out}"
    );
}

/// Being dataset-scoped does not exempt a metric from ADR-0011: summing the
/// one side of a many-to-one join still counts each row once per order.
#[test]
fn a_dataset_metric_the_join_would_inflate_is_refused() {
    let err = plan(&SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["customers.total_credit".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    })
    .expect_err("SUM over the repeated side");
    assert!(
        matches!(err, QueryError::FanOut { ref metric, .. } if metric == "customers.total_credit"),
        "{err}"
    );
}

#[test]
fn an_unknown_dataset_metric_is_not_found() {
    let err = plan(&SemanticQuery {
        metrics: vec!["orders.nope".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    })
    .expect_err("no such metric");
    assert!(
        matches!(err, QueryError::MetricNotFound(ref n) if n == "orders.nope"),
        "{err}"
    );
}

/// The bare name is the model-scoped namespace; it does not fall back to a
/// dataset.
#[test]
fn a_dataset_metric_is_not_reachable_by_its_bare_name() {
    let err = plan(&SemanticQuery {
        metrics: vec!["order_count".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    })
    .expect_err("order_count is only dataset-scoped");
    assert!(matches!(err, QueryError::MetricNotFound(_)), "{err}");
}

#[test]
fn a_failed_dataset_metric_reports_why_when_queried() {
    let doc = from_yaml_str(&one_metric("SUM(nope)")).expect("parse");
    let model = CompiledModel::compile(&doc).expect("structurally valid");
    let err = bind_query(
        &model,
        &SemanticQuery {
            metrics: vec!["orders.probe".into()],
            dataset: Some("orders".into()),
            ..Default::default()
        },
    )
    .expect_err("probe did not compile");
    assert!(
        matches!(err, QueryError::NotCompiled { ref target, .. } if target == r#"metric "orders"."probe""#),
        "{err}"
    );
}

#[test]
fn a_qualified_reference_is_refused() {
    let msg = probe_issue("SUM(orders.amount)");
    assert!(msg.contains("is qualified"), "{msg}");
    let msg = probe_issue("COUNT(DISTINCT customers.id)");
    assert!(msg.contains("is qualified"), "{msg}");
}

#[test]
fn a_sibling_metric_is_refused() {
    let msg = probe_issue("total * 2");
    assert!(
        msg.contains("may reference only that dataset's declared fields"),
        "{msg}"
    );
}

#[test]
fn an_undeclared_name_is_refused() {
    // A column of the source that no field declares is a future extension.
    let msg = probe_issue("SUM(discount)");
    assert!(msg.contains("field not found: orders.discount"), "{msg}");
}

#[test]
fn a_case_mismatch_names_the_declared_field() {
    let msg = probe_issue("SUM(Amount)");
    assert!(msg.contains(r#"the model declares "amount""#), "{msg}");
}

#[test]
fn a_dataset_metric_needs_an_aggregate() {
    let msg = probe_issue("amount");
    assert!(msg.contains("has no aggregate"), "{msg}");
}

/// A model with `doubled: <expression>` added at the model level.
fn model_metric_issue(expression: &str) -> Option<String> {
    let yaml = one_metric("SUM(amount)").replace(
        "relationships:",
        &format!(
            r#"metrics:
  - name: doubled
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: "{expression}"
relationships:"#
        ),
    );
    let model = CompiledModel::compile(&from_yaml_str(&yaml).expect("parse")).expect("valid");
    model
        .issue_for(&IssueTarget::metric("doubled"))
        .map(|i| i.message.clone())
}

/// A model-scoped metric builds on dataset-scoped ones by substitution, so it
/// compiles to exactly what the same metric written over fields does.
#[test]
fn a_model_metric_built_from_dataset_metrics_matches_the_hand_written_one() {
    let query = |metric: &str| SemanticQuery {
        group_by: vec!["customers.region".into()],
        metrics: vec![metric.into()],
        dataset: Some("orders".into()),
        ..Default::default()
    };
    let composed = sql(&query("net_per_customer"));
    let hand_written = sql(&query("revenue_per_customer"));
    assert_eq!(
        composed.replace("net_per_customer", "revenue_per_customer"),
        hand_written
    );

    let model = model();
    let m = model.metric("net_per_customer").expect("compiled");
    assert_eq!(
        m.datasets.iter().collect::<Vec<_>>(),
        ["customers", "orders"],
        "the referenced metrics' datasets decide the joins"
    );
}

/// A model metric over a model metric over dataset metrics.
#[test]
fn composition_nests() {
    let out = sql(&SemanticQuery {
        metrics: vec!["net_per_customer_pct".into()],
        dataset: Some("orders".into()),
        ..Default::default()
    });
    assert!(out.contains("COUNT(DISTINCT customers.id)"), "{out}");
    assert!(out.contains("* 100"), "{out}");
}

/// Substitution keeps ADR-0011 intact: the join still repeats `customers`.
#[test]
fn a_composed_metric_the_join_would_inflate_is_refused() {
    let yaml = include_str!("fixtures/dataset_metrics.yaml").replace(
        "  - name: net_per_customer\n",
        "  - name: credit_per_order\n    expression:\n      dialects:\n        \
         - dialect: ANSI_SQL\n          expression: customers.total_credit / orders.order_count\n  \
         - name: net_per_customer\n",
    );
    let model =
        CompiledModel::compile_strict(&from_yaml_str(&yaml).expect("parse")).expect("compile");
    let bound = bind_query(
        &model,
        &SemanticQuery {
            metrics: vec!["credit_per_order".into()],
            dataset: Some("orders".into()),
            ..Default::default()
        },
    )
    .expect("bind");
    let err = build_logical_plan(&bound, &model).expect_err("SUM over the repeated side");
    assert!(matches!(err, QueryError::FanOut { .. }), "{err}");
}

#[test]
fn a_dataset_metric_cannot_be_reaggregated() {
    let msg = model_metric_issue("SUM(orders.total)").expect("refused");
    assert!(msg.contains("already aggregated"), "{msg}");
}

#[test]
fn referencing_a_failed_dataset_metric_says_whose_failure_it_is() {
    let yaml = one_metric("SUM(nope)").replace(
        "relationships:",
        r#"metrics:
  - name: doubled
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: orders.probe * 2
relationships:"#,
    );
    let model = CompiledModel::compile(&from_yaml_str(&yaml).expect("parse")).expect("valid");
    let msg = &model
        .issue_for(&IssueTarget::metric("doubled"))
        .expect("doubled should not compile")
        .message;
    assert!(
        msg.contains(r#"references metric "orders.probe", which did not compile"#)
            && msg.contains("orders.nope"),
        "{msg}"
    );
}

#[test]
fn a_case_mismatch_on_a_dataset_metric_names_the_declared_metric() {
    let msg = model_metric_issue("orders.Total * 2").expect("refused");
    assert!(
        msg.contains(r#"metric "orders.Total" not found; the model declares "total""#),
        "{msg}"
    );
}

#[test]
fn a_model_metric_compiles_over_a_dataset_metric() {
    assert_eq!(model_metric_issue("orders.total * 2"), None);
}

#[cfg(feature = "substrait")]
#[test]
fn a_dataset_metric_emits_substrait_named_by_its_reference() {
    let plan = plan(&SemanticQuery {
        group_by: vec!["orders.status".into()],
        metrics: vec!["orders.total_net_amount".into()],
        ..Default::default()
    })
    .expect("plan");
    let substrait = to_plan(&plan).expect("substrait");
    let names = format!("{substrait:?}");
    assert!(names.contains("\"orders.total_net_amount\""), "{names}");
}
