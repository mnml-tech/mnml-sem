//! Build a [`LogicalPlan`](crate::plan::LogicalPlan) from a [`BoundQuery`](crate::resolver::BoundQuery) (verb).
//!
//! Expressions arrive already lowered by [`CompiledModel`](crate::compiled::CompiledModel),
//! so this module only assembles plan nodes.

use crate::compiled::CompiledModel;
use crate::plan::{
    ColumnRef, Expr, Literal, LogicalPlan, NamedExpr, ScanColumn, SchemaColumn, SortKey,
    find_column,
};
use crate::query::OrderBy;
use crate::resolver::{BoundQuery, QueryError, ResolvedField};

/// A group-by or filter field, qualified by its dataset.
fn field_expr(rf: &ResolvedField) -> Expr {
    rf.field.expr.clone()
}

fn dataset_columns(
    model: &CompiledModel,
    dataset_name: &str,
) -> Result<Vec<ScanColumn>, QueryError> {
    let ds = model
        .dataset(dataset_name)
        .ok_or_else(|| QueryError::DatasetNotFound(dataset_name.to_string()))?;
    let cols: Vec<ScanColumn> = ds
        .scan_columns()
        .into_iter()
        .map(|(name, datatype)| ScanColumn::new(name, datatype))
        .collect();
    if cols.is_empty() {
        return Err(QueryError::InvalidExpression(format!(
            "dataset {dataset_name:?} has no physical columns for scan"
        )));
    }
    Ok(cols)
}

fn filter_to_expr(f: &crate::resolver::BoundFilter, qualify: bool) -> Result<Expr, QueryError> {
    use crate::catalog::Func;
    use crate::query::FilterOperator as Op;

    let lhs = qualify_column(field_expr(&f.field), qualify);

    let invalid = |reason: String| QueryError::InvalidFilter {
        field: f.field.field.name.clone(),
        reason,
    };

    // A single literal operand, for the comparison operators.
    let literal = || -> Result<Expr, QueryError> {
        Literal::from_json(&f.value)
            .map(Expr::Literal)
            .ok_or_else(|| invalid(format!("{} is not a scalar value", f.value)))
    };

    // A literal list, for the set operators.
    let values = || -> Result<Vec<Literal>, QueryError> {
        let items = f
            .value
            .as_array()
            .ok_or_else(|| invalid("expects an array value".to_string()))?;
        items
            .iter()
            .map(|v| {
                Literal::from_json(v).ok_or_else(|| invalid(format!("{v} is not a scalar value")))
            })
            .collect()
    };

    let compare = |func: Func| -> Result<Expr, QueryError> {
        Ok(Expr::binary(func, lhs.clone(), literal()?))
    };

    Ok(match f.operator {
        Op::Eq => compare(Func::Equal)?,
        Op::Ne => compare(Func::NotEqual)?,
        Op::Lt => compare(Func::LessThan)?,
        Op::Lte => compare(Func::LessThanOrEqual)?,
        Op::Gt => compare(Func::GreaterThan)?,
        Op::Gte => compare(Func::GreaterThanOrEqual)?,
        Op::In => Expr::InList {
            expr: Box::new(lhs),
            values: values()?,
        },
        Op::NotIn => Expr::call(
            Func::Not,
            vec![Expr::InList {
                expr: Box::new(lhs),
                values: values()?,
            }],
        ),
        Op::IsNull => Expr::call(Func::IsNull, vec![lhs]),
        Op::IsNotNull => Expr::call(Func::IsNotNull, vec![lhs]),
        Op::EqField => {
            let rhs = f.rhs_field.as_ref().ok_or_else(|| {
                invalid("eq_field is missing its right-hand field binding".to_string())
            })?;
            Expr::binary(Func::Equal, lhs, qualify_column(field_expr(rhs), qualify))
        }
    })
}

fn qualify_column(expr: Expr, qualify: bool) -> Expr {
    if qualify {
        expr
    } else if let Expr::Column(c) = expr {
        Expr::column("", c.column)
    } else {
        expr
    }
}

