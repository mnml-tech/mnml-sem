//! Shared function catalog — the single source of truth for the function set.
//!
//! Both emitters consume this: [`emit::substrait`](crate::emit::substrait) reads
//! the extension-group URNs and function anchors, while
//! [`plan::sql_format`](crate::plan) reads the [`SqlForm`] to render ANSI SQL.
//! Adding or changing a function is a single edit to [`CATALOG`], which keeps the
//! two emitters aligned by construction.

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
    And,
    Or,
    Subtract,
    CurrentDate,
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
}

/// A single function entry: its Substrait coordinates and SQL rendering.
#[derive(Debug, Clone, Copy)]
pub struct FuncDef {
    pub func: Func,
    pub group: ExtensionGroup,
    /// Substrait `function_anchor` / `function_reference`.
    pub anchor: u32,
    /// Substrait function name within its extension group.
    pub name: &'static str,
    /// ANSI SQL rendering.
    pub sql: SqlForm,
}

impl FuncDef {
    /// SQL call name for [`SqlForm::Call`] functions (e.g. aggregates); panics otherwise.
    pub fn call_name(&self) -> &'static str {
        match self.sql {
            SqlForm::Call(name) => name,
            _ => panic!("{:?} is not a SQL call function", self.func),
        }
    }
}

/// Every supported function. Order matters: it fixes the order of emitted
/// Substrait extension declarations.
pub const CATALOG: &[FuncDef] = &[
    FuncDef { func: Func::Sum, group: AGGREGATE, anchor: 1, name: "sum", sql: SqlForm::Call("SUM") },
    FuncDef { func: Func::Avg, group: AGGREGATE, anchor: 2, name: "avg", sql: SqlForm::Call("AVG") },
    FuncDef { func: Func::Count, group: AGGREGATE, anchor: 3, name: "count", sql: SqlForm::Call("COUNT") },
    FuncDef { func: Func::Min, group: AGGREGATE, anchor: 5, name: "min", sql: SqlForm::Call("MIN") },
    FuncDef { func: Func::Max, group: AGGREGATE, anchor: 6, name: "max", sql: SqlForm::Call("MAX") },
    FuncDef { func: Func::Equal, group: COMPARISON, anchor: 100, name: "equal", sql: SqlForm::Infix("=") },
    FuncDef { func: Func::NotEqual, group: COMPARISON, anchor: 101, name: "not_equal", sql: SqlForm::Infix("<>") },
    FuncDef { func: Func::And, group: BOOLEAN, anchor: 200, name: "and", sql: SqlForm::Variadic("AND") },
    FuncDef { func: Func::Or, group: BOOLEAN, anchor: 201, name: "or", sql: SqlForm::Variadic("OR") },
    FuncDef { func: Func::Subtract, group: ARITHMETIC, anchor: 300, name: "subtract", sql: SqlForm::InfixParen("-") },
    FuncDef { func: Func::CurrentDate, group: DATETIME, anchor: 400, name: "current_date", sql: SqlForm::Keyword("CURRENT_DATE") },
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
            vec![1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn anchors_are_unique() {
        for (i, a) in CATALOG.iter().enumerate() {
            for b in &CATALOG[i + 1..] {
                assert_ne!(a.anchor, b.anchor, "duplicate anchor for {:?}/{:?}", a.func, b.func);
            }
        }
    }
}
