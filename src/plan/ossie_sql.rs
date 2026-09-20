//! Lower Ossie SQL expression strings into [`Expr`](super::expr::Expr).
//!
//! [`sqlparser`] does the parsing — text to a general-purpose SQL AST. This
//! module does the *lowering*: mapping that AST onto the narrow planner IR, and
//! rejecting everything outside the Ossie expression profile.
//!
//! The profile (`core-spec/expression_language.md`) allows scalar and aggregate
//! expressions only: no `SELECT`/`FROM`/`JOIN`, no subqueries, no CTEs. A SQL
//! parser happily accepts those, so the gate lives here.
//!
//! # Two resolution modes
//!
//! A bare identifier means different things depending on where the expression
//! sits, so there are two entry points:
//!
//! - [`lower_field_expression`] — a field's *own* definition. Identifiers name
//!   physical columns of the owning dataset. Field references are **not**
//!   resolved here: a field whose expression is `id` would otherwise resolve to
//!   itself forever.
//! - [`lower_metric_expression`] — a metric. `dataset.field` names a *field*,
//!   resolved through the model to that field's expression (itself lowered in
//!   field-definition mode).

use sqlparser::ast;
use sqlparser::dialect::{
    AnsiDialect, BigQueryDialect, DatabricksDialect, Dialect as SqlParserDialect, SnowflakeDialect,
};
use sqlparser::parser::Parser;

use crate::catalog::{self, Func};
use crate::model::{Dialect, SemanticModel};
use crate::plan::expr::{Expr, Literal};
use crate::resolver::QueryError;

/// Resolution context for one lowering.
struct Ctx<'a> {
    /// `Some` in metric mode: `dataset.field` resolves through the model.
    /// `None` in field-definition mode: identifiers are physical columns.
    model: Option<&'a SemanticModel>,
    /// Dataset that unqualified identifiers belong to.
    dataset: Option<&'a str>,
    /// Dialect the source text was written in, for nested lowering.
    dialect: Dialect,
}

/// The sqlparser dialect to parse an Ossie dialect's text with.
///
/// The four SQL dialects in the Ossie enum all have a sqlparser counterpart.
/// The rest (`MDX`, `TABLEAU`, `MAQL`, `SIGMA`, `THOUGHTSPOT`, `DAX`) are not
/// SQL and cannot be parsed here.
fn sqlparser_dialect(dialect: Dialect) -> Result<Box<dyn SqlParserDialect>, QueryError> {
    Ok(match dialect {
        Dialect::AnsiSql => Box::new(AnsiDialect {}),
        Dialect::Snowflake => Box::new(SnowflakeDialect {}),
        Dialect::Databricks => Box::new(DatabricksDialect {}),
        Dialect::BigQuery => Box::new(BigQueryDialect {}),
        // Not a SQL dialect — there is nothing sensible to parse it with.
        Dialect::Mdx
        | Dialect::Tableau
        | Dialect::Maql
        | Dialect::Sigma
        | Dialect::ThoughtSpot
        | Dialect::Dax => {
            return Err(QueryError::UnsupportedExpression(format!(
                "dialect {dialect:?} is not a SQL dialect and cannot be compiled"
            )));
        }
    })
}

/// Whether this Ossie dialect carries SQL text this module can compile.
pub fn is_sql_dialect(dialect: Dialect) -> bool {
    sqlparser_dialect(dialect).is_ok()
}

fn parse(sql: &str, dialect: Dialect) -> Result<ast::Expr, QueryError> {
    let d = sqlparser_dialect(dialect)?;
    let mut parser = Parser::new(d.as_ref())
        .try_with_sql(sql)
        .map_err(|e| QueryError::InvalidExpression(format!("{sql:?}: {e}")))?;
    let expr = parser
        .parse_expr()
        .map_err(|e| QueryError::InvalidExpression(format!("{sql:?}: {e}")))?;
    // `parse_expr` stops at the first token it cannot continue with, so trailing
    // junk would otherwise be silently dropped.
    let rest = parser.peek_token().token;
    if rest != sqlparser::tokenizer::Token::EOF {
        return Err(QueryError::InvalidExpression(format!(
            "{sql:?}: unexpected trailing input at {rest}"
        )));
    }
    Ok(expr)
}

