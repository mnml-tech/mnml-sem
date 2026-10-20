//! Shared function catalog — the single source of truth for the function set.
//!
//! Both emitters consume this: the `ossie-substrait` crate reads
//! the extension-group URNs and function anchors, while
//! [`plan::sql_format`](crate::plan) reads the [`SqlForm`] to render ANSI SQL.
//! Adding or changing a function is a single edit to [`CATALOG`], which keeps the
//! two emitters aligned by construction.

use ossie_model::DataType;

/// A Substrait simple-extension group, identified by an anchor and its URN.
///
/// The anchor is the `extension_urn_anchor` referenced by every function declared
/// in the group; the `urn` is the `extension:<owner>:<id>` resource name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionGroup {
    pub anchor: u32,
    pub urn: &'static str,
}

pub const AGGREGATE: ExtensionGroup = ExtensionGroup {
    anchor: 1,
    urn: "extension:io.substrait:functions_aggregate_generic",
};
pub const COMPARISON: ExtensionGroup = ExtensionGroup {
    anchor: 2,
    urn: "extension:io.substrait:functions_comparison",
};
pub const BOOLEAN: ExtensionGroup = ExtensionGroup {
    anchor: 3,
    urn: "extension:io.substrait:functions_boolean",
};
pub const ARITHMETIC: ExtensionGroup = ExtensionGroup {
    anchor: 4,
    urn: "extension:io.substrait:functions_arithmetic",
};
pub const DATETIME: ExtensionGroup = ExtensionGroup {
    anchor: 5,
    urn: "extension:io.substrait:functions_datetime",
};
pub const STRING: ExtensionGroup = ExtensionGroup {
    anchor: 6,
    urn: "extension:io.substrait:functions_string",
};

/// Substrait's `unknown` type, for columns whose type the model does not
/// declare. Not a function group, so it is declared separately from [`CATALOG`].
pub const UNKNOWN_TYPE: ExtensionGroup = ExtensionGroup {
    anchor: 7,
    urn: "extension:io.substrait:unknown",
};

/// `type_anchor` for [`UNKNOWN_TYPE`]'s single type. Type anchors are a
/// separate namespace from function anchors.
pub const UNKNOWN_TYPE_ANCHOR: u32 = 1;

/// Whether a function aggregates rows or maps values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuncKind {
    Scalar,
    Aggregate {
        /// Whether repeating an input row changes the result.
        ///
        /// A join duplicates the rows of its one side, so a `SUM` over them is
        /// inflated while a `MIN` is not. Stating it here means adding an
        /// aggregate has to answer the question.
        duplicate_sensitive: bool,
    },
}

/// How many arguments a function accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Exact(usize),
    AtLeast(usize),
}

impl Arity {
    pub fn accepts(self, n: usize) -> bool {
        match self {
            Arity::Exact(k) => n == k,
            Arity::AtLeast(k) => n >= k,
        }
    }
}

/// What a function returns, in terms of its arguments.
///
/// Ossie's `datatype` is optional, so an unknown argument type stays unknown
/// rather than becoming a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    /// Always this type, whatever the arguments — comparisons are `Boolean`.
    Fixed(DataType),
    /// The type of argument `n` — `MIN`, `MAX`, `ABS`.
    SameAsArg(usize),
    /// The widest numeric argument — `+`, `-`, `*`, `/`.
    Numeric,
    /// The first argument with a known type — `COALESCE`.
    FirstKnown,
    /// Not determinable here.
    Unknown,
}

impl ReturnType {
    /// Resolve against the argument types, `None` where any input is unknown.
    pub fn resolve(self, args: &[Option<DataType>]) -> Option<DataType> {
        match self {
            ReturnType::Fixed(t) => Some(t),
            ReturnType::SameAsArg(n) => args.get(n).copied().flatten(),
            ReturnType::FirstKnown => args.iter().find_map(|a| *a),
            ReturnType::Numeric => args
                .iter()
                .copied()
                .try_fold(None::<DataType>, |acc, arg| match (acc, arg?) {
                    (None, t) => Some(Some(t)),
                    (Some(a), b) => Some(Some(wider_numeric(a, b))),
                })
                .flatten(),
            ReturnType::Unknown => None,
        }
    }
}

