//! Compile MNML JSON expressions into [`Expr`](super::expr::Expr).

use crate::model::{envelope_from_expression, field_mnml_expr_value};
use crate::model::mnml_expression::{MnmlExpressionKind, MnmlMetricDef};
use crate::model::{Field, Metric, SemanticModel};
use crate::plan::expr::{AggFunc, Expr, Literal};
use crate::resolver::QueryError;

/// Context for resolving `dataset.field` paths in MNML JSON.
pub struct MnmlResolveCtx<'a> {
    pub model: &'a SemanticModel,
    /// Default dataset when `col` is unqualified (field on a dataset).
    pub default_dataset: Option<&'a str>,
}

fn field_mnml_expr_json(field: &Field) -> Result<Option<serde_json::Value>, QueryError> {
    envelope_from_expression(&field.expression, MnmlExpressionKind::FieldExpr)
        .map_err(QueryError::InvalidMnmlExpression)
}

/// Convert MNML dialect or legacy extension on a [`Field`] when present; otherwise `None`.
pub fn field_to_expr(field: &Field, dataset: &str, model: &SemanticModel) -> Result<Option<Expr>, QueryError> {
    let Some(expr_json) = field_mnml_expr_json(field)? else {
        return Ok(None);
    };
    let ctx = MnmlResolveCtx {
        model,
        default_dataset: Some(dataset),
    };
    Ok(Some(mnml_value_to_expr(&expr_json, &ctx)?))
}

fn metric_mnml_def_json(metric: &Metric) -> Result<Option<serde_json::Value>, QueryError> {
    envelope_from_expression(&metric.expression, MnmlExpressionKind::Metric)
        .map_err(QueryError::InvalidMnmlExpression)
}

/// Convert MNML dialect or legacy extension on a [`Metric`] when present; otherwise `None`.
pub fn metric_to_expr(metric: &Metric, model: &SemanticModel) -> Result<Option<Expr>, QueryError> {
    let Some(def_json) = metric_mnml_def_json(metric)? else {
        return Ok(None);
    };
    let def: MnmlMetricDef = serde_json::from_value(def_json).map_err(|e| {
        QueryError::InvalidMnmlExpression(format!("metric expr: {e}"))
    })?;
    let ctx = MnmlResolveCtx {
        model,
        default_dataset: None,
    };
    Ok(Some(metric_def_to_expr(&def, &ctx)?))
}

fn metric_def_to_expr(def: &MnmlMetricDef, ctx: &MnmlResolveCtx<'_>) -> Result<Expr, QueryError> {
    let func = parse_agg_name(&def.agg)?;
    let arg = if let Some(field) = &def.field {
        Some(Box::new(resolve_field_ref(field, ctx)?))
    } else if let Some(v) = &def.arg {
        Some(Box::new(mnml_value_to_expr(v, ctx)?))
    } else {
        None
    };
    Ok(Expr::Agg {
        func,
        distinct: def.distinct,
        arg,
    })
}

fn parse_agg_name(s: &str) -> Result<AggFunc, QueryError> {
    match s.to_ascii_lowercase().as_str() {
        "count" => Ok(AggFunc::Count),
        "sum" => Ok(AggFunc::Sum),
        "avg" => Ok(AggFunc::Avg),
        "min" => Ok(AggFunc::Min),
        "max" => Ok(AggFunc::Max),
        other => Err(QueryError::InvalidMnmlExpression(format!(
            "unknown aggregate {other:?}"
        ))),
    }
}

