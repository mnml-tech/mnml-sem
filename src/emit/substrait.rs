//! Emit Substrait [`Plan`](substrait::proto::Plan) from a structured [`LogicalPlan`](crate::plan::LogicalPlan).

use substrait::proto::{
    self,
    aggregate_function::AggregationInvocation,
    aggregate_rel::{Grouping, Measure},
    expression::{
        self,
        literal::LiteralType,
        reference_segment::{ReferenceType, StructField},
        ReferenceSegment,
    },
    extensions::{
        simple_extension_declaration::MappingType, SimpleExtensionDeclaration, SimpleExtensionUrn,
    },
    function_argument::ArgType,
    plan_rel::RelType as PlanRelType,
    r#type::{Kind, Nullability},
    read_rel::{NamedTable, ReadType},
    rel::RelType,
    AggregationPhase,
};

use crate::catalog::{self, Func};
use crate::emit::EmitError;
use crate::plan::expr::{ColumnRef, Expr, JoinKey, Literal};
use crate::plan::{LogicalPlan, NamedAggregate};

#[derive(Debug, Clone)]
struct SchemaContext {
    columns: Vec<(String, String)>,
}

impl SchemaContext {
    fn new() -> Self {
        Self {
            columns: Vec::new(),
        }
    }

    fn add_scan(&mut self, alias: &str, columns: &[String]) {
        for col in columns {
            self.columns.push((alias.to_string(), col.clone()));
        }
    }

    fn merge(&mut self, other: SchemaContext) {
        self.columns.extend(other.columns);
    }

    fn find_column(&self, dataset: &str, sql: &str) -> Option<usize> {
        if dataset.is_empty() {
            return self.columns.iter().position(|(_, c)| c == sql);
        }
        self.columns
            .iter()
            .position(|(d, c)| d == dataset && c == sql)
    }

    fn len(&self) -> usize {
        self.columns.len()
    }
}

/// Encode a Substrait plan as protobuf bytes (for engines such as DataFusion).
pub fn encode_plan(plan: &proto::Plan) -> Vec<u8> {
    use prost::Message;
    plan.encode_to_vec()
}

/// Build a Substrait plan from `plan` (aggregate root).
pub fn to_plan(plan: &LogicalPlan) -> Result<proto::Plan, EmitError> {
    let LogicalPlan::Aggregate {
        input,
        group_by,
        aggregates,
    } = plan
    else {
        return Err(EmitError::Unsupported(
            "root must be Aggregate (use planner output)",
        ));
    };

    let mut ctx = SchemaContext::new();
    let input_rel = emit_rel(input, &mut ctx)?;
    let rel = emit_aggregate_rel(input_rel, group_by, aggregates, &mut ctx)?;

    let mut output_names: Vec<String> = group_by.iter().map(output_name_for_expr).collect();
    for a in aggregates {
        output_names.push(a.name.clone());
    }

    Ok(proto::Plan {
        version: Some(proto::Version {
            major_number: 0,
            minor_number: 63,
            patch_number: 0,
            ..Default::default()
        }),
        extension_urns: build_extension_urns(),
        extensions: build_extensions(),
        relations: vec![proto::PlanRel {
            rel_type: Some(PlanRelType::Root(proto::RelRoot {
                input: Some(rel),
                names: output_names,
            })),
        }],
        ..Default::default()
    })
}

fn output_name_for_expr(expr: &Expr) -> String {
    match expr {
        Expr::Column(c) if !c.dataset.is_empty() => format!("{}.{}", c.dataset, c.sql),
        Expr::Column(c) => c.sql.clone(),
        Expr::Sql(s) => s.clone(),
        _ => "expr".to_string(),
    }
}