/// The type that holds both, for the numeric tower Ossie can express.
///
/// `Float` is approximate so it absorbs everything; `Decimal` is exact and
/// wider than `Integer`. A non-numeric operand makes the result unknown.
fn wider_numeric(a: DataType, b: DataType) -> DataType {
    use DataType::{Decimal, Float, Integer};
    match (a, b) {
        (Float, _) | (_, Float) => Float,
        (Decimal, _) | (_, Decimal) => Decimal,
        (Integer, Integer) => Integer,
        // Date arithmetic and anything else: keep the left operand's type,
        // which is what `DATEDIFF`'s own row overrides anyway.
        (other, _) => other,
    }
}

/// Logical function identity, independent of any target dialect or IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    IsNull,
    IsNotNull,
    Coalesce,
    NullIf,
    And,
    Or,
    Not,
    Add,
    Subtract,
    Multiply,
    Divide,
    Abs,
    Power,
    Sqrt,
    Concat,
    Lower,
    Upper,
    Trim,
    CurrentDate,
    /// Difference between two dates in days.
    DateDiff,
}

/// How a function renders in ANSI SQL. Tokens are bare operators; the formatter
/// supplies surrounding spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlForm {
    /// `a OP b` — e.g. `=`, `<>`.
    Infix(&'static str),
    /// `(a OP b)` — parenthesized, e.g. `-`.
    InfixParen(&'static str),
    /// `a OP b OP c …` — variadic, e.g. `AND`, `OR`.
    Variadic(&'static str),
    /// `NAME(args)` — function call, e.g. aggregates.
    Call(&'static str),
    /// `NAME` — niladic keyword, e.g. `CURRENT_DATE`.
    Keyword(&'static str),
    /// `OP a` — prefix operator, e.g. `NOT`.
    Prefix(&'static str),
    /// `a OP` — postfix operator, e.g. `IS NULL`.
    Postfix(&'static str),
}

/// A single function entry: its Substrait coordinates and SQL rendering.
#[derive(Debug, Clone, Copy)]
pub struct FuncDef {
    pub func: Func,
    /// Aggregate functions become Substrait measures; scalars become
    /// `ScalarFunction` expressions.
    pub kind: FuncKind,
    pub arity: Arity,
    pub group: ExtensionGroup,
    /// Substrait `function_anchor` / `function_reference`.
    pub anchor: u32,
    /// Substrait function name within its extension group.
    pub name: &'static str,
    /// ANSI SQL rendering.
    pub sql: SqlForm,
    /// The type this function produces.
    pub returns: ReturnType,
    /// SQL spellings that parse to this function.
    ///
    /// Empty for operators, which the parser reports structurally rather than
    /// by name. A function listed here is reachable from model expressions the
    /// moment it appears in [`CATALOG`].
    pub sql_names: &'static [&'static str],
}

impl FuncDef {
    pub fn is_aggregate(&self) -> bool {
        matches!(self.kind, FuncKind::Aggregate { .. })
    }

    /// Whether duplicated input rows would change this aggregate's result.
    /// Scalars are unaffected.
    pub fn is_duplicate_sensitive(&self) -> bool {
        matches!(
            self.kind,
            FuncKind::Aggregate {
                duplicate_sensitive: true
            }
        )
    }

    /// The SQL call name, for functions rendered as `NAME(args)`.
    pub fn name_for_sql(&self) -> &'static str {
        match self.sql {
            SqlForm::Call(name) | SqlForm::Keyword(name) => name,
            _ => self.sql_names.first().copied().unwrap_or(self.name),
        }
    }
}

/// Look up a function by a SQL spelling, case-insensitively.
pub fn by_sql_name(name: &str) -> Option<&'static FuncDef> {
    CATALOG
        .iter()
        .find(|d| d.sql_names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

/// Every supported function. Order matters: it fixes the order of emitted
/// Substrait extension declarations.
pub const CATALOG: &[FuncDef] = &[
    FuncDef {
        func: Func::Sum,
        kind: FuncKind::Aggregate {
            duplicate_sensitive: true,
        },
        arity: Arity::Exact(1),
        group: AGGREGATE,
        anchor: 1,
        name: "sum",
        sql: SqlForm::Call("SUM"),
        returns: ReturnType::SameAsArg(0),
        sql_names: &["SUM"],
    },
    FuncDef {
        func: Func::Avg,
        kind: FuncKind::Aggregate {
            duplicate_sensitive: true,
        },
        arity: Arity::Exact(1),
        group: AGGREGATE,
        anchor: 2,
        name: "avg",
        sql: SqlForm::Call("AVG"),
        returns: ReturnType::Fixed(DataType::Float),
        sql_names: &["AVG"],
    },
    FuncDef {
        func: Func::Count,
        kind: FuncKind::Aggregate {
            duplicate_sensitive: true,
        },
        arity: Arity::AtLeast(0),
        group: AGGREGATE,
        anchor: 3,
        name: "count",
        sql: SqlForm::Call("COUNT"),
        returns: ReturnType::Fixed(DataType::Integer),
        sql_names: &["COUNT"],
    },
    FuncDef {
        func: Func::Min,
        kind: FuncKind::Aggregate {
            duplicate_sensitive: false,
        },
        arity: Arity::Exact(1),
        group: AGGREGATE,
        anchor: 5,
        name: "min",
        sql: SqlForm::Call("MIN"),
        returns: ReturnType::SameAsArg(0),
        sql_names: &["MIN"],
    },
    FuncDef {
        func: Func::Max,
        kind: FuncKind::Aggregate {
            duplicate_sensitive: false,
        },
        arity: Arity::Exact(1),
        group: AGGREGATE,
        anchor: 6,
        name: "max",
        sql: SqlForm::Call("MAX"),
        returns: ReturnType::SameAsArg(0),
        sql_names: &["MAX"],
    },
    FuncDef {
        func: Func::Equal,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 100,
        name: "equal",
        sql: SqlForm::Infix("="),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::NotEqual,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 101,
        name: "not_equal",
        sql: SqlForm::Infix("<>"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::LessThan,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 102,
        name: "lt",
        sql: SqlForm::Infix("<"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::LessThanOrEqual,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 103,
        name: "lte",
        sql: SqlForm::Infix("<="),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::GreaterThan,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 104,
        name: "gt",
        sql: SqlForm::Infix(">"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::GreaterThanOrEqual,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 105,
        name: "gte",
        sql: SqlForm::Infix(">="),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::IsNull,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: COMPARISON,
        anchor: 106,
        name: "is_null",
        sql: SqlForm::Postfix("IS NULL"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::IsNotNull,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: COMPARISON,
        anchor: 107,
        name: "is_not_null",
        sql: SqlForm::Postfix("IS NOT NULL"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::Coalesce,
        kind: FuncKind::Scalar,
        arity: Arity::AtLeast(1),
        group: COMPARISON,
        anchor: 108,
        name: "coalesce",
        sql: SqlForm::Call("COALESCE"),
        returns: ReturnType::FirstKnown,
        sql_names: &["COALESCE"],
    },
    FuncDef {
        func: Func::NullIf,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: COMPARISON,
        anchor: 109,
        name: "nullif",
        sql: SqlForm::Call("NULLIF"),
        returns: ReturnType::SameAsArg(0),
        sql_names: &["NULLIF"],
    },
    FuncDef {
        func: Func::And,
        kind: FuncKind::Scalar,
        arity: Arity::AtLeast(2),
        group: BOOLEAN,
        anchor: 200,
        name: "and",
        sql: SqlForm::Variadic("AND"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::Or,
        kind: FuncKind::Scalar,
        arity: Arity::AtLeast(2),
        group: BOOLEAN,
        anchor: 201,
        name: "or",
        sql: SqlForm::Variadic("OR"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::Not,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: BOOLEAN,
        anchor: 202,
        name: "not",
        sql: SqlForm::Prefix("NOT"),
        returns: ReturnType::Fixed(DataType::Boolean),
        sql_names: &[],
    },
    FuncDef {
        func: Func::Add,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: ARITHMETIC,
        anchor: 300,
        name: "add",
        sql: SqlForm::InfixParen("+"),
        returns: ReturnType::Numeric,
        sql_names: &[],
    },
    FuncDef {
        func: Func::Subtract,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: ARITHMETIC,
        anchor: 301,
        name: "subtract",
        sql: SqlForm::InfixParen("-"),
        returns: ReturnType::Numeric,
        sql_names: &[],
    },
    FuncDef {
        func: Func::Multiply,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: ARITHMETIC,
        anchor: 302,
        name: "multiply",
        sql: SqlForm::InfixParen("*"),
        returns: ReturnType::Numeric,
        sql_names: &[],
    },
    FuncDef {
        func: Func::Divide,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: ARITHMETIC,
        anchor: 303,
        name: "divide",
        sql: SqlForm::InfixParen("/"),
        returns: ReturnType::Numeric,
        sql_names: &[],
    },
    FuncDef {
        func: Func::Abs,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: ARITHMETIC,
        anchor: 304,
        name: "abs",
        sql: SqlForm::Call("ABS"),
        returns: ReturnType::SameAsArg(0),
        sql_names: &["ABS"],
    },
    FuncDef {
        func: Func::Power,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: ARITHMETIC,
        anchor: 305,
        name: "power",
        sql: SqlForm::Call("POWER"),
        returns: ReturnType::Fixed(DataType::Float),
        sql_names: &["POWER"],
    },
    FuncDef {
        func: Func::Sqrt,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: ARITHMETIC,
        anchor: 306,
        name: "sqrt",
        sql: SqlForm::Call("SQRT"),
        returns: ReturnType::Fixed(DataType::Float),
        sql_names: &["SQRT"],
    },
    FuncDef {
        func: Func::CurrentDate,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(0),
        group: DATETIME,
        anchor: 400,
        name: "current_date",
        sql: SqlForm::Keyword("CURRENT_DATE"),
        returns: ReturnType::Fixed(DataType::Date),
        sql_names: &["CURRENT_DATE"],
    },
    FuncDef {
        func: Func::Concat,
        kind: FuncKind::Scalar,
        arity: Arity::AtLeast(1),
        group: STRING,
        anchor: 500,
        name: "concat",
        sql: SqlForm::Variadic("||"),
        returns: ReturnType::Fixed(DataType::String),
        sql_names: &["CONCAT"],
    },
    FuncDef {
        func: Func::Lower,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: STRING,
        anchor: 501,
        name: "lower",
        sql: SqlForm::Call("LOWER"),
        returns: ReturnType::Fixed(DataType::String),
        sql_names: &["LOWER"],
    },
    FuncDef {
        func: Func::Upper,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: STRING,
        anchor: 502,
        name: "upper",
        sql: SqlForm::Call("UPPER"),
        returns: ReturnType::Fixed(DataType::String),
        sql_names: &["UPPER"],
    },
    FuncDef {
        func: Func::Trim,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(1),
        group: STRING,
        anchor: 503,
        name: "trim",
        sql: SqlForm::Call("TRIM"),
        returns: ReturnType::Fixed(DataType::String),
        sql_names: &["TRIM"],
    },
    FuncDef {
        func: Func::DateDiff,
        kind: FuncKind::Scalar,
        arity: Arity::Exact(2),
        group: ARITHMETIC,
        anchor: 307,
        name: "subtract",
        sql: SqlForm::InfixParen("-"),
        returns: ReturnType::Fixed(DataType::Integer),
        sql_names: &["DATEDIFF", "DATE_DIFF"],
    },
];

/// Look up a function's definition.
pub fn def(func: Func) -> &'static FuncDef {
    CATALOG
        .iter()
        .find(|d| d.func == func)
        .expect("every Func has a catalog entry")
}

/// Distinct extension groups, in first-seen catalog order (for emitting URNs).
pub fn extension_groups() -> Vec<ExtensionGroup> {
    let mut groups: Vec<ExtensionGroup> = Vec::new();
    for d in CATALOG {
        if !groups.iter().any(|g| g.anchor == d.group.anchor) {
            groups.push(d.group);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_func_resolves() {
        for d in CATALOG {
            assert_eq!(def(d.func).anchor, d.anchor);
        }
    }

    #[test]
    fn extension_groups_are_deduped_in_order() {
        let groups = extension_groups();
        assert_eq!(
            groups.iter().map(|g| g.anchor).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn anchors_are_unique() {
        for (i, a) in CATALOG.iter().enumerate() {
            for b in &CATALOG[i + 1..] {
                assert_ne!(
                    a.anchor, b.anchor,
                    "duplicate anchor for {:?}/{:?}",
                    a.func, b.func
                );
            }
        }
    }
}