/// Lower a field's own expression. Identifiers are physical columns of `dataset`.
///
/// Rejects aggregates: the spec requires field expressions to be scalar.
pub fn lower_field_expression(
    sql: &str,
    dialect: Dialect,
    dataset: &str,
) -> Result<Expr, QueryError> {
    let ast = parse(sql, dialect)?;
    let ctx = Ctx {
        model: None,
        dataset: Some(dataset),
        dialect,
    };
    let expr = lower(&ast, &ctx)?;
    if expr.contains_aggregate() {
        return Err(QueryError::InvalidExpression(format!(
            "field expression {sql:?} contains an aggregate; fields must be scalar"
        )));
    }
    Ok(expr)
}

/// Lower a metric expression. `dataset.field` resolves through `model`.
///
/// Requires at least one aggregate: the spec defines metrics as aggregate
/// expressions over fields.
pub fn lower_metric_expression(
    sql: &str,
    dialect: Dialect,
    model: &SemanticModel,
) -> Result<Expr, QueryError> {
    let ast = parse(sql, dialect)?;
    let ctx = Ctx {
        model: Some(model),
        dataset: None,
        dialect,
    };
    let expr = lower(&ast, &ctx)?;
    if !expr.contains_aggregate() {
        return Err(QueryError::InvalidExpression(format!(
            "metric expression {sql:?} has no aggregate"
        )));
    }
    Ok(expr)
}

fn lower(e: &ast::Expr, ctx: &Ctx<'_>) -> Result<Expr, QueryError> {
    match e {
        ast::Expr::Nested(inner) => lower(inner, ctx),
        ast::Expr::Identifier(id) => resolve_identifier(std::slice::from_ref(id), ctx),
        ast::Expr::CompoundIdentifier(parts) => resolve_identifier(parts, ctx),
        ast::Expr::Value(v) => Ok(Expr::Literal(lower_literal(&v.value)?)),
        ast::Expr::UnaryOp { op, expr } => lower_unary(op, expr, ctx),
        ast::Expr::BinaryOp { left, op, right } => lower_binary(left, op, right, ctx),
        ast::Expr::IsNull(inner) => Ok(Expr::call(Func::IsNull, vec![lower(inner, ctx)?])),
        ast::Expr::IsNotNull(inner) => Ok(Expr::call(Func::IsNotNull, vec![lower(inner, ctx)?])),
        ast::Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => lower_case(operand.as_deref(), conditions, else_result.as_deref(), ctx),
        ast::Expr::Function(f) => lower_function(f, ctx),
        ast::Expr::InList {
            expr,
            list,
            negated,
        } => lower_in_list(expr, list, *negated, ctx),
        ast::Expr::Between {
            expr,
            negated,
            low,
            high,
        } => lower_between(expr, *negated, low, high, ctx),

        // Outside the Ossie expression profile.
        ast::Expr::Subquery(_) | ast::Expr::InSubquery { .. } | ast::Expr::Exists { .. } => {
            Err(QueryError::UnsupportedExpression(
                "subqueries are not permitted in Ossie expressions".to_string(),
            ))
        }
        other => Err(QueryError::UnsupportedExpression(format!(
            "unsupported expression: {other}"
        ))),
    }
}