fn emit_rel(plan: &LogicalPlan, ctx: &mut SchemaContext) -> Result<proto::Rel, EmitError> {
    match plan {
        LogicalPlan::Scan {
            source,
            dataset,
            columns,
        } => emit_scan(source, dataset, columns, ctx),
        LogicalPlan::Join { left, right, on } => emit_join(left, right, on, ctx),
        LogicalPlan::Filter { input, predicate } => emit_filter(input, predicate, ctx),
        LogicalPlan::Aggregate { .. } => Err(EmitError::InvalidPlan(
            "unexpected nested Aggregate in input rel",
        )),
    }
}

fn emit_scan(
    source: &str,
    dataset: &str,
    columns: &[String],
    ctx: &mut SchemaContext,
) -> Result<proto::Rel, EmitError> {
    ctx.add_scan(dataset, columns);

    let types: Vec<proto::Type> = columns.iter().map(|_| string_type()).collect();
    let base_schema = proto::NamedStruct {
        names: columns.to_vec(),
        r#struct: Some(proto::r#type::Struct {
            types,
            type_variation_reference: 0,
            nullability: Nullability::Nullable as i32,
        }),
    };

    let table_names: Vec<String> = source.split('.').map(String::from).collect();

    Ok(proto::Rel {
        rel_type: Some(RelType::Read(Box::new(proto::ReadRel {
            read_type: Some(ReadType::NamedTable(NamedTable {
                names: table_names,
                advanced_extension: None,
            })),
            base_schema: Some(base_schema),
            ..Default::default()
        }))),
    })
}

fn emit_join(
    left: &LogicalPlan,
    right: &LogicalPlan,
    on: &[JoinKey],
    ctx: &mut SchemaContext,
) -> Result<proto::Rel, EmitError> {
    let mut left_ctx = SchemaContext::new();
    let left_rel = emit_rel(left, &mut left_ctx)?;

    let mut right_ctx = SchemaContext::new();
    let right_rel = emit_rel(right, &mut right_ctx)?;

    ctx.merge(left_ctx.clone());
    let right_offset = ctx.len();
    ctx.merge(right_ctx.clone());

    if on.is_empty() {
        return Err(EmitError::InvalidPlan("join with no keys"));
    }

    let mut cond = join_key_expr(&on[0], &left_ctx, &right_ctx, right_offset)?;
    for key in on.iter().skip(1) {
        cond = and_two(cond, join_key_expr(key, &left_ctx, &right_ctx, right_offset)?);
    }

    Ok(proto::Rel {
        rel_type: Some(RelType::Join(Box::new(proto::JoinRel {
            left: Some(Box::new(left_rel)),
            right: Some(Box::new(right_rel)),
            r#type: proto::join_rel::JoinType::Inner as i32,
            expression: Some(Box::new(cond)),
            ..Default::default()
        }))),
    })
}

fn join_key_expr(
    key: &JoinKey,
    left_ctx: &SchemaContext,
    right_ctx: &SchemaContext,
    right_offset: usize,
) -> Result<proto::Expression, EmitError> {
    let left_idx = left_ctx
        .find_column(&key.left_dataset, &key.left_sql)
        .ok_or_else(|| EmitError::ColumnNotFound(format!("{}.{}", key.left_dataset, key.left_sql)))?
        as u32;
    let right_local = right_ctx
        .find_column(&key.right_dataset, &key.right_sql)
        .ok_or_else(|| EmitError::ColumnNotFound(format!("{}.{}", key.right_dataset, key.right_sql)))?
        as u32;
    Ok(binary_eq_field_ref(left_idx, right_offset as u32 + right_local))
}

fn emit_filter(
    input: &LogicalPlan,
    predicate: &Expr,
    ctx: &mut SchemaContext,
) -> Result<proto::Rel, EmitError> {
    let input_rel = emit_rel(input, ctx)?;
    let condition = emit_expr(predicate, ctx)?;
    Ok(proto::Rel {
        rel_type: Some(RelType::Filter(Box::new(proto::FilterRel {
            input: Some(Box::new(input_rel)),
            condition: Some(Box::new(condition)),
            ..Default::default()
        }))),
    })
}

