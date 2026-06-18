//! Format structured [`Expr`](super::expr::Expr) as ANSI-style SQL fragments.

use super::expr::{ColumnRef, Expr, JoinKey, Literal};
use crate::catalog::{self, Func, SqlForm};
use crate::sql_quote::dotted_osi_name_as_sql_table_expr;

/// Render a catalog function from pre-formatted operand SQL.
fn render(func: Func, operands: &[String]) -> String {
    match catalog::def(func).sql {
        SqlForm::Infix(op) => format!("{} {op} {}", operands[0], operands[1]),
        SqlForm::InfixParen(op) => format!("({} {op} {})", operands[0], operands[1]),
        SqlForm::Variadic(op) => operands.join(&format!(" {op} ")),
        SqlForm::Call(name) => format!("{name}({})", operands.join(", ")),
        SqlForm::Keyword(kw) => kw.to_string(),
    }
}

pub fn format_column(col: &ColumnRef, qualify: bool) -> String {
    if qualify && !col.dataset.is_empty() {
        let prefix = dotted_osi_name_as_sql_table_expr(&col.dataset);
        format!("{prefix}.{}", col.sql)
    } else {
        col.sql.clone()
    }
}

pub fn format_expr(expr: &Expr, qualify: bool) -> String {
    match expr {
        Expr::Column(c) => format_column(c, qualify),
        Expr::Literal(lit) => format_literal(lit),
        Expr::Eq(l, r) => render(
            Func::Equal,
            &[format_expr(l, qualify), format_expr(r, qualify)],
        ),
        Expr::Ne(l, r) => render(
            Func::NotEqual,
            &[format_expr(l, qualify), format_expr(r, qualify)],
        ),
        Expr::And(exprs) => render(
            Func::And,
            &exprs.iter().map(|e| format_expr(e, qualify)).collect::<Vec<_>>(),
        ),
        Expr::In { expr, values } => {
            let vals: Vec<String> = values.iter().map(format_literal).collect();
            format!("{} IN ({})", format_expr(expr, qualify), vals.join(", "))
        }
        Expr::Case {
            when_then,
            else_expr,
        } => {
            let mut parts = Vec::new();
            for (when, then) in when_then {
                parts.push(format!(
                    "WHEN {} THEN {}",
                    format_expr(when, qualify),
                    format_expr(then, qualify)
                ));
            }
            format!(
                "CASE {} ELSE {} END",
                parts.join(" "),
                format_expr(else_expr, qualify)
            )
        }
        Expr::Sub(l, r) => render(
            Func::Subtract,
            &[format_expr(l, qualify), format_expr(r, qualify)],
        ),
        Expr::CurrentDate => render(Func::CurrentDate, &[]),
        Expr::Agg { func, distinct, arg } => {
            let distinct_s = if *distinct { "DISTINCT " } else { "" };
            let inner = match arg {
                None => "*".to_string(),
                Some(a) => format_expr(a, qualify),
            };
            let name = catalog::def(func.to_func()).call_name();
            format!("{name}({distinct_s}{inner})")
        }
        Expr::Sql(s) => s.clone(),
    }
}

pub fn format_join_on(keys: &[JoinKey], qualify: bool) -> Vec<String> {
    keys.iter()
        .map(|k| {
            let l = format_column(
                &ColumnRef::new(&k.left_dataset, &k.left_sql),
                qualify,
            );
            let r = format_column(
                &ColumnRef::new(&k.right_dataset, &k.right_sql),
                qualify,
            );
            format!("{l} = {r}")
        })
        .collect()
}

fn format_literal(lit: &Literal) -> String {
    match lit {
        Literal::Null => "NULL".to_string(),
        Literal::Bool(true) => "TRUE".to_string(),
        Literal::Bool(false) => "FALSE".to_string(),
        Literal::Int64(i) => i.to_string(),
        Literal::Float64(f) => f.to_string(),
        Literal::String(s) => format!("'{}'", s.replace('\'', "''")),
    }
}
