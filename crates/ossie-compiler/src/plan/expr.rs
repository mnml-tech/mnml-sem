//! Structured expressions for logical plans (Substrait-primary, SQL derived).
//!
//! Each variant maps to exactly one Substrait expression type, so emission is a
//! direct translation rather than a decision:
//!
//! | [`Expr`]  | Substrait          |
//! |-----------|--------------------|
//! | `Column`  | `Selection`        |
//! | `Literal` | `Literal`          |
//! | `Call`    | `ScalarFunction`   |
//! | `Agg`     | `AggregateFunction`|
//! | `Case`    | `IfThen`           |
//! | `InList`  | `SingularOrList`   |
//!
//! Functions are not variants: they are rows in [`catalog::CATALOG`], which
//! carries each one's arity, Substrait coordinates, and SQL rendering. Adding a
//! function is a table row, not a change to this enum.

use crate::catalog::{self, Func};
use crate::plan::{SchemaColumn, find_column};
use ossie_model::DataType;
use serde_json::Value;

/// Reference to a physical column of a logical dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRef {
    /// Logical dataset name; empty when the query has a single source.
    pub dataset: String,
    /// Physical column name.
    pub column: String,
}

impl ColumnRef {
    pub fn new(dataset: impl Into<String>, column: impl Into<String>) -> Self {
        Self {
            dataset: dataset.into(),
            column: column.into(),
        }
    }
}

/// Literal value in filters and expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Bool(bool),
    Int64(i64),
    Float64(f64),
    String(String),
}

impl Literal {
    /// The type this literal denotes. `NULL` denotes none.
    pub fn datatype(&self) -> Option<DataType> {
        Some(match self {
            Literal::Null => return None,
            Literal::Bool(_) => DataType::Boolean,
            Literal::Int64(_) => DataType::Integer,
            Literal::Float64(_) => DataType::Float,
            Literal::String(_) => DataType::String,
        })
    }

    /// Convert a JSON filter value.
    ///
    /// Composite JSON (arrays, objects) has no literal form; callers that can
    /// receive it must reject it before getting here.
    pub fn from_json(v: &Value) -> Option<Self> {
        Some(match v {
            Value::Null => Literal::Null,
            Value::Bool(b) => Literal::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Literal::Int64(i)
                } else {
                    Literal::Float64(n.as_f64()?)
                }
            }
            Value::String(s) => Literal::String(s.clone()),
            Value::Array(_) | Value::Object(_) => return None,
        })
    }
}

/// Structured expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(ColumnRef),
    Literal(Literal),
    /// Scalar function application. `func` is always [`FuncKind::Scalar`].
    ///
    /// [`FuncKind::Scalar`]: crate::catalog::FuncKind::Scalar
    Call {
        func: Func,
        args: Vec<Expr>,
    },
    /// Aggregate application. `args` is empty for `COUNT(*)`.
    Agg {
        func: Func,
        distinct: bool,
        args: Vec<Expr>,
    },
    /// `CASE WHEN … THEN … ELSE … END`.
    Case {
        when_then: Vec<(Expr, Expr)>,
        else_expr: Box<Expr>,
    },
    /// `expr IN (…)` over literals.
    InList {
        expr: Box<Expr>,
        values: Vec<Literal>,
    },
}

impl Expr {
    pub fn column(dataset: impl Into<String>, column: impl Into<String>) -> Self {
        Expr::Column(ColumnRef::new(dataset, column))
    }

    pub fn call(func: Func, args: Vec<Expr>) -> Self {
        Expr::Call { func, args }
    }

    pub fn binary(func: Func, left: Expr, right: Expr) -> Self {
        Expr::Call {
            func,
            args: vec![left, right],
        }
    }

    pub fn literal(lit: Literal) -> Self {
        Expr::Literal(lit)
    }

    /// Fold a conjunction, collapsing the degenerate cases.
    pub fn and(exprs: Vec<Expr>) -> Self {
        Self::fold(Func::And, exprs, Literal::Bool(true))
    }