fn emit_aggregate_rel(
    input: proto::Rel,
    group_by: &[Expr],
    aggregates: &[NamedAggregate],
    ctx: &mut SchemaContext,
) -> Result<proto::Rel, EmitError> {
    let grouping_expressions: Vec<proto::Expression> = group_by
        .iter()
        .map(|e| emit_expr(e, ctx))
        .collect::<Result<_, _>>()?;

    #[allow(deprecated)]
    let groupings = if grouping_expressions.is_empty() {
        vec![]
    } else {
        vec![Grouping {
            grouping_expressions: vec![],
            expression_references: (0..grouping_expressions.len() as u32).collect(),
        }]
    };

    let measures: Vec<Measure> = aggregates
        .iter()
        .map(|a| emit_measure(a, ctx))
        .collect::<Result<_, _>>()?;

    let mut new_ctx = SchemaContext::new();
    for e in group_by {
        if let Expr::Column(c) = e {
            new_ctx.columns.push((c.dataset.clone(), c.sql.clone()));
        }
    }
    for a in aggregates {
        new_ctx.columns.push((String::new(), a.name.clone()));
    }
    *ctx = new_ctx;

    Ok(proto::Rel {
        rel_type: Some(RelType::Aggregate(Box::new(proto::AggregateRel {
            input: Some(Box::new(input)),
            groupings,
            grouping_expressions,
            measures,
            ..Default::default()
        }))),
    })
}

fn emit_measure(agg: &NamedAggregate, ctx: &SchemaContext) -> Result<Measure, EmitError> {
    let Expr::Agg { func, distinct, arg } = &agg.expr else {
        return Err(EmitError::UnsupportedExpression(format!(
            "metric `{}` is not a structured aggregate",
            agg.name
        )));
    };

    let function_reference = catalog::def(func.to_func()).anchor;

    let arg_expr = match arg {
        None => i64_literal(1),
        Some(inner) => emit_expr(inner, ctx)?,
    };

    Ok(Measure {
        measure: Some(proto::AggregateFunction {
            function_reference,
            arguments: vec![proto::FunctionArgument {
                arg_type: Some(ArgType::Value(arg_expr)),
            }],
            output_type: None,
            phase: AggregationPhase::Unspecified as i32,
            invocation: if *distinct {
                AggregationInvocation::Distinct as i32
            } else {
                AggregationInvocation::All as i32
            },
            ..Default::default()
        }),
        filter: None,
    })
}

fn emit_expr(expr: &Expr, ctx: &SchemaContext) -> Result<proto::Expression, EmitError> {
    match expr {
        Expr::Column(col) => emit_column_ref(col, ctx),
        Expr::Literal(lit) => emit_literal(lit),
        Expr::Eq(l, r) => emit_binary_eq(l, r, ctx),
        Expr::Ne(l, r) => emit_binary_ne(l, r, ctx),
        Expr::And(exprs) => emit_and(exprs, ctx),
        Expr::In { expr, values } => emit_in(expr, values, ctx),
        Expr::Case {
            when_then,
            else_expr,
        } => emit_case(when_then, Some(else_expr.as_ref()), ctx),
        Expr::Sub(l, r) => Ok(scalar(
            Func::Subtract,
            vec![emit_expr(l, ctx)?, emit_expr(r, ctx)?],
        )),
        Expr::CurrentDate => Ok(scalar(Func::CurrentDate, vec![])),
        Expr::Agg { .. } => Err(EmitError::UnsupportedExpression(
            "aggregate Expr belongs in NamedAggregate".to_string(),
        )),
        Expr::Sql(s) => Err(EmitError::UnsupportedExpression(format!("SQL: {s}"))),
    }
}

