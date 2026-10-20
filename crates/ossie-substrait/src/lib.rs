//! Emit Substrait [`Plan`](substrait::proto::Plan) from a structured [`LogicalPlan`](ossie_compiler::plan::LogicalPlan).
//!
//! Plans come from `ossie-compiler`; depending on this crate is how a caller
//! opts into Substrait.

#![forbid(unsafe_code)]

use substrait::proto::{
    self, AggregationPhase,
    aggregate_function::AggregationInvocation,
    aggregate_rel::{Grouping, Measure},
    expression::{
        self, ReferenceSegment,
        literal::LiteralType,
        reference_segment::{ReferenceType, StructField},
    },
    extensions::{
        SimpleExtensionDeclaration, SimpleExtensionUrn, simple_extension_declaration::MappingType,
    },
    function_argument::ArgType,
    plan_rel::RelType as PlanRelType,
    read_rel::{NamedTable, ReadType},
    rel::RelType,
    r#type::{Kind, Nullability},
};

use ossie_compiler::catalog::{self, Func};
use ossie_compiler::emit::EmitError;
use ossie_compiler::plan::expr::{ColumnRef, Expr, JoinKey, Literal};
use ossie_compiler::plan::{
    LogicalPlan, NamedExpr, ScanColumn, SchemaColumn, SortKey, find_column,
};
use ossie_compiler::query::{Direction, NullOrder};
use ossie_model::DataType;

#[derive(Debug, Clone)]
/// The columns a field reference resolves against, taken from the plan's own
/// [`LogicalPlan::output_schema`] so this cannot disagree with the planner.
struct SchemaContext {
    columns: Vec<SchemaColumn>,
}

impl SchemaContext {
    fn of(plan: &LogicalPlan) -> Self {
        Self {
            columns: plan.output_schema(),
        }
    }