fn mnml_value_to_expr(v: &serde_json::Value, ctx: &MnmlResolveCtx<'_>) -> Result<Expr, QueryError> {
    if v.get("current_date").is_some() || v.get("today").is_some() {
        return Ok(Expr::CurrentDate);
    }
    if let Some(arr) = v.get("sub").and_then(|a| a.as_array()) {
        let pair = two_exprs(arr, "sub", ctx)?;
        return Ok(Expr::Sub(Box::new(pair.0), Box::new(pair.1)));
    }
    if let Some(col) = v.get("col").and_then(|c| c.as_str()) {
        return resolve_col(col, ctx);
    }
    if let Some(lit) = v.get("lit") {
        return Ok(Expr::Literal(Literal::from_json(lit)));
    }
    if let Some(arr) = v.get("eq").and_then(|a| a.as_array()) {
        let pair = two_exprs(arr, "eq", ctx)?;
        return Ok(Expr::Eq(Box::new(pair.0), Box::new(pair.1)));
    }
    if let Some(arr) = v.get("ne").and_then(|a| a.as_array()) {
        let pair = two_exprs(arr, "ne", ctx)?;
        return Ok(Expr::Ne(Box::new(pair.0), Box::new(pair.1)));
    }
    if let Some(arr) = v.get("and").and_then(|a| a.as_array()) {
        let exprs: Result<Vec<_>, _> = arr.iter().map(|e| mnml_value_to_expr(e, ctx)).collect();
        return Ok(Expr::and(exprs?));
    }
    if let Some(obj) = v.get("in").and_then(|o| o.as_object()) {
        let col = obj
            .get("col")
            .and_then(|c| c.as_str())
            .ok_or_else(|| QueryError::InvalidMnmlExpression("in.col required".into()))?;
        let values = obj
            .get("values")
            .and_then(|x| x.as_array())
            .ok_or_else(|| QueryError::InvalidMnmlExpression("in.values required".into()))?;
        return Ok(Expr::In {
            expr: Box::new(resolve_col(col, ctx)?),
            values: values.iter().map(Literal::from_json).collect(),
        });
    }
    if let Some(branches) = v.get("case").and_then(|c| c.as_array()) {
        let mut when_then = Vec::new();
        for b in branches {
            let when = b.get("when").ok_or_else(|| {
                QueryError::InvalidMnmlExpression("case branch missing when".into())
            })?;
            let then = b.get("then").ok_or_else(|| {
                QueryError::InvalidMnmlExpression("case branch missing then".into())
            })?;
            when_then.push((
                mnml_value_to_expr(when, ctx)?,
                mnml_value_to_expr(then, ctx)?,
            ));
        }
        let else_expr = match v.get("else") {
            Some(e) => mnml_value_to_expr(e, ctx)?,
            None => Expr::Literal(Literal::Null),
        };
        return Ok(Expr::Case {
            when_then,
            else_expr: Box::new(else_expr),
        });
    }
    Err(QueryError::InvalidMnmlExpression(format!(
        "unsupported MNML expression node: {v}"
    )))
}

fn two_exprs(
    arr: &[serde_json::Value],
    op: &str,
    ctx: &MnmlResolveCtx<'_>,
) -> Result<(Expr, Expr), QueryError> {
    if arr.len() != 2 {
        return Err(QueryError::InvalidMnmlExpression(format!(
            "{op} requires exactly two arguments"
        )));
    }
    Ok((
        mnml_value_to_expr(&arr[0], ctx)?,
        mnml_value_to_expr(&arr[1], ctx)?,
    ))
}

/// Resolve `dataset.field` to a full expression (MNML field body or simple column).
fn resolve_field_ref(path: &str, ctx: &MnmlResolveCtx<'_>) -> Result<Expr, QueryError> {
    let (dataset, field_name) = parse_col_path(path, ctx.default_dataset)?;
    let ds = ctx
        .model
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .ok_or_else(|| QueryError::DatasetNotFound(dataset.clone()))?;
    let field = ds
        .fields
        .iter()
        .find(|f| f.name == field_name)
        .ok_or_else(|| QueryError::FieldNotFound(format!("{dataset}.{field_name}")))?;
    if let Some(expr) = field_to_expr(field, &dataset, ctx.model)? {
        return Ok(expr);
    }
    let sql = crate::plan::pick_expression_sql(&field.expression)
        .ok_or(QueryError::MissingExpressionSql)?;
    Ok(Expr::column(dataset, sql))
}

/// Resolve to a physical scan column (for filters and `col` nodes in MNML).
fn resolve_col(path: &str, ctx: &MnmlResolveCtx<'_>) -> Result<Expr, QueryError> {
    let (dataset, field_name) = parse_col_path(path, ctx.default_dataset)?;
    let physical = physical_column_for_field(ctx.model, &dataset, &field_name)?;
    Ok(Expr::column(dataset, physical))
}

fn parse_col_path(path: &str, default_dataset: Option<&str>) -> Result<(String, String), QueryError> {
    let path = path.trim();
    if let Some((ds, field)) = path.split_once('.') {
        if ds.is_empty() || field.is_empty() {
            return Err(QueryError::InvalidFieldRef(path.to_string()));
        }
        return Ok((ds.to_string(), field.to_string()));
    }
    let ds = default_dataset
        .ok_or_else(|| QueryError::InvalidFieldRef(format!("unqualified col {path:?}")))?;
    Ok((ds.to_string(), path.to_string()))
}

