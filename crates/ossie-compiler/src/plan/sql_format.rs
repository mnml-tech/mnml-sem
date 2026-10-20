//! Format structured [`Expr`](super::expr::Expr) as SQL fragments for a given dialect.

use super::expr::{ColumnRef, Expr, JoinKey, Literal};
use crate::catalog::{self, Func};
use crate::sql_dialect::SqlDialect;

/// Invariant context for SQL formatting: target dialect plus whether columns
/// must be qualified (multi-table queries). Threaded unchanged through the
/// formatter, so it is cheap to copy.
#[derive(Clone, Copy)]
pub struct SqlCtx<'a> {
    pub dialect: &'a dyn SqlDialect,
    pub qualify: bool,
}

pub fn format_column(col: &ColumnRef, ctx: SqlCtx) -> String {
    let name = ctx.dialect.quote_ident(&col.column);
    if ctx.qualify && !col.dataset.is_empty() {
        // A dataset alias is one identifier, not a qualified source path.
        format!("{}.{name}", ctx.dialect.quote_ident(&col.dataset))
    } else {
        name
    }
}

pub fn format_expr(expr: &Expr, ctx: SqlCtx) -> String {
    match expr {
        Expr::Column(c) => format_column(c, ctx),
        Expr::Literal(lit) => format_literal(lit, ctx),
        Expr::Call { func, args } => {
            let rendered: Vec<String> = args.iter().map(|a| format_expr(a, ctx)).collect();
            ctx.dialect.render_func(*func, &rendered)
        }
        Expr::Agg {
            func,
            distinct,
            args,
        } => {
            let name = catalog::def(*func).name_for_sql();
            let inner = if args.is_empty() {
                "*".to_string()
            } else {
                args.iter()
                    .map(|a| format_expr(a, ctx))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let distinct = if *distinct { "DISTINCT " } else { "" };
            format!("{name}({distinct}{inner})")
        }
        Expr::Case {
            when_then,
            else_expr,
        } => {
            let arms: Vec<String> = when_then
                .iter()
                .map(|(w, t)| format!("WHEN {} THEN {}", format_expr(w, ctx), format_expr(t, ctx)))
                .collect();
            format!(
                "CASE {} ELSE {} END",
                arms.join(" "),
                format_expr(else_expr, ctx)
            )
        }
        Expr::InList { expr, values } => {
            let vals: Vec<String> = values.iter().map(|v| format_literal(v, ctx)).collect();
            format!("{} IN ({})", format_expr(expr, ctx), vals.join(", "))
        }
    }
}

pub fn format_join_on(keys: &[JoinKey], ctx: SqlCtx) -> Vec<String> {
    keys.iter()
        .map(|k| {
            let l = format_column(&ColumnRef::new(&k.left_dataset, &k.left_column), ctx);
            let r = format_column(&ColumnRef::new(&k.right_dataset, &k.right_column), ctx);
            ctx.dialect.render_func(Func::Equal, &[l, r])
        })
        .collect()
}

fn format_literal(lit: &Literal, ctx: SqlCtx) -> String {
    match lit {
        Literal::Null => "NULL".to_string(),
        Literal::Bool(true) => "TRUE".to_string(),
        Literal::Bool(false) => "FALSE".to_string(),
        Literal::Int64(i) => i.to_string(),
        Literal::Float64(f) => f.to_string(),
        Literal::String(s) => ctx.dialect.quote_string_literal(s),
    }
}