    fn find_column(&self, dataset: &str, column: &str) -> Option<usize> {
        find_column(&self.columns, dataset, column)
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

/// Build a Substrait plan from `plan`.
///
/// The root is an [`LogicalPlan::Aggregate`], or a [`LogicalPlan::Project`]
/// over one when a metric combines several aggregates.
pub fn to_plan(plan: &LogicalPlan) -> Result<proto::Plan, EmitError> {
    // Sort and Fetch sit above the aggregate core; peel them first so the
    // core is matched the same way whether or not they are present.
    let mut trailing: Vec<&LogicalPlan> = Vec::new();
    let mut core = plan;
    while let LogicalPlan::Sort { input, .. } | LogicalPlan::Fetch { input, .. } = core {
        trailing.push(core);
        core = input;
    }

    let (aggregate, projection) = match core {
        LogicalPlan::Aggregate { .. } => (core, None),
        LogicalPlan::Project { input, exprs }
            if matches!(**input, LogicalPlan::Aggregate { .. }) =>
        {
            (input.as_ref(), Some(exprs))
        }
        _ => {
            return Err(EmitError::Unsupported(
                "root must be Aggregate, or Project over Aggregate (use planner output)",
            ));
        }
    };

    let LogicalPlan::Aggregate {
        input,
        group_by,
        aggregates,
    } = aggregate
    else {
        unreachable!("matched above");
    };

    let input_rel = emit_rel(input)?;
    let input_ctx = SchemaContext::of(input);
    let rel = emit_aggregate_rel(input_rel, group_by, aggregates, &input_ctx)?;

    let (rel, output_names) = match projection {
        None => {
            let mut names: Vec<String> = group_by.iter().map(output_name_for_expr).collect();
            names.extend(aggregates.iter().map(|a| a.name.clone()));
            (rel, names)
        }
        Some(exprs) => {
            let agg_ctx = SchemaContext::of(aggregate);
            let rel = emit_project_rel(rel, exprs, &agg_ctx)?;
            (rel, exprs.iter().map(|e| e.name.clone()).collect())
        }
    };

    // Re-apply the peeled operators, innermost first.
    let mut rel = rel;
    let out_ctx = SchemaContext::of(core_plan_output(plan));
    for node in trailing.into_iter().rev() {
        rel = match node {
            LogicalPlan::Sort { keys, .. } => emit_sort_rel(rel, keys, &out_ctx)?,
            LogicalPlan::Fetch { offset, limit, .. } => emit_fetch_rel(rel, *offset, *limit),
            _ => unreachable!("only Sort and Fetch are peeled"),
        };
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

/// The node whose schema the trailing operators see: Sort and Fetch pass
/// their input through, so it is the aggregate core's output.
fn core_plan_output(plan: &LogicalPlan) -> &LogicalPlan {
    let mut core = plan;
    while let LogicalPlan::Sort { input, .. } | LogicalPlan::Fetch { input, .. } = core {
        core = input;
    }
    core
}

fn emit_sort_rel(
    input: proto::Rel,
    keys: &[SortKey],
    ctx: &SchemaContext,
) -> Result<proto::Rel, EmitError> {
    let sorts = keys
        .iter()
        .map(|k| {
            Ok(proto::SortField {
                expr: Some(emit_expr(&k.expr, ctx)?),
                sort_kind: Some(proto::sort_field::SortKind::Direction(
                    sort_direction(k) as i32
                )),
            })
        })
        .collect::<Result<Vec<_>, EmitError>>()?;

    Ok(proto::Rel {
        rel_type: Some(RelType::Sort(Box::new(proto::SortRel {
            input: Some(Box::new(input)),
            sorts,
            ..Default::default()
        }))),
    })
}

/// Substrait folds nulls placement into the direction, which is why the plan
/// always states it.
fn sort_direction(key: &SortKey) -> proto::sort_field::SortDirection {
    use proto::sort_field::SortDirection as D;
    match (key.direction, key.nulls) {
        (Direction::Asc, NullOrder::First) => D::AscNullsFirst,
        (Direction::Asc, NullOrder::Last) => D::AscNullsLast,
        (Direction::Desc, NullOrder::First) => D::DescNullsFirst,
        (Direction::Desc, NullOrder::Last) => D::DescNullsLast,
    }
}

fn emit_fetch_rel(input: proto::Rel, offset: u64, limit: Option<u64>) -> proto::Rel {
    proto::Rel {
        rel_type: Some(RelType::Fetch(Box::new(proto::FetchRel {
            input: Some(Box::new(input)),
            // The literal-expression forms; the bare int64 fields are deprecated.
            offset_mode: (offset > 0).then(|| {
                proto::fetch_rel::OffsetMode::OffsetExpr(Box::new(i64_literal(offset as i64)))
            }),
            count_mode: limit
                .map(|n| proto::fetch_rel::CountMode::CountExpr(Box::new(i64_literal(n as i64)))),
            ..Default::default()
        }))),
    }
}

fn output_name_for_expr(expr: &Expr) -> String {
    match expr {
        Expr::Column(c) if !c.dataset.is_empty() => format!("{}.{}", c.dataset, c.column),
        Expr::Column(c) => c.column.clone(),
        _ => "expr".to_string(),
    }
}

fn emit_rel(plan: &LogicalPlan) -> Result<proto::Rel, EmitError> {
    match plan {
        LogicalPlan::Scan {
            source,
            dataset: _,
            columns,
        } => emit_scan(source, plan, columns),
        LogicalPlan::Join { left, right, on } => emit_join(left, right, on),
        LogicalPlan::Filter { input, predicate } => emit_filter(input, predicate),
        LogicalPlan::Aggregate { .. }
        | LogicalPlan::Project { .. }
        | LogicalPlan::Sort { .. }
        | LogicalPlan::Fetch { .. } => Err(EmitError::InvalidPlan(
            "unexpected nested Aggregate in input rel",
        )),
    }
}

fn emit_scan(
    source: &str,
    _plan: &LogicalPlan,
    columns: &[ScanColumn],
) -> Result<proto::Rel, EmitError> {
    let types: Vec<proto::Type> = columns.iter().map(|c| substrait_type(c.datatype)).collect();
    let base_schema = proto::NamedStruct {
        names: columns.iter().map(|c| c.name.clone()).collect(),
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
) -> Result<proto::Rel, EmitError> {
    let left_rel = emit_rel(left)?;
    let right_rel = emit_rel(right)?;

    let left_ctx = SchemaContext::of(left);
    let right_ctx = SchemaContext::of(right);
    // A join's columns are the left's followed by the right's, so a right
    // column's index is offset by the left's width.
    let right_offset = left_ctx.len();

    if on.is_empty() {
        return Err(EmitError::InvalidPlan("join with no keys"));
    }

    let mut cond = join_key_expr(&on[0], &left_ctx, &right_ctx, right_offset)?;
    for key in on.iter().skip(1) {
        cond = and_two(
            cond,
            join_key_expr(key, &left_ctx, &right_ctx, right_offset)?,
        );
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
        .find_column(&key.left_dataset, &key.left_column)
        .ok_or_else(|| {
            EmitError::ColumnNotFound(format!("{}.{}", key.left_dataset, key.left_column))
        })? as u32;
    let right_local = right_ctx
        .find_column(&key.right_dataset, &key.right_column)
        .ok_or_else(|| {
            EmitError::ColumnNotFound(format!("{}.{}", key.right_dataset, key.right_column))
        })? as u32;
    Ok(binary_eq_field_ref(
        left_idx,
        right_offset as u32 + right_local,
    ))
}

fn emit_filter(input: &LogicalPlan, predicate: &Expr) -> Result<proto::Rel, EmitError> {
    let input_rel = emit_rel(input)?;
    let condition = emit_expr(predicate, &SchemaContext::of(input))?;
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
    aggregates: &[NamedExpr],
    ctx: &SchemaContext,
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

/// Project the grouping columns through unchanged and append one computed
/// column per metric.
fn emit_project_rel(
    input: proto::Rel,
    projections: &[NamedExpr],
    ctx: &SchemaContext,
) -> Result<proto::Rel, EmitError> {
    let expressions: Vec<proto::Expression> = projections
        .iter()
        .map(|p| emit_expr(&p.expr, ctx))
        .collect::<Result<_, _>>()?;

    // ProjectRel appends its expressions to the input columns, so the emit
    // mapping selects only the appended ones, dropping the intermediate
    // measures the planner lifted out.
    let input_width = ctx.len();
    let output_mapping: Vec<i32> = (0..expressions.len())
        .map(|i| (input_width + i) as i32)
        .collect();

    Ok(proto::Rel {
        rel_type: Some(RelType::Project(Box::new(proto::ProjectRel {
            common: Some(proto::RelCommon {
                emit_kind: Some(proto::rel_common::EmitKind::Emit(proto::rel_common::Emit {
                    output_mapping,
                })),
                ..Default::default()
            }),
            input: Some(Box::new(input)),
            expressions,
            ..Default::default()
        }))),
    })
}

fn emit_measure(agg: &NamedExpr, ctx: &SchemaContext) -> Result<Measure, EmitError> {
    let Expr::Agg {
        func,
        distinct,
        args,
    } = &agg.expr
    else {
        return Err(EmitError::UnsupportedExpression(format!(
            "metric `{}` is not a structured aggregate",
            agg.name
        )));
    };

    let function_reference = catalog::def(*func).anchor;

    // `COUNT(*)` takes no argument; Substrait still wants one, so it counts a
    // constant.
    let arguments = if args.is_empty() {
        vec![proto::FunctionArgument {
            arg_type: Some(ArgType::Value(i64_literal(1))),
        }]
    } else {
        args.iter()
            .map(|a| {
                Ok(proto::FunctionArgument {
                    arg_type: Some(ArgType::Value(emit_expr(a, ctx)?)),
                })
            })
            .collect::<Result<Vec<_>, EmitError>>()?
    };

    // Declared only where Ossie says enough to know it; a consumer derives
    // the rest from the function definition rather than trusting a guess.
    let output_type = agg.expr.result_type(&ctx.columns).map(substrait_type_of);

    Ok(Measure {
        measure: Some(proto::AggregateFunction {
            function_reference,
            arguments,
            output_type,
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
        Expr::Call { func, args } => emit_scalar_call(*func, args, ctx),
        Expr::Case {
            when_then,
            else_expr,
        } => emit_case(when_then, Some(else_expr.as_ref()), ctx),
        Expr::InList { expr, values } => emit_in_list(expr, values, ctx),
        Expr::Agg { .. } => Err(EmitError::UnsupportedExpression(
            "aggregate belongs in an AggregateRel measure, not a scalar position".to_string(),
        )),
    }
}

/// Emit a scalar function call.
///
/// Substrait's boolean `and`/`or` are binary, so a variadic conjunction folds
/// into nested calls; everything else maps one-to-one.
fn emit_scalar_call(
    func: Func,
    args: &[Expr],
    ctx: &SchemaContext,
) -> Result<proto::Expression, EmitError> {
    let emitted = args
        .iter()
        .map(|a| emit_expr(a, ctx))
        .collect::<Result<Vec<_>, _>>()?;

    if matches!(func, Func::And | Func::Or) {
        let identity = Literal::Bool(matches!(func, Func::And));
        let mut it = emitted.into_iter();
        let Some(first) = it.next() else {
            return emit_literal(&identity);
        };
        return Ok(it.fold(first, |acc, next| scalar(func, vec![acc, next])));
    }

    Ok(scalar(func, emitted))
}

/// `expr IN (…)` — Substrait models this as a dedicated expression type.
fn emit_in_list(
    expr: &Expr,
    values: &[Literal],
    ctx: &SchemaContext,
) -> Result<proto::Expression, EmitError> {
    if values.is_empty() {
        return emit_literal(&Literal::Bool(false));
    }
    let options = values
        .iter()
        .map(emit_literal)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(proto::Expression {
        rex_type: Some(expression::RexType::SingularOrList(Box::new(
            expression::SingularOrList {
                value: Some(Box::new(emit_expr(expr, ctx)?)),
                options,
            },
        ))),
    })
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
        .find_column(&col.dataset, &col.column)
        .ok_or_else(|| EmitError::ColumnNotFound(format!("{}.{}", col.dataset, col.column)))?;
    emit_field_reference(idx as u32)
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

/// Sub-second precision assumed when Ossie does not state one.
///
/// Substrait's deprecated `Time`/`Timestamp`/`TimestampTz` were defined as
/// microseconds, and the `Precision*` types are their replacements, so 6
/// preserves that meaning rather than inventing a resolution.
const TIMESTAMP_PRECISION_MICROS: i32 = 6;

/// Substrait type for a declared Ossie [`DataType`].
///
/// Types Ossie leaves under-specified map to Substrait's `unknown` rather than
/// to an invented parameterization:
///
/// - `Decimal` — Substrait requires precision and scale; Ossie states neither,
///   and a wrong scale changes values.
/// - `Opaque` — explicitly "outside the portable vocabulary".
/// - absent — `datatype` is optional in the core spec.
///
/// `Integer` widens to `i64`: Ossie leaves width unspecified, and `i64` holds
/// every integral value Ossie can describe.
fn substrait_type(datatype: Option<DataType>) -> proto::Type {
    match datatype {
        Some(t) => substrait_type_of(t),
        None => unknown_type(),
    }
}

/// Substrait's `unknown`, for a type Ossie leaves unstated.
fn unknown_type() -> proto::Type {
    proto::Type {
        kind: Some(Kind::UserDefined(proto::r#type::UserDefined {
            type_reference: catalog::UNKNOWN_TYPE_ANCHOR,
            type_variation_reference: 0,
            nullability: Nullability::Nullable as i32,
            type_parameters: vec![],
        })),
    }
}

fn substrait_type_of(datatype: DataType) -> proto::Type {
    let nullability = Nullability::Nullable as i32;
    let kind = match Some(datatype) {
        Some(DataType::String) => Kind::String(proto::r#type::String {
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::Integer) => Kind::I64(proto::r#type::I64 {
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::Float) => Kind::Fp64(proto::r#type::Fp64 {
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::Boolean) => Kind::Bool(proto::r#type::Boolean {
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::Date) => Kind::Date(proto::r#type::Date {
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::Time) => Kind::PrecisionTime(proto::r#type::PrecisionTime {
            precision: TIMESTAMP_PRECISION_MICROS,
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::DateTime) => Kind::PrecisionTimestamp(proto::r#type::PrecisionTimestamp {
            precision: TIMESTAMP_PRECISION_MICROS,
            type_variation_reference: 0,
            nullability,
        }),
        Some(DataType::DateTimeTz) => {
            Kind::PrecisionTimestampTz(proto::r#type::PrecisionTimestampTz {
                precision: TIMESTAMP_PRECISION_MICROS,
                type_variation_reference: 0,
                nullability,
            })
        }
        Some(DataType::Decimal) | Some(DataType::Opaque) | None => {
            Kind::UserDefined(proto::r#type::UserDefined {
                type_reference: catalog::UNKNOWN_TYPE_ANCHOR,
                type_variation_reference: 0,
                nullability,
                type_parameters: vec![],
            })
        }
    };
    proto::Type { kind: Some(kind) }
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
        .chain(std::iter::once(catalog::UNKNOWN_TYPE))
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
        .chain(std::iter::once(make_type_extension(
            catalog::UNKNOWN_TYPE.anchor,
            catalog::UNKNOWN_TYPE_ANCHOR,
            "unknown",
        )))
        .collect()
}

fn make_type_extension(urn_ref: u32, anchor: u32, name: &str) -> SimpleExtensionDeclaration {
    SimpleExtensionDeclaration {
        mapping_type: Some(MappingType::ExtensionType(
            proto::extensions::simple_extension_declaration::ExtensionType {
                extension_urn_reference: urn_ref,
                type_anchor: anchor,
                name: name.to_string(),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ossie_compiler::catalog::Func;
    use ossie_compiler::plan::expr::{Expr, Literal};
    use ossie_compiler::plan::{LogicalPlan, NamedExpr, ScanColumn};

    fn minimal_plan() -> LogicalPlan {
        LogicalPlan::Aggregate {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    source: "warehouse.public.fact_table".into(),
                    dataset: "fact".into(),
                    columns: vec![
                        ScanColumn::new("id", None),
                        ScanColumn::new("day", None),
                        ScanColumn::new("month_days", None),
                    ],
                }),
                predicate: Expr::binary(
                    Func::Equal,
                    Expr::column("", "id"),
                    Expr::Literal(Literal::Int64(1)),
                ),
            }),
            group_by: vec![Expr::column("", "id")],
            aggregates: vec![NamedExpr {
                name: "row_count".into(),
                expr: Expr::Agg {
                    func: Func::Count,
                    distinct: false,
                    args: vec![],
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

pub mod prelude {
    pub use crate::{encode_plan, to_plan};
    pub use ossie_compiler::prelude::*;
}
