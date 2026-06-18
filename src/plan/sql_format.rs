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
    if ctx.qualify && !col.dataset.is_empty() {
        let prefix = ctx.dialect.quote_table(&col.dataset);
        format!("{prefix}.{}", col.sql)
    } else {
        col.sql.clone()
    }
}

pub fn format_expr(expr: &Expr, ctx: SqlCtx) -> String {
    match expr {
        Expr::Column(c) => format_column(c, ctx),
        Expr::Literal(lit) => format_literal(lit),
        Expr::Eq(l, r) => ctx
            .dialect
            .render_func(Func::Equal, &[format_expr(l, ctx), format_expr(r, ctx)]),
        Expr::Ne(l, r) => ctx
            .dialect
            .render_func(Func::NotEqual, &[format_expr(l, ctx), format_expr(r, ctx)]),
        Expr::And(exprs) => ctx.dialect.render_func(
            Func::And,
            &exprs.iter().map(|e| format_expr(e, ctx)).collect::<Vec<_>>(),
        ),
        Expr::In { expr, values } => {
            let vals: Vec<String> = values.iter().map(format_literal).collect();
            format!("{} IN ({})", format_expr(expr, ctx), vals.join(", "))
        }
        Expr::Case {
            when_then,
            else_expr,
        } => {
            let mut parts = Vec::new();
            for (when, then) in when_then {
                parts.push(format!(
                    "WHEN {} THEN {}",
                    format_expr(when, ctx),
                    format_expr(then, ctx)
                ));
            }
            format!(
                "CASE {} ELSE {} END",
                parts.join(" "),
                format_expr(else_expr, ctx)
            )
        }
        Expr::Sub(l, r) => ctx
            .dialect
            .render_func(Func::Subtract, &[format_expr(l, ctx), format_expr(r, ctx)]),
        Expr::CurrentDate => ctx.dialect.render_func(Func::CurrentDate, &[]),
        Expr::Agg { func, distinct, arg } => {
            let distinct_s = if *distinct { "DISTINCT " } else { "" };
            let inner = match arg {
                None => "*".to_string(),
                Some(a) => format_expr(a, ctx),
            };
            let name = catalog::def(func.to_func()).call_name();
            format!("{name}({distinct_s}{inner})")
        }
        Expr::Sql(s) => s.clone(),
    }
}

pub fn format_join_on(keys: &[JoinKey], ctx: SqlCtx) -> Vec<String> {
    keys.iter()
        .map(|k| {
            let l = format_column(&ColumnRef::new(&k.left_dataset, &k.left_sql), ctx);
            let r = format_column(&ColumnRef::new(&k.right_dataset, &k.right_sql), ctx);
            ctx.dialect.render_func(Func::Equal, &[l, r])
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