/// Physical parquet/SQL column name for a logical field (MNML `{ "col": "x" }` or simple dialect).
pub fn physical_column_for_field(
    model: &SemanticModel,
    dataset_name: &str,
    field_name: &str,
) -> Result<String, QueryError> {
    let ds = model
        .datasets
        .iter()
        .find(|d| d.name == dataset_name)
        .ok_or_else(|| QueryError::DatasetNotFound(dataset_name.to_string()))?;
    let field = ds
        .fields
        .iter()
        .find(|f| f.name == field_name)
        .ok_or_else(|| QueryError::FieldNotFound(format!("{dataset_name}.{field_name}")))?;
    if let Some(expr_json) = field_mnml_expr_json(field)? {
        let body = field_mnml_expr_value(&expr_json);
        if let Some(col) = body.get("col").and_then(serde_json::Value::as_str) {
            let (_, fname) = parse_col_path(col, Some(dataset_name))?;
            if fname == field.name {
                return Ok(fname.to_string());
            }
        }
        return Err(QueryError::InvalidMnmlExpression(format!(
            "field {dataset_name}.{field_name} is not a physical column"
        )));
    }
    let sql = crate::plan::pick_expression_sql(&field.expression)
        .ok_or(QueryError::MissingExpressionSql)?;
    if sql.contains(' ') || sql.contains('(') {
        return Err(QueryError::InvalidMnmlExpression(format!(
            "field {dataset_name}.{field_name} has non-physical dialect expression"
        )));
    }
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::from_yaml_str;

    #[test]
    fn mnml_count_distinct_field() {
        let yaml = r#"
version: "0.1.1"
semantic_model:
  - name: m
    datasets:
      - name: d
        source: s.t
        fields:
          - name: dt
            expression:
              dialects:
                - dialect: MNML
                  col: dt
    metrics:
      - name: num_days
        expression:
          dialects:
            - dialect: MNML
              aggregation: count
              distinct: true
              field: d.dt
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let model = &doc.semantic_model[0];
        let metric = model.metrics.iter().find(|m| m.name == "num_days").unwrap();
        let expr = metric_to_expr(metric, model).expect("metric").expect("some");
        let Expr::Agg {
            func,
            distinct,
            arg,
        } = expr
        else {
            panic!("expected agg");
        };
        assert_eq!(func, AggFunc::Count);
        assert!(distinct);
        assert!(arg.is_some());
    }

    #[test]
    fn mnml_max_date_subtract() {
        let yaml = r#"
version: "0.1.1"
semantic_model:
  - name: m
    datasets:
      - name: d
        source: s.t
        fields:
          - name: employment_start_date
            expression:
              dialects:
                - dialect: MNML
                  col: employment_start_date
    metrics:
      - name: tenure
        expression:
          dialects:
            - dialect: MNML
              aggregation: max
              expr:
                sub:
                  - current_date: true
                  - col: d.employment_start_date
"#;
        let doc = from_yaml_str(yaml).expect("parse");
        let model = &doc.semantic_model[0];
        let metric = model.metrics.iter().find(|m| m.name == "tenure").unwrap();
        let expr = metric_to_expr(metric, model).expect("metric").expect("some");
        let Expr::Agg { func, arg, .. } = expr else {
            panic!("expected agg");
        };
        assert_eq!(func, AggFunc::Max);
        let Expr::Sub(left, right) = *arg.expect("arg") else {
            panic!("expected sub");
        };
        assert!(matches!(*left, Expr::CurrentDate));
        assert!(matches!(*right, Expr::Column(_)));
    }

    #[test]
    fn mnml_metric_sum_field() {
        let yaml = include_str!("../../tests/fixtures/mnml_expressions.yaml");
        let doc = from_yaml_str(yaml).expect("parse");
        let model = &doc.semantic_model[0];
        let metric = model.metrics.iter().find(|m| m.name == "starters").unwrap();
        let expr = metric_to_expr(metric, model).expect("metric").expect("some");
        let Expr::Agg { func, arg, .. } = expr else {
            panic!("expected agg");
        };
        assert_eq!(func, AggFunc::Sum);
        let arg = arg.expect("arg");
        let Expr::Case { when_then, .. } = *arg else {
            panic!("expected CASE from starter_flag field, got {arg:?}");
        };
        assert_eq!(when_then.len(), 1);
    }
}