/// Build a plan: scan (and joins) → optional combined filter → aggregate.
pub fn build_logical_plan(
    bound: &BoundQuery,
    model: &CompiledModel,
) -> Result<LogicalPlan, QueryError> {
    let qualify = bound.sources.len() > 1;

    let root_src = bound
        .sources
        .get(&bound.root_dataset)
        .cloned()
        .ok_or_else(|| QueryError::DatasetNotFound(bound.root_dataset.clone()))?;

    let root_cols = dataset_columns(model, &bound.root_dataset)?;

    let mut plan = LogicalPlan::Scan {
        source: root_src,
        dataset: bound.root_dataset.clone(),
        columns: root_cols,
    };

    for j in &bound.joins {
        let right_src = bound
            .sources
            .get(&j.right_dataset)
            .cloned()
            .ok_or_else(|| QueryError::DatasetNotFound(j.right_dataset.clone()))?;
        let right_cols = dataset_columns(model, &j.right_dataset)?;
        let right = LogicalPlan::Scan {
            source: right_src,
            dataset: j.right_dataset.clone(),
            columns: right_cols,
        };
        plan = LogicalPlan::Join {
            left: Box::new(plan),
            right: Box::new(right),
            on: j.on.clone(),
        };
    }

    let after_filter = if bound.filters.is_empty() {
        plan
    } else {
        let parts: Vec<Expr> = bound
            .filters
            .iter()
            .map(|f| filter_to_expr(f, qualify))
            .collect::<Result<_, _>>()?;
        LogicalPlan::Filter {
            input: Box::new(plan),
            predicate: Expr::and(parts),
        }
    };

    let mut group_by = Vec::with_capacity(bound.group_by.len());
    for f in &bound.group_by {
        group_by.push(qualify_column(field_expr(f), qualify));
    }

    let metrics: Vec<NamedExpr> = bound
        .metrics
        .iter()
        .map(|m| NamedExpr::new(m.reference(), m.expr.clone()))
        .collect();

    check_fan_out(&metrics, &bound.fanned_datasets)?;

    let Split {
        aggregates,
        projection,
    } = split_composite_metrics(&group_by, metrics);

    let aggregate = LogicalPlan::Aggregate {
        input: Box::new(after_filter),
        group_by: group_by.clone(),
        aggregates,
    };

    let plan = match projection {
        None => aggregate,
        Some(metric_exprs) => {
            // Grouping columns pass through, but the node lists them so its
            // output schema is readable from the node alone.
            let mut exprs: Vec<NamedExpr> = group_by
                .iter()
                .map(|e| NamedExpr::new(output_name(e), e.clone()))
                .collect();
            exprs.extend(metric_exprs);
            LogicalPlan::Project {
                input: Box::new(aggregate),
                exprs,
            }
        }
    };

    // Ordering goes above the projection, so it can name metric outputs.
    let plan = match sort_keys(&plan, &bound.order_by)? {
        keys if keys.is_empty() => plan,
        keys => LogicalPlan::Sort {
            input: Box::new(plan),
            keys,
        },
    };

    let plan = if bound.limit.is_some() || bound.offset.unwrap_or(0) > 0 {
        LogicalPlan::Fetch {
            input: Box::new(plan),
            offset: bound.offset.unwrap_or(0),
            limit: bound.limit,
        }
    } else {
        plan
    };

    check_plan_columns(&plan)?;
    Ok(plan)
}