/// Resolve an identifier path to a column or a field's expression.
fn resolve_identifier(parts: &[ast::Ident], ctx: &Ctx<'_>) -> Result<Expr, QueryError> {
    let names: Vec<String> = parts.iter().map(normalize_ident).collect();
    let (dataset, name) = match names.as_slice() {
        [name] => {
            let ds = ctx.dataset.ok_or_else(|| {
                QueryError::InvalidFieldRef(format!("unqualified identifier {name:?}"))
            })?;
            (ds.to_string(), name.clone())
        }
        [dataset, name] => (dataset.clone(), name.clone()),
        _ => {
            return Err(QueryError::InvalidFieldRef(format!(
                "identifier {:?} has more than two parts",
                names.join(".")
            )));
        }
    };

    let Some(model) = ctx.model else {
        // Field-definition mode: this is a physical column, full stop.
        return Ok(Expr::column(dataset, name));
    };

    // Metric mode: the identifier names a field, which carries its own expression.
    let ds = model
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .ok_or_else(|| QueryError::DatasetNotFound(dataset.clone()))?;
    let field = ds
        .fields
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| QueryError::FieldNotFound(format!("{dataset}.{name}")))?;

    let (field_dialect, sql) = super::pick_expression_dialect(&field.expression)
        .ok_or(QueryError::MissingExpressionSql)?;
    let dialect = if is_sql_dialect(field_dialect) {
        field_dialect
    } else {
        ctx.dialect
    };
    lower_field_expression(&sql, dialect, &dataset)
}

/// Ossie identifier rules: case-insensitive unless quoted, max 128 characters.
fn normalize_ident(id: &ast::Ident) -> String {
    match id.quote_style {
        Some(_) => id.value.clone(),
        None => id.value.to_lowercase(),
    }
}

fn lower_literal(v: &ast::Value) -> Result<Literal, QueryError> {
    Ok(match v {
        ast::Value::Null => Literal::Null,
        ast::Value::Boolean(b) => Literal::Bool(*b),
        ast::Value::Number(n, _) => n
            .parse::<i64>()
            .map(Literal::Int64)
            .or_else(|_| n.parse::<f64>().map(Literal::Float64))
            .map_err(|_| QueryError::InvalidExpression(format!("bad numeric literal {n:?}")))?,
        ast::Value::SingleQuotedString(s)
        | ast::Value::DoubleQuotedString(s)
        | ast::Value::EscapedStringLiteral(s) => Literal::String(s.clone()),
        other => {
            return Err(QueryError::UnsupportedExpression(format!(
                "unsupported literal: {other}"
            )));
        }
    })
}

fn lower_unary(op: &ast::UnaryOperator, e: &ast::Expr, ctx: &Ctx<'_>) -> Result<Expr, QueryError> {
    let inner = lower(e, ctx)?;
    match op {
        ast::UnaryOperator::Not => Ok(Expr::call(Func::Not, vec![inner])),
        ast::UnaryOperator::Plus => Ok(inner),
        ast::UnaryOperator::Minus => Ok(Expr::binary(
            Func::Subtract,
            Expr::Literal(Literal::Int64(0)),
            inner,
        )),
        other => Err(QueryError::UnsupportedExpression(format!(
            "unsupported unary operator {other}"
        ))),
    }
}

fn lower_binary(
    l: &ast::Expr,
    op: &ast::BinaryOperator,
    r: &ast::Expr,
    ctx: &Ctx<'_>,
) -> Result<Expr, QueryError> {
    use ast::BinaryOperator as B;
    // AND/OR flatten into the variadic nodes the IR and emitters expect.
    if matches!(op, B::And | B::Or) {
        let mut parts = Vec::new();
        flatten_logical(l, op, ctx, &mut parts)?;
        flatten_logical(r, op, ctx, &mut parts)?;
        return Ok(match op {
            B::And => Expr::and(parts),
            _ => Expr::or(parts),
        });
    }
    if matches!(op, B::StringConcat) {
        let mut parts = Vec::new();
        flatten_concat(l, ctx, &mut parts)?;
        flatten_concat(r, ctx, &mut parts)?;
        return Ok(Expr::call(Func::Concat, parts));
    }

    let func = match op {
        B::Eq => Func::Equal,
        B::NotEq => Func::NotEqual,
        B::Lt => Func::LessThan,
        B::LtEq => Func::LessThanOrEqual,
        B::Gt => Func::GreaterThan,
        B::GtEq => Func::GreaterThanOrEqual,
        B::Plus => Func::Add,
        B::Minus => Func::Subtract,
        B::Multiply => Func::Multiply,
        B::Divide => Func::Divide,
        other => {
            return Err(QueryError::UnsupportedExpression(format!(
                "unsupported operator {other}"
            )));
        }
    };
    Ok(Expr::binary(func, lower(l, ctx)?, lower(r, ctx)?))
}