    /// Fold a disjunction, collapsing the degenerate cases.
    pub fn or(exprs: Vec<Expr>) -> Self {
        Self::fold(Func::Or, exprs, Literal::Bool(false))
    }

    fn fold(func: Func, exprs: Vec<Expr>, empty: Literal) -> Self {
        match exprs.len() {
            0 => Expr::Literal(empty),
            1 => exprs.into_iter().next().expect("len 1"),
            _ => Expr::Call { func, args: exprs },
        }
    }

    /// The type this expression produces, given the schema it reads.
    ///
    /// `None` wherever the answer is not determined — an undeclared column, or
    /// a function whose result depends on an argument whose type is unknown.
    /// Ossie's `datatype` is optional, so that is the common case and it must
    /// stay `None` rather than become a guess.
    pub fn result_type(&self, schema: &[SchemaColumn]) -> Option<DataType> {
        match self {
            Expr::Column(c) => {
                find_column(schema, &c.dataset, &c.column).and_then(|i| schema[i].datatype)
            }
            Expr::Literal(lit) => lit.datatype(),
            Expr::Call { func, args } | Expr::Agg { func, args, .. } => {
                let arg_types: Vec<Option<DataType>> =
                    args.iter().map(|a| a.result_type(schema)).collect();
                catalog::def(*func).returns.resolve(&arg_types)
            }
            // Every branch should agree; the first known one answers.
            Expr::Case {
                when_then,
                else_expr,
            } => when_then
                .iter()
                .map(|(_, then)| then)
                .chain(std::iter::once(else_expr.as_ref()))
                .find_map(|e| e.result_type(schema)),
            Expr::InList { .. } => Some(DataType::Boolean),
        }
    }

    /// Whether this node is an aggregate, or contains one at any depth.
    pub fn contains_aggregate(&self) -> bool {
        matches!(self, Expr::Agg { .. }) || self.children().iter().any(|c| c.contains_aggregate())
    }

    /// Every dataset named by a column reference anywhere in this expression.
    pub fn referenced_datasets(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        self.collect_datasets(&mut out);
        out
    }

    fn collect_datasets(&self, out: &mut std::collections::BTreeSet<String>) {
        if let Expr::Column(c) = self {
            if !c.dataset.is_empty() {
                out.insert(c.dataset.clone());
            }
        }
        for child in self.children() {
            child.collect_datasets(out);
        }
    }

    /// Immediate sub-expressions, left to right.
    pub fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Column(_) | Expr::Literal(_) => vec![],
            Expr::Call { args, .. } | Expr::Agg { args, .. } => args.iter().collect(),
            Expr::InList { expr, .. } => vec![expr.as_ref()],
            Expr::Case {
                when_then,
                else_expr,
            } => {
                let mut out: Vec<&Expr> = Vec::with_capacity(when_then.len() * 2 + 1);
                for (w, t) in when_then {
                    out.push(w);
                    out.push(t);
                }
                out.push(else_expr.as_ref());
                out
            }
        }
    }

    /// Rebuild this node with each immediate child replaced by `f(child)`.
    pub fn map_children(&self, f: &mut impl FnMut(&Expr) -> Expr) -> Expr {
        match self {
            Expr::Column(_) | Expr::Literal(_) => self.clone(),
            Expr::Call { func, args } => Expr::Call {
                func: *func,
                args: args.iter().map(&mut *f).collect(),
            },
            Expr::Agg {
                func,
                distinct,
                args,
            } => Expr::Agg {
                func: *func,
                distinct: *distinct,
                args: args.iter().map(&mut *f).collect(),
            },
            Expr::InList { expr, values } => Expr::InList {
                expr: Box::new(f(expr)),
                values: values.clone(),
            },
            Expr::Case {
                when_then,
                else_expr,
            } => Expr::Case {
                when_then: when_then.iter().map(|(w, t)| (f(w), f(t))).collect(),
                else_expr: Box::new(f(else_expr)),
            },
        }
    }
}

/// Equi-join key between two datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinKey {
    pub left_dataset: String,
    pub left_column: String,
    pub right_dataset: String,
    pub right_column: String,
}