/// Resolve each ordering term against what the plan actually produces.
fn sort_keys(plan: &LogicalPlan, order_by: &[OrderBy]) -> Result<Vec<SortKey>, QueryError> {
    if order_by.is_empty() {
        return Ok(Vec::new());
    }
    let schema = plan.output_schema();
    order_by
        .iter()
        .map(|o| {
            // A metric output carries no dataset and is named by its
            // reference, so a dataset-scoped `orders.total` matches whole.
            // Otherwise `dataset.field`, or a bare name. A field and a metric
            // of one dataset never share a name, so the two cannot collide.
            let (dataset, name) = match o.reference.split_once('.') {
                Some((d, n)) if !d.is_empty() && !n.is_empty() => (d, n),
                _ => ("", o.reference.as_str()),
            };
            let index = find_column(&schema, "", &o.reference)
                .or_else(|| find_column(&schema, dataset, name))
                .ok_or_else(|| QueryError::OrderByNotSelected {
                    reference: o.reference.clone(),
                    available: schema
                        .iter()
                        .map(|c| {
                            if c.dataset.is_empty() {
                                c.name.clone()
                            } else {
                                format!("{}.{}", c.dataset, c.name)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                })?;
            let col = &schema[index];
            Ok(SortKey {
                expr: Expr::column(&col.dataset, &col.name),
                direction: o.direction,
                nulls: o.null_order(),
            })
        })
        .collect()
}

/// Name a pass-through expression gets as an output column.
fn output_name(expr: &Expr) -> String {
    match expr {
        Expr::Column(c) if !c.dataset.is_empty() => format!("{}.{}", c.dataset, c.column),
        Expr::Column(c) => c.column.clone(),
        _ => "expr".to_string(),
    }
}

/// Verify every column reference resolves against the schema of the node's
/// input, so emission cannot fail on a lookup.
///
/// [`build_logical_plan`] runs this on the plans it produces; it is public so
/// a hand-built plan can be checked before emission.
pub fn check_plan_columns(plan: &LogicalPlan) -> Result<(), QueryError> {
    let (input, exprs): (&LogicalPlan, Vec<&Expr>) = match plan {
        LogicalPlan::Scan { .. } => return Ok(()),
        LogicalPlan::Join { left, right, .. } => {
            check_plan_columns(left)?;
            return check_plan_columns(right);
        }
        LogicalPlan::Filter { input, predicate } => (input, vec![predicate]),
        LogicalPlan::Sort { input, keys } => (input, keys.iter().map(|k| &k.expr).collect()),
        LogicalPlan::Fetch { input, .. } => (input, vec![]),
        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => (
            input,
            group_by
                .iter()
                .chain(aggregates.iter().map(|a| &a.expr))
                .collect(),
        ),
        LogicalPlan::Project { input, exprs } => (input, exprs.iter().map(|e| &e.expr).collect()),
    };

    let schema = input.output_schema();
    for expr in exprs {
        check_expr(expr, &schema)?;
    }
    check_plan_columns(input)
}

fn check_expr(expr: &Expr, schema: &[SchemaColumn]) -> Result<(), QueryError> {
    if let Expr::Column(c) = expr {
        if find_column(schema, &c.dataset, &c.column).is_none() {
            return Err(QueryError::ColumnNotInScope {
                column: output_name(expr),
                available: schema
                    .iter()
                    .map(|c| {
                        if c.dataset.is_empty() {
                            c.name.clone()
                        } else {
                            format!("{}.{}", c.dataset, c.name)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
    }
    for child in expr.children() {
        check_expr(child, schema)?;
    }
    Ok(())
}

/// Refuse a metric the join would inflate.
///
/// A join repeats the rows of its one side, so a `SUM` over them counts each
/// one once per matching row on the many side. The correct plan aggregates
/// that side before joining, which this crate's IR cannot express yet — so
/// rather than return a number that is quietly too large, say so.
///
/// `MIN`, `MAX` and any `DISTINCT` aggregate are unaffected by repeated rows
/// and stay allowed.
fn check_fan_out(
    metrics: &[NamedExpr],
    fanned: &std::collections::BTreeSet<String>,
) -> Result<(), QueryError> {
    if fanned.is_empty() {
        return Ok(());
    }
    for metric in metrics {
        if let Some(err) = first_inflated(&metric.expr, &metric.name, fanned) {
            return Err(err);
        }
    }
    Ok(())
}

fn first_inflated(
    expr: &Expr,
    metric: &str,
    fanned: &std::collections::BTreeSet<String>,
) -> Option<QueryError> {
    if let Expr::Agg {
        func,
        distinct,
        args,
    } = expr
    {
        // DISTINCT collapses the repeats, so it reads the same either way.
        if !distinct && crate::catalog::def(*func).is_duplicate_sensitive() {
            let touched: std::collections::BTreeSet<String> = args
                .iter()
                .flat_map(|a| a.referenced_datasets())
                .filter(|d| fanned.contains(d))
                .collect();
            if let Some(dataset) = touched.into_iter().next() {
                return Some(QueryError::FanOut {
                    metric: metric.to_string(),
                    dataset,
                    agg: crate::catalog::def(*func).name_for_sql().to_string(),
                });
            }
        }
    }
    expr.children()
        .into_iter()
        .find_map(|c| first_inflated(c, metric, fanned))
}

/// Metrics rearranged so every aggregate stands alone.
struct Split {
    /// One bare aggregate per measure.
    aggregates: Vec<NamedExpr>,
    /// Output columns over those measures, or `None` when every metric was
    /// already a bare aggregate and no projection is needed.
    projection: Option<Vec<NamedExpr>>,
}

/// Lift aggregate subtrees out of composite metrics.
///
/// A Substrait measure is a single aggregate call, so `SUM(a) / COUNT(b)`
/// becomes two measures plus a projection that divides them. When no metric is
/// composite the plan keeps its simpler shape.
fn split_composite_metrics(group_by: &[Expr], metrics: Vec<NamedExpr>) -> Split {
    if metrics.iter().all(|m| matches!(m.expr, Expr::Agg { .. })) {
        return Split {
            aggregates: metrics,
            projection: None,
        };
    }

    let prefix = fresh_measure_prefix(group_by);
    let mut aggregates: Vec<NamedExpr> = Vec::new();
    let mut projection = Vec::with_capacity(metrics.len());
    for m in metrics {
        let expr = extract_aggregates(&m.expr, &prefix, &mut aggregates);
        projection.push(NamedExpr::new(m.name, expr));
    }
    Split {
        aggregates,
        projection: Some(projection),
    }
}

/// Replace each aggregate subtree with a reference to a synthesized measure
/// column, collecting the aggregates. Identical aggregates share one measure.
fn extract_aggregates(expr: &Expr, prefix: &str, out: &mut Vec<NamedExpr>) -> Expr {
    if matches!(expr, Expr::Agg { .. }) {
        if let Some(existing) = out.iter().find(|m| m.expr == *expr) {
            return Expr::column("", existing.name.clone());
        }
        let name = format!("{prefix}{}", out.len());
        out.push(NamedExpr::new(&name, expr.clone()));
        return Expr::column("", name);
    }
    expr.map_children(&mut |child| extract_aggregates(child, prefix, out))
}

/// A measure-name prefix that cannot collide with a grouping column.
fn fresh_measure_prefix(group_by: &[Expr]) -> String {
    let taken: Vec<&str> = group_by
        .iter()
        .filter_map(|e| match e {
            Expr::Column(ColumnRef { column, .. }) => Some(column.as_str()),
            _ => None,
        })
        .collect();
    let mut prefix = "__agg".to_string();
    while taken.iter().any(|c| c.starts_with(&prefix)) {
        prefix.push('_');
    }
    prefix
}