fn flatten_logical(
    e: &ast::Expr,
    op: &ast::BinaryOperator,
    ctx: &Ctx<'_>,
    out: &mut Vec<Expr>,
) -> Result<(), QueryError> {
    if let ast::Expr::BinaryOp {
        left,
        op: inner,
        right,
    } = e
    {
        if inner == op {
            flatten_logical(left, op, ctx, out)?;
            flatten_logical(right, op, ctx, out)?;
            return Ok(());
        }
    }
    out.push(lower(e, ctx)?);
    Ok(())
}

fn flatten_concat(e: &ast::Expr, ctx: &Ctx<'_>, out: &mut Vec<Expr>) -> Result<(), QueryError> {
    if let ast::Expr::BinaryOp {
        left,
        op: ast::BinaryOperator::StringConcat,
        right,
    } = e
    {
        flatten_concat(left, ctx, out)?;
        flatten_concat(right, ctx, out)?;
        return Ok(());
    }
    out.push(lower(e, ctx)?);
    Ok(())
}

fn lower_case(
    operand: Option<&ast::Expr>,
    conditions: &[ast::CaseWhen],
    else_result: Option<&ast::Expr>,
    ctx: &Ctx<'_>,
) -> Result<Expr, QueryError> {
    let operand = operand.map(|o| lower(o, ctx)).transpose()?;
    let mut when_then = Vec::with_capacity(conditions.len());
    for cw in conditions {
        let cond = lower(&cw.condition, ctx)?;
        // `CASE x WHEN v THEN …` is shorthand for `CASE WHEN x = v THEN …`.
        let cond = match &operand {
            Some(o) => Expr::binary(Func::Equal, o.clone(), cond),
            None => cond,
        };
        when_then.push((cond, lower(&cw.result, ctx)?));
    }
    let else_expr = match else_result {
        Some(e) => lower(e, ctx)?,
        None => Expr::Literal(Literal::Null),
    };
    Ok(Expr::Case {
        when_then,
        else_expr: Box::new(else_expr),
    })
}

fn lower_in_list(
    expr: &ast::Expr,
    list: &[ast::Expr],
    negated: bool,
    ctx: &Ctx<'_>,
) -> Result<Expr, QueryError> {
    let lhs = lower(expr, ctx)?;
    let mut values = Vec::with_capacity(list.len());
    for item in list {
        match item {
            ast::Expr::Value(v) => values.push(lower_literal(&v.value)?),
            other => {
                return Err(QueryError::UnsupportedExpression(format!(
                    "IN list accepts literals only, got {other}"
                )));
            }
        }
    }
    let in_expr = Expr::InList {
        expr: Box::new(lhs),
        values,
    };
    Ok(if negated {
        Expr::call(Func::Not, vec![in_expr])
    } else {
        in_expr
    })
}

/// `a BETWEEN lo AND hi` desugars to `a >= lo AND a <= hi`.
fn lower_between(
    expr: &ast::Expr,
    negated: bool,
    low: &ast::Expr,
    high: &ast::Expr,
    ctx: &Ctx<'_>,
) -> Result<Expr, QueryError> {
    let e = lower(expr, ctx)?;
    let bounds = Expr::and(vec![
        Expr::binary(Func::GreaterThanOrEqual, e.clone(), lower(low, ctx)?),
        Expr::binary(Func::LessThanOrEqual, e, lower(high, ctx)?),
    ]);
    Ok(if negated {
        Expr::call(Func::Not, vec![bounds])
    } else {
        bounds
    })
}