fn emit_case(
    when_then: &[(Expr, Expr)],
    else_result: Option<&Expr>,
    ctx: &SchemaContext,
) -> Result<proto::Expression, EmitError> {
    let ifs: Vec<expression::if_then::IfClause> = when_then
        .iter()
        .map(|(cond, then)| {
            Ok(expression::if_then::IfClause {
                r#if: Some(emit_expr(cond, ctx)?),
                then: Some(emit_expr(then, ctx)?),
            })
        })
        .collect::<Result<Vec<_>, EmitError>>()?;

    let else_expr = match else_result {
        Some(e) => Some(Box::new(emit_expr(e, ctx)?)),
        None => None,
    };

    Ok(proto::Expression {
        rex_type: Some(expression::RexType::IfThen(Box::new(expression::IfThen {
            ifs,
            r#else: else_expr,
        }))),
    })
}

fn emit_column_ref(col: &ColumnRef, ctx: &SchemaContext) -> Result<proto::Expression, EmitError> {
    let idx = ctx
        .find_column(&col.dataset, &col.sql)
        .ok_or_else(|| EmitError::ColumnNotFound(format!("{}.{}", col.dataset, col.sql)))?;
    emit_field_reference(idx as u32)
}

fn emit_in(
    expr: &Expr,
    values: &[Literal],
    ctx: &SchemaContext,
) -> Result<proto::Expression, EmitError> {
    if values.is_empty() {
        return emit_literal(&Literal::Bool(false));
    }
    let mut acc = emit_binary_eq(expr, &Expr::Literal(values[0].clone()), ctx)?;
    for v in values.iter().skip(1) {
        acc = or_two(acc, emit_binary_eq(expr, &Expr::Literal(v.clone()), ctx)?);
    }
    Ok(acc)
}

fn emit_binary_eq(l: &Expr, r: &Expr, ctx: &SchemaContext) -> Result<proto::Expression, EmitError> {
    let left = emit_expr(l, ctx)?;
    let right = emit_expr(r, ctx)?;
    Ok(scalar(Func::Equal, vec![left, right]))
}

fn emit_binary_ne(l: &Expr, r: &Expr, ctx: &SchemaContext) -> Result<proto::Expression, EmitError> {
    let left = emit_expr(l, ctx)?;
    let right = emit_expr(r, ctx)?;
    Ok(scalar(Func::NotEqual, vec![left, right]))
}

fn emit_and(exprs: &[Expr], ctx: &SchemaContext) -> Result<proto::Expression, EmitError> {
    if exprs.is_empty() {
        return emit_literal(&Literal::Bool(true));
    }
    if exprs.len() == 1 {
        return emit_expr(&exprs[0], ctx);
    }
    let mut acc = emit_expr(&exprs[0], ctx)?;
    for e in exprs.iter().skip(1) {
        acc = and_two(acc, emit_expr(e, ctx)?);
    }
    Ok(acc)
}

fn emit_literal(lit: &Literal) -> Result<proto::Expression, EmitError> {
    let literal_type = match lit {
        Literal::Null => LiteralType::Null(string_type()),
        Literal::Bool(b) => LiteralType::Boolean(*b),
        Literal::Int64(i) => LiteralType::I64(*i),
        Literal::Float64(f) => LiteralType::Fp64(*f),
        Literal::String(s) => LiteralType::String(s.clone()),
    };
    Ok(proto::Expression {
        rex_type: Some(expression::RexType::Literal(proto::expression::Literal {
            nullable: true,
            type_variation_reference: 0,
            literal_type: Some(literal_type),
        })),
    })
}

fn i64_literal(v: i64) -> proto::Expression {
    proto::Expression {
        rex_type: Some(expression::RexType::Literal(proto::expression::Literal {
            nullable: false,
            type_variation_reference: 0,
            literal_type: Some(LiteralType::I64(v)),
        })),
    }
}

