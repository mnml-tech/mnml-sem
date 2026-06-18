//! Structured expressions for logical plans (Substrait-primary, SQL derived).

use serde_json::Value;

/// Reference to a column in a logical dataset (OSI dataset name + physical SQL fragment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRef {
    pub dataset: String,
    pub sql: String,
}

impl ColumnRef {
    pub fn new(dataset: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            dataset: dataset.into(),
            sql: sql.into(),
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
    pub fn from_json(v: &Value) -> Self {
        match v {
            Value::Null => Literal::Null,
            Value::Bool(b) => Literal::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Literal::Int64(i)
                } else if let Some(f) = n.as_f64() {
                    Literal::Float64(f)
                } else {
                    Literal::String(n.to_string())
                }
            }
            Value::String(s) => Literal::String(s.clone()),
            _ => Literal::String(v.to_string()),
        }
    }
}

/// Aggregate function for metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFunc {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggFunc {
    /// Logical [`catalog`](crate::catalog) function for this aggregate.
    pub fn to_func(self) -> crate::catalog::Func {
        use crate::catalog::Func;
        match self {
            AggFunc::Count => Func::Count,
            AggFunc::Sum => Func::Sum,
            AggFunc::Avg => Func::Avg,
            AggFunc::Min => Func::Min,
            AggFunc::Max => Func::Max,
        }
    }
}

/// Structured expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(ColumnRef),
    Literal(Literal),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    And(Vec<Expr>),
    In {
        expr: Box<Expr>,
        values: Vec<Literal>,
    },
    /// Metric / aggregate expression.
    Agg {
        func: AggFunc,
        distinct: bool,
        arg: Option<Box<Expr>>,
    },
    /// `CASE WHEN` chain (MNML `case` / derived fields).
    Case {
        when_then: Vec<(Expr, Expr)>,
        else_expr: Box<Expr>,
    },
    /// Binary subtraction (`sub` in MNML), e.g. `CURRENT_DATE - employment_start_date`.
    Sub(Box<Expr>, Box<Expr>),
    /// `CURRENT_DATE` (`current_date` in MNML).
    CurrentDate,
    /// Opaque SQL when the planner cannot structure the expression (Substrait may reject).
    Sql(String),
}

impl Expr {
    pub fn column(dataset: impl Into<String>, sql: impl Into<String>) -> Self {
        Expr::Column(ColumnRef::new(dataset, sql))
    }

    pub fn and(exprs: Vec<Expr>) -> Self {
        match exprs.len() {
            0 => Expr::Literal(Literal::Bool(true)),
            1 => exprs.into_iter().next().unwrap(),
            _ => Expr::And(exprs),
        }
    }
}

/// Equi-join key between two datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinKey {
    pub left_dataset: String,
    pub left_sql: String,
    pub right_dataset: String,
    pub right_sql: String,
}

/// Parse common metric SQL into structured [`Expr::Agg`], or [`Expr::Sql`] fallback.
pub fn parse_metric_sql(sql: &str) -> Expr {
    let s = sql.trim();
    let upper = s.to_uppercase();
    if upper == "COUNT(*)" || upper == "COUNT(1)" {
        return Expr::Agg {
            func: AggFunc::Count,
            distinct: false,
            arg: None,
        };
    }
    for (prefix, func) in [
        ("COUNT(", AggFunc::Count),
        ("SUM(", AggFunc::Sum),
        ("AVG(", AggFunc::Avg),
        ("MIN(", AggFunc::Min),
        ("MAX(", AggFunc::Max),
    ] {
        if let Some(inner) = s.strip_prefix(prefix).and_then(|t| t.strip_suffix(')')) {
            let distinct = inner.trim_start().to_uppercase().starts_with("DISTINCT ");
            let inner = if distinct {
                inner.trim()[8..].trim_start()
            } else {
                inner.trim()
            };
            if inner == "*" {
                return Expr::Agg {
                    func,
                    distinct,
                    arg: None,
                };
            }
            if let Some(arg) = parse_column_sql(inner) {
                return Expr::Agg {
                    func,
                    distinct,
                    arg: Some(Box::new(arg)),
                };
            }
            return Expr::Sql(s.to_string());
        }
    }
    Expr::Sql(s.to_string())
}

/// Parse `dataset.column` or bare `column` into a column reference.
pub fn parse_column_sql(sql: &str) -> Option<Expr> {
    let s = sql.trim();
    if s.is_empty() || s.contains(' ') || s.contains('(') {
        return None;
    }
    if let Some((ds, col)) = s.split_once('.') {
        if !ds.is_empty() && !col.is_empty() {
            return Some(Expr::column(ds, col));
        }
    }
    Some(Expr::column("", s))
}