fn lower_function(f: &ast::Function, ctx: &Ctx<'_>) -> Result<Expr, QueryError> {
    let name = f
        .name
        .0
        .last()
        .map(|p| p.to_string().trim_matches('"').to_uppercase())
        .unwrap_or_default();

    if f.over.is_some() {
        return Err(QueryError::UnsupportedExpression(format!(
            "window function {name} is not supported yet"
        )));
    }
    if f.filter.is_some() || !f.within_group.is_empty() {
        return Err(QueryError::UnsupportedExpression(format!(
            "{name} with FILTER / WITHIN GROUP is not supported"
        )));
    }

    let (args, distinct, star) = function_args(f, &name)?;

    // The catalog is the whole vocabulary: a function is callable from a model
    // expression exactly when it has a row there.
    let def = catalog::by_sql_name(&name).ok_or_else(|| {
        QueryError::UnsupportedExpression(format!("function {name} is not supported"))
    })?;

    // `DATEDIFF`'s unit is a bare keyword rather than a value, so it is
    // stripped before the arguments are lowered.
    let args = if def.func == Func::DateDiff {
        date_diff_args(&name, &args)?
    } else {
        args
    };

    if star {
        if def.func != Func::Count {
            return Err(QueryError::UnsupportedExpression(format!(
                "{name}(*) is not valid"
            )));
        }
        return Ok(Expr::Agg {
            func: def.func,
            distinct,
            args: vec![],
        });
    }

    if !def.arity.accepts(args.len()) {
        return Err(QueryError::InvalidExpression(format!(
            "{name} does not take {} argument(s)",
            args.len()
        )));
    }

    let lowered = args
        .iter()
        .map(|a| lower(a, ctx))
        .collect::<Result<Vec<_>, _>>()?;

    if def.is_aggregate() {
        if lowered.iter().any(Expr::contains_aggregate) {
            return Err(QueryError::UnsupportedExpression(format!(
                "nested aggregate inside {name} is not supported"
            )));
        }
        return Ok(Expr::Agg {
            func: def.func,
            distinct,
            args: lowered,
        });
    }

    if distinct {
        return Err(QueryError::InvalidExpression(format!(
            "DISTINCT is only valid in an aggregate, not {name}"
        )));
    }
    Ok(Expr::Call {
        func: def.func,
        args: lowered,
    })
}

/// Positional arguments, `DISTINCT` flag, and whether the sole argument is `*`.
fn function_args(
    f: &ast::Function,
    name: &str,
) -> Result<(Vec<ast::Expr>, bool, bool), QueryError> {
    let list = match &f.args {
        ast::FunctionArguments::None => return Ok((vec![], false, false)),
        ast::FunctionArguments::List(list) => list,
        ast::FunctionArguments::Subquery(_) => {
            return Err(QueryError::UnsupportedExpression(
                "subqueries are not permitted in Ossie expressions".to_string(),
            ));
        }
    };

    let distinct = matches!(
        list.duplicate_treatment,
        Some(ast::DuplicateTreatment::Distinct)
    );

    let mut args = Vec::with_capacity(list.args.len());
    let mut star = false;
    for a in &list.args {
        match a {
            ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(e)) => args.push(e.clone()),
            ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Wildcard) => star = true,
            other => {
                return Err(QueryError::UnsupportedExpression(format!(
                    "{name} has an unsupported argument form: {other}"
                )));
            }
        }
    }
    Ok((args, distinct, star))
}