fn emit_field_reference(field: u32) -> Result<proto::Expression, EmitError> {
    Ok(proto::Expression {
        rex_type: Some(expression::RexType::Selection(Box::new(
            proto::expression::FieldReference {
                reference_type: Some(expression::field_reference::ReferenceType::DirectReference(
                    ReferenceSegment {
                        reference_type: Some(ReferenceType::StructField(Box::new(StructField {
                            field: field as i32,
                            child: None,
                        }))),
                    },
                )),
                root_type: None,
            },
        ))),
    })
}

fn binary_eq_field_ref(left: u32, right: u32) -> proto::Expression {
    scalar(
        Func::Equal,
        vec![
            emit_field_reference(left).unwrap(),
            emit_field_reference(right).unwrap(),
        ],
    )
}

/// Scalar function call referencing `func`'s catalog anchor.
fn scalar(func: Func, args: Vec<proto::Expression>) -> proto::Expression {
    scalar_fn(catalog::def(func).anchor, args)
}

fn scalar_fn(function_reference: u32, args: Vec<proto::Expression>) -> proto::Expression {
    proto::Expression {
        rex_type: Some(expression::RexType::ScalarFunction(
            proto::expression::ScalarFunction {
                function_reference,
                arguments: args
                    .into_iter()
                    .map(|expr| proto::FunctionArgument {
                        arg_type: Some(ArgType::Value(expr)),
                    })
                    .collect(),
                output_type: None,
                ..Default::default()
            },
        )),
    }
}

fn and_two(a: proto::Expression, b: proto::Expression) -> proto::Expression {
    scalar(Func::And, vec![a, b])
}

fn or_two(a: proto::Expression, b: proto::Expression) -> proto::Expression {
    scalar(Func::Or, vec![a, b])
}

fn string_type() -> proto::Type {
    proto::Type {
        kind: Some(Kind::String(proto::r#type::String {
            type_variation_reference: 0,
            nullability: Nullability::Nullable as i32,
        })),
    }
}

fn make_function_extension(urn_ref: u32, anchor: u32, name: &str) -> SimpleExtensionDeclaration {
    SimpleExtensionDeclaration {
        mapping_type: Some(MappingType::ExtensionFunction(
            proto::extensions::simple_extension_declaration::ExtensionFunction {
                extension_urn_reference: urn_ref,
                function_anchor: anchor,
                name: name.to_string(),
            },
        )),
    }
}

fn build_extension_urns() -> Vec<SimpleExtensionUrn> {
    catalog::extension_groups()
        .into_iter()
        .map(|g| SimpleExtensionUrn {
            extension_urn_anchor: g.anchor,
            urn: g.urn.to_string(),
        })
        .collect()
}

fn build_extensions() -> Vec<SimpleExtensionDeclaration> {
    catalog::CATALOG
        .iter()
        .map(|d| make_function_extension(d.group.anchor, d.anchor, d.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::expr::{AggFunc, Expr, Literal};
    use crate::plan::{LogicalPlan, NamedAggregate};

    fn minimal_plan() -> LogicalPlan {
        LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    source: "warehouse.public.fact_table".into(),
                    dataset: "fact".into(),
                    columns: vec!["id".into(), "day".into(), "month_days".into()],
                }),
                predicate: Expr::Eq(
                    Box::new(Expr::column("", "id")),
                    Box::new(Expr::Literal(Literal::Int64(1))),
                ),
            }),
            group_by: vec![Expr::column("", "id")],
            aggregates: vec![NamedAggregate {
                name: "row_count".into(),
                expr: Expr::Agg {
                    func: AggFunc::Count,
                    distinct: false,
                    arg: None,
                },
            }],
        }
    }

    #[test]
    fn aggregate_to_substrait_plan() {
        let p = to_plan(&minimal_plan()).expect("plan");
        assert_eq!(p.relations.len(), 1);
    }

    #[test]
    fn encode_roundtrip_len() {
        let p = to_plan(&minimal_plan()).expect("plan");
        let bytes = encode_plan(&p);
        assert!(!bytes.is_empty());
    }
}
