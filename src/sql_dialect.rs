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

    /// Quote a [`Dataset::source`] path for `FROM`.
    ///
    /// `source` is a dot-separated qualified name (`database.schema.table`, per
    /// the core spec), so dialects differ in whether each part is quoted
    /// separately or the whole path is wrapped once.
    ///
    /// [`Dataset::source`]: crate::model::Dataset::source
    fn quote_source(&self, path: &str) -> String;

    /// Quote a string literal, including whatever escaping the dialect needs.
    ///
    /// The ANSI default doubles embedded single quotes. A dialect that also
    /// treats backslash as an escape character must override this.
    fn quote_string_literal(&self, value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

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
        SqlForm::Prefix(op) => format!("{op} {}", operands[0]),
        SqlForm::Postfix(op) => format!("{} {op}", operands[0]),
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

    /// Each part is its own identifier: `a.b.c` becomes `"a"."b"."c"`.
    fn quote_source(&self, path: &str) -> String {
        path.split('.')
            .map(|part| self.quote_ident(part))
            .collect::<Vec<_>>()
            .join(".")
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

    /// BigQuery quotes the whole `project.dataset.table` path in one pair of
    /// backticks rather than quoting each part.
    fn quote_source(&self, path: &str) -> String {
        if path.contains('.') {
            delimit(path, '`', "\\`")
        } else {
            path.to_string()
        }
    }

    /// BigQuery string literals also honour backslash escapes, so a trailing
    /// backslash would otherwise escape the closing quote.
    fn quote_string_literal(&self, value: &str) -> String {
        format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
    }

    fn render_func(&self, func: Func, operands: &[String]) -> String {
        match func {
            Func::CurrentDate => "CURRENT_DATE()".to_string(),
            // BigQuery has no date arithmetic operator; it needs the call form
            // with an explicit unit.
            Func::DateDiff => format!("DATE_DIFF({}, {}, DAY)", operands[0], operands[1]),
            _ => render_ansi(func, operands),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dotted `source` is a qualified name, so ANSI quotes each part.
    #[test]
    fn ansi_source_quotes_each_part() {
        assert_eq!(Ansi.quote_source("a.b.c"), "a.b.c");
        assert_eq!(Ansi.quote_source("plain"), "plain");
        assert_eq!(Ansi.quote_source("we ird.tbl"), "\"we ird\".tbl");
    }

    #[test]
    fn bigquery_source_wraps_the_whole_path() {
        assert_eq!(BigQuery.quote_source("a.b.c"), "`a.b.c`");
        assert_eq!(BigQuery.quote_source("plain"), "plain");
    }

    #[test]
    fn ansi_string_literals_double_single_quotes() {
        assert_eq!(Ansi.quote_string_literal("O'Brien"), "'O''Brien'");
    }

    /// A trailing backslash must not escape the closing quote in BigQuery.
    #[test]
    fn bigquery_string_literals_escape_backslashes() {
        assert_eq!(BigQuery.quote_string_literal("a\\"), "'a\\\\'");
        assert_eq!(BigQuery.quote_string_literal("O'Brien"), "'O\\'Brien'");
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

    #[test]
    fn date_diff_differs_by_dialect() {
        let ops = vec!["CURRENT_DATE".to_string(), "d".to_string()];
        assert_eq!(Ansi.render_func(Func::DateDiff, &ops), "(CURRENT_DATE - d)");
        let ops = vec!["CURRENT_DATE()".to_string(), "d".to_string()];
        assert_eq!(
            BigQuery.render_func(Func::DateDiff, &ops),
            "DATE_DIFF(CURRENT_DATE(), d, DAY)"
        );
    }
}