/// Strip `DATEDIFF`'s optional unit argument, leaving `(end, start)`.
///
/// Only the `(end, start)` operand order is accepted. Snowflake spells this
/// with the unit first (`DATEDIFF(DAY, start, end)`); that form is reported
/// rather than silently reinterpreted, since the operands would swap.
fn date_diff_args(name: &str, args: &[ast::Expr]) -> Result<Vec<ast::Expr>, QueryError> {
    if let Some(first) = args.first().and_then(date_part_name) {
        return Err(QueryError::UnsupportedExpression(format!(
            "{name} with the unit first ({first}, …) is not supported; \
             write {name}(end, start) or {name}(end, start, DAY)"
        )));
    }
    match args {
        [end, start] => Ok(vec![end.clone(), start.clone()]),
        [end, start, unit] => {
            let unit_name = date_part_name(unit).ok_or_else(|| {
                QueryError::InvalidExpression(format!("{name} unit must be a date part"))
            })?;
            if unit_name != "DAY" {
                return Err(QueryError::UnsupportedExpression(format!(
                    "{name} unit {unit_name} is not supported; only DAY"
                )));
            }
            Ok(vec![end.clone(), start.clone()])
        }
        _ => Err(QueryError::InvalidExpression(format!(
            "{name} takes 2 or 3 arguments, got {}",
            args.len()
        ))),
    }
}

/// The uppercased name if `e` is a bare identifier naming a date part.
fn date_part_name(e: &ast::Expr) -> Option<String> {
    let ast::Expr::Identifier(id) = e else {
        return None;
    };
    if id.quote_style.is_some() {
        return None;
    }
    let upper = id.value.to_uppercase();
    matches!(
        upper.as_str(),
        "YEAR"
            | "QUARTER"
            | "MONTH"
            | "WEEK"
            | "DAY"
            | "HOUR"
            | "MINUTE"
            | "SECOND"
            | "MILLISECOND"
            | "MICROSECOND"
    )
    .then_some(upper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::from_yaml_str;
    use crate::plan::{SqlCtx, format_expr};
    use crate::sql_dialect::Ansi;

    const MODEL: &str = r#"
version: "0.2.0.dev0"
name: m
datasets:
  - name: orders
    source: s.orders
    fields:
      - name: amount
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: amount }]
      - name: qty
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: qty }]
      - name: full_name
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: "first_name || ' ' || last_name" }]
  - name: customers
    source: s.customers
    fields:
      - name: id
        expression:
          dialects: [{ dialect: ANSI_SQL, expression: id }]
