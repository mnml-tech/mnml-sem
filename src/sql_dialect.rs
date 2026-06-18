//! Target SQL dialects — identifier quoting and per-function rendering.
//!
//! [`to_sql`](crate::emit::sql::to_sql) emits ANSI SQL; [`to_sql_with`](crate::emit::sql::to_sql_with)
//! takes any [`SqlDialect`]. A dialect inherits ANSI function rendering from the
//! shared [`catalog`](crate::catalog) by default and overrides only what differs.

use crate::catalog::{self, Func, SqlForm};

/// A target SQL dialect.
pub trait SqlDialect {
    /// Quote a bare identifier (e.g. a column alias).
    fn quote_ident(&self, ident: &str) -> String;

    /// Quote a (possibly dotted) source path used in `FROM` and column qualifiers.
    fn quote_table(&self, path: &str) -> String;

    /// Render a catalog function from pre-formatted operand SQL. Defaults to the
    /// catalog's ANSI [`SqlForm`]; override for dialect-specific spellings.
    fn render_func(&self, func: Func, operands: &[String]) -> String {
        render_ansi(func, operands)
    }
}

/// ANSI rendering of a catalog function — the default for every dialect.
pub fn render_ansi(func: Func, operands: &[String]) -> String {
    match catalog::def(func).sql {
        SqlForm::Infix(op) => format!("{} {op} {}", operands[0], operands[1]),
        SqlForm::InfixParen(op) => format!("({} {op} {})", operands[0], operands[1]),
        SqlForm::Variadic(op) => operands.join(&format!(" {op} ")),
        SqlForm::Call(name) => format!("{name}({})", operands.join(", ")),
        SqlForm::Keyword(kw) => kw.to_string(),
    }
}

/// `true` when `s` needs no quoting in any of our dialects.
fn is_bare_ident(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Wrap `s` in `quote`, escaping any embedded `quote` as `escaped`.
fn delimit(s: &str, quote: char, escaped: &str) -> String {
    format!("{quote}{}{quote}", s.replace(quote, escaped))
}

/// ANSI SQL (double-quoted identifiers). The default dialect.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ansi;

impl SqlDialect for Ansi {
    fn quote_ident(&self, ident: &str) -> String {
        if is_bare_ident(ident) {
            ident.to_string()
        } else {
            delimit(ident, '"', "\"\"")
        }
    }

    fn quote_table(&self, path: &str) -> String {
        crate::sql_quote::dotted_osi_name_as_sql_table_expr(path)
    }
}

/// Google BigQuery (backtick-quoted identifiers; `CURRENT_DATE()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BigQuery;

impl SqlDialect for BigQuery {
    fn quote_ident(&self, ident: &str) -> String {
        if is_bare_ident(ident) {
            ident.to_string()
        } else {
            delimit(ident, '`', "\\`")
        }
    }

    fn quote_table(&self, path: &str) -> String {
        // BigQuery quotes the whole `project.dataset.table` path in one pair of backticks.
        if path.contains('.') {
            delimit(path, '`', "\\`")
        } else {
            path.to_string()
        }
    }

    fn render_func(&self, func: Func, operands: &[String]) -> String {
        match func {
            Func::CurrentDate => "CURRENT_DATE()".to_string(),
            _ => render_ansi(func, operands),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_table_uses_double_quotes_for_dotted() {
        assert_eq!(Ansi.quote_table("a.b.c"), "\"a.b.c\"");
        assert_eq!(Ansi.quote_table("plain"), "plain");
    }

    #[test]
    fn bigquery_table_uses_backticks_for_dotted() {
        assert_eq!(BigQuery.quote_table("a.b.c"), "`a.b.c`");
        assert_eq!(BigQuery.quote_table("plain"), "plain");
    }

    #[test]
    fn bigquery_current_date_is_a_call() {
        assert_eq!(BigQuery.render_func(Func::CurrentDate, &[]), "CURRENT_DATE()");
        assert_eq!(Ansi.render_func(Func::CurrentDate, &[]), "CURRENT_DATE");
    }

    #[test]
    fn shared_funcs_match_ansi() {
        let operands = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            BigQuery.render_func(Func::Equal, &operands),
            Ansi.render_func(Func::Equal, &operands)
        );
    }
}