"#;

    fn model() -> crate::model::SemanticModel {
        from_yaml_str(MODEL).expect("parse model").model
    }

    /// Round-trip through the formatter: proves the tree is structured, not `Expr::Sql`.
    fn metric_sql(sql: &str) -> String {
        let m = model();
        let e = lower_metric_expression(sql, Dialect::AnsiSql, &m).expect(sql);
        format_expr(
            &e,
            SqlCtx {
                dialect: &Ansi,
                qualify: false,
            },
        )
    }

    fn field_expr(sql: &str) -> Expr {
        lower_field_expression(sql, Dialect::AnsiSql, "orders").expect(sql)
    }

    #[test]
    fn simple_aggregate() {
        assert_eq!(metric_sql("SUM(orders.amount)"), "SUM(amount)");
    }

    #[test]
    fn count_star_and_distinct() {
        assert_eq!(metric_sql("COUNT(*)"), "COUNT(*)");
        assert_eq!(
            metric_sql("COUNT(DISTINCT customers.id)"),
            "COUNT(DISTINCT id)"
        );
    }

    #[test]
    fn ratio_of_two_aggregates() {
        assert_eq!(
            metric_sql("SUM(orders.amount) / COUNT(DISTINCT customers.id)"),
            "(SUM(amount) / COUNT(DISTINCT id))"
        );
    }

    #[test]
    fn nullif_guarded_ratio() {
        assert_eq!(
            metric_sql("SUM(orders.amount) / NULLIF(SUM(orders.qty), 0)"),
            "(SUM(amount) / NULLIF(SUM(qty), 0))"
        );
    }

    #[test]
    fn aggregate_over_arithmetic() {
        assert_eq!(
            metric_sql("SUM(orders.amount * orders.qty)"),
            "SUM((amount * qty))"
        );
    }

    #[test]
    fn aggregate_over_case() {
        assert_eq!(
            metric_sql("SUM(CASE WHEN orders.qty > 100 THEN orders.amount ELSE 0 END)"),
            "SUM(CASE WHEN qty > 100 THEN amount ELSE 0 END)"
        );
    }

    /// A metric referencing a computed field inlines that field's definition.
    #[test]
    fn field_reference_expands_to_its_expression() {
        assert_eq!(
            metric_sql("COUNT(DISTINCT orders.full_name)"),
            "COUNT(DISTINCT first_name || ' ' || last_name)"
        );
    }

    #[test]
    fn precedence_is_preserved() {
        // `*` binds tighter than `+`, so the tree must be Add(a, Mul(b, c)).
        let m = model();
        let e = lower_metric_expression(
            "SUM(orders.amount + orders.qty * orders.qty)",
            Dialect::AnsiSql,
            &m,
        )
        .expect("lower");
        let Expr::Agg { args, .. } = &e else {
            panic!("expected aggregate, got {e:?}");
        };
        let Expr::Call {
            func: Func::Add,
            args: add_args,
        } = &args[0]
        else {
            panic!("expected an addition at the top, got {:?}", args[0]);
        };
        assert!(
            matches!(
                &add_args[1],
                Expr::Call {
                    func: Func::Multiply,
                    ..
                }
            ),
            "`*` must bind tighter than `+`: {:?}",
            add_args[1]
        );
    }

    #[test]
    fn between_desugars_to_bounds() {
        let e = field_expr("qty BETWEEN 1 AND 10");
        assert_eq!(
            format_expr(
                &e,
                SqlCtx {
                    dialect: &Ansi,
                    qualify: false
                }
            ),
            "qty >= 1 AND qty <= 10"
        );
    }

    #[test]
    fn concat_flattens() {
        let e = field_expr("first_name || ' ' || last_name");
        assert!(
            matches!(&e, Expr::Call { func: Func::Concat, args } if args.len() == 3),
            "{e:?}"
        );
    }

    #[test]
    fn field_expressions_reject_aggregates() {
        let err = lower_field_expression("SUM(amount)", Dialect::AnsiSql, "orders")
            .expect_err("aggregate in a field must be rejected");
        assert!(err.to_string().contains("must be scalar"), "{err}");
    }

    #[test]
    fn metric_expressions_require_an_aggregate() {
        let err = lower_metric_expression("orders.amount", Dialect::AnsiSql, &model())
            .expect_err("scalar metric must be rejected");
        assert!(err.to_string().contains("no aggregate"), "{err}");
    }

    #[test]
    fn subqueries_are_rejected() {
        let err = lower_metric_expression("SUM((SELECT 1))", Dialect::AnsiSql, &model())
            .expect_err("subquery must be rejected");
        assert!(err.to_string().contains("subqueries"), "{err}");
    }

    #[test]
    fn window_functions_report_their_own_limitation() {
        let err = lower_metric_expression(
            "RANK() OVER (ORDER BY orders.amount DESC)",
            Dialect::AnsiSql,
            &model(),
        )
        .expect_err("window function must be rejected");
        assert!(err.to_string().contains("window function"), "{err}");
    }

    #[test]
    fn unknown_references_are_errors_not_passthrough() {
        let err = lower_metric_expression("SUM(orders.nope)", Dialect::AnsiSql, &model())
            .expect_err("unknown field must be rejected");
        assert!(err.to_string().contains("field not found"), "{err}");

        let err = lower_metric_expression("SUM(nope.amount)", Dialect::AnsiSql, &model())
            .expect_err("unknown dataset must be rejected");
        assert!(err.to_string().contains("dataset not found"), "{err}");
    }

    #[test]
    fn unsupported_functions_are_named_in_the_error() {
        let err = lower_metric_expression(
            "SUM(REGEXP_EXTRACT(orders.amount, 'x'))",
            Dialect::AnsiSql,
            &model(),
        )
        .expect_err("a function with no catalog row must be rejected");
        assert!(err.to_string().contains("REGEXP_EXTRACT"), "{err}");
    }

    /// A function is callable as soon as it has a catalog row — no lowering,
    /// formatting, or emission code of its own.
    #[test]
    fn catalog_rows_are_immediately_callable() {
        assert_eq!(metric_sql("SUM(SQRT(orders.amount))"), "SUM(SQRT(amount))");
        assert_eq!(metric_sql("MAX(ABS(orders.amount))"), "MAX(ABS(amount))");
        assert_eq!(
            metric_sql("SUM(POWER(orders.amount, 2))"),
            "SUM(POWER(amount, 2))"
        );
    }

    #[test]
    fn arity_is_enforced_from_the_catalog() {
        let err = lower_metric_expression("SUM(NULLIF(orders.amount))", Dialect::AnsiSql, &model())
            .expect_err("NULLIF takes two arguments");
        assert!(err.to_string().contains("does not take 1"), "{err}");
    }

    #[test]
    fn trailing_junk_is_rejected() {
        let err = lower_field_expression("amount)) garbage", Dialect::AnsiSql, "orders")
            .expect_err("trailing input must be rejected");
        assert!(err.to_string().contains("trailing"), "{err}");
    }

    #[test]
    fn date_diff_lowers_to_the_semantic_node() {
        let e = field_expr("DATEDIFF(CURRENT_DATE, amount)");
        assert!(
            matches!(
                &e,
                Expr::Call {
                    func: Func::DateDiff,
                    ..
                }
            ),
            "{e:?}"
        );
        // The three-argument BigQuery spelling means the same thing.
        let e = field_expr("DATE_DIFF(CURRENT_DATE, amount, DAY)");
        assert!(
            matches!(
                &e,
                Expr::Call {
                    func: Func::DateDiff,
                    ..
                }
            ),
            "{e:?}"
        );
    }

    #[test]
    fn date_diff_rejects_units_it_cannot_model() {
        let err = lower_field_expression(
            "DATE_DIFF(CURRENT_DATE, amount, MONTH)",
            Dialect::AnsiSql,
            "orders",
        )
        .expect_err("MONTH is not representable");
        assert!(err.to_string().contains("only DAY"), "{err}");
    }

    /// Snowflake puts the unit first, which would silently swap the operands.
    #[test]
    fn date_diff_rejects_unit_first_ordering() {
        let err = lower_field_expression(
            "DATEDIFF(DAY, amount, CURRENT_DATE)",
            Dialect::AnsiSql,
            "orders",
        )
        .expect_err("unit-first ordering must be reported");
        assert!(err.to_string().contains("unit first"), "{err}");
    }

    #[test]
    fn non_sql_dialects_are_refused() {
        let err = lower_field_expression("[Measures].[Sales]", Dialect::Mdx, "orders")
            .expect_err("MDX is not SQL");
        assert!(err.to_string().contains("not a SQL dialect"), "{err}");
    }

    #[test]
    fn quoted_identifiers_keep_their_case() {
        let e = field_expr("\"MixedCase\"");
        assert!(
            matches!(&e, Expr::Column(c) if c.column == "MixedCase"),
            "{e:?}"
        );
        let e = field_expr("MixedCase");
        assert!(
            matches!(&e, Expr::Column(c) if c.column == "mixedcase"),
            "{e:?}"
        );
    }
}
