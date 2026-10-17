//! SQL text tools: formatting, completion and the keyword list shared with the editor's highlighter.

use nook_core::{quote_ident, Dialect};
use nook_model::Catalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordCase {
    Upper,
    Lower,
    Preserve,
}

impl KeywordCase {
    pub fn parse(s: &str) -> Option<KeywordCase> {
        match s.to_ascii_lowercase().as_str() {
            "upper" | "uppercase" => Some(KeywordCase::Upper),
            "lower" | "lowercase" => Some(KeywordCase::Lower),
            "preserve" | "keep" => Some(KeywordCase::Preserve),
            _ => None,
        }
    }
}

/// Clause heads that start a new line when formatting.
const LINE_BREAKERS: &[&str] = &[
    "select",
    "from",
    "where",
    "group by",
    "order by",
    "having",
    "limit",
    "offset",
    "insert into",
    "values",
    "update",
    "set",
    "delete from",
    "union",
    "union all",
    "returning",
    "join",
    "inner join",
    "left join",
    "left outer join",
    "right join",
    "full join",
    "cross join",
    "on",
    "and",
    "or",
    "with",
    "create table",
    "create index",
    "alter table",
    "drop table",
    "truncate table",
];

/// Words the formatter and the highlighter both treat as keywords.
pub fn keywords() -> &'static [&'static str] {
    &[
        "add",
        "all",
        "alter",
        "and",
        "any",
        "as",
        "asc",
        "begin",
        "between",
        "by",
        "cascade",
        "case",
        "cast",
        "check",
        "column",
        "commit",
        "constraint",
        "create",
        "cross",
        "database",
        "default",
        "delete",
        "desc",
        "distinct",
        "drop",
        "else",
        "end",
        "except",
        "exists",
        "explain",
        "false",
        "foreign",
        "from",
        "full",
        "grant",
        "group",
        "having",
        "if",
        "in",
        "index",
        "inner",
        "insert",
        "intersect",
        "into",
        "is",
        "join",
        "key",
        "left",
        "like",
        "limit",
        "not",
        "null",
        "offset",
        "on",
        "or",
        "order",
        "outer",
        "primary",
        "references",
        "returning",
        "revoke",
        "right",
        "rollback",
        "select",
        "set",
        "table",
        "then",
        "true",
        "truncate",
        "union",
        "unique",
        "update",
        "using",
        "values",
        "view",
        "when",
        "where",
        "window",
        "with",
    ]
}

pub fn functions() -> &'static [&'static str] {
    &[
        "abs",
        "avg",
        "cast",
        "ceil",
        "coalesce",
        "concat",
        "count",
        "current_date",
        "current_timestamp",
        "date_trunc",
        "extract",
        "floor",
        "greatest",
        "json_agg",
        "jsonb_build_object",
        "least",
        "length",
        "lower",
        "max",
        "min",
        "now",
        "nullif",
        "position",
        "round",
        "row_number",
        "substring",
        "sum",
        "to_char",
        "trim",
        "upper",
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Word,
    Keyword,
    Str,
    Number,
    Punct,
    Comment,
}

fn classify(word: &str) -> TokenKind {
    if keywords().iter().any(|k| k.eq_ignore_ascii_case(word)) {
        TokenKind::Keyword
    } else {
        TokenKind::Word
    }
}

/// Split SQL into words, literals, punctuation and comments. One tokenizer feeds formatting,
/// completion and the editor's highlighter so they can never disagree.
pub fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied().unwrap_or('\0');
        if c.is_whitespace() {
            let mut s = String::new();
            while i < chars.len() && chars[i].is_whitespace() {
                s.push(chars[i]);
                i += 1;
            }
            out.push(Token {
                text: s,
                kind: TokenKind::Punct,
            });
            continue;
        }
        if c == '-' && next == '-' {
            let mut s = String::new();
            while i < chars.len() && chars[i] != '\n' {
                s.push(chars[i]);
                i += 1;
            }
            out.push(Token {
                text: s,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if c == '/' && next == '*' {
            let mut s = String::new();
            s.push(chars[i]);
            i += 1;
            while i < chars.len() {
                s.push(chars[i]);
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    s.push('/');
                    i += 2;
                    break;
                }
                i += 1;
            }
            out.push(Token {
                text: s,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if c == '\'' || c == '"' || c == '`' {
            let quote = c;
            let mut s = String::new();
            s.push(c);
            i += 1;
            while i < chars.len() {
                s.push(chars[i]);
                if chars[i] == quote {
                    if chars.get(i + 1) == Some(&quote) {
                        s.push(quote);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Token {
                text: s,
                kind: TokenKind::Str,
            });
            continue;
        }
        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                s.push(chars[i]);
                i += 1;
            }
            out.push(Token {
                text: s,
                kind: TokenKind::Number,
            });
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '@' || c == '$' {
            let mut s = String::new();
            while i < chars.len()
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '$'
                    || chars[i] == '@')
            {
                s.push(chars[i]);
                i += 1;
            }
            let kind = classify(&s);
            out.push(Token { text: s, kind });
            continue;
        }
        out.push(Token {
            text: c.to_string(),
            kind: TokenKind::Punct,
        });
        i += 1;
    }
    out
}

/// Pretty-print a statement. Conservative on purpose: it never changes meaning, only whitespace,
/// keyword case and where the lines break.
pub fn format(sql: &str, case: KeywordCase, indent_width: usize) -> String {
    let tokens = tokenize(sql);
    let words: Vec<&Token> = tokens
        .iter()
        .filter(|t| !(t.kind == TokenKind::Punct && t.text.chars().all(char::is_whitespace)))
        .collect();
    let mut out = String::new();
    let mut depth: usize = 0;
    let mut space = false;
    let mut at_line_start = true;

    let push_indent = |out: &mut String, depth: usize| {
        for _ in 0..depth {
            out.push_str("  ");
        }
        let _ = indent_width;
    };

    let mut index = 0;
    while index < words.len() {
        let token = words[index];
        let mut starts_line = false;
        if token.kind == TokenKind::Comment {
            if !at_line_start {
                out.push('\n');
            }
            push_indent(&mut out, depth);
            out.push_str(token.text.trim_end());
            out.push('\n');
            at_line_start = true;
            space = false;
            index += 1;
            continue;
        }

        // A clause head starts a new line: `WHERE`, `ORDER BY`, `AND`, …
        let mut head: Option<String> = None;
        let mut head_words = 1usize;
        if token.kind == TokenKind::Keyword {
            let lower = token.text.to_lowercase();
            if LINE_BREAKERS.contains(&lower.as_str()) {
                head = Some(lower.clone());
            }
            if let Some(next) = words.get(index + 1) {
                if next.kind == TokenKind::Keyword {
                    let pair = format!("{lower} {}", next.text.to_lowercase());
                    if LINE_BREAKERS.contains(&pair.as_str()) {
                        head = Some(pair);
                        head_words = 2;
                    }
                }
            }
        }

        if let Some(head) = head {
            if !out.is_empty() && !at_line_start {
                out.push('\n');
            }
            let indent = if matches!(head.as_str(), "and" | "or" | "on") {
                depth + 1
            } else {
                depth
            };
            push_indent(&mut out, indent);
            for offset in 0..head_words {
                if offset > 0 {
                    out.push(' ');
                }
                out.push_str(&case_word(&words[index + offset].text, case));
            }
            at_line_start = false;
            space = true;
            index += head_words;
            continue;
        }

        match token.kind {
            TokenKind::Punct => match token.text.as_str() {
                "," => {
                    out.push(',');
                    out.push('\n');
                    push_indent(&mut out, depth + 1);
                    space = false;
                }
                "(" => {
                    if space && !at_line_start {
                        out.push(' ');
                    }
                    out.push('(');
                    depth += 1;
                    space = false;
                }
                ")" => {
                    depth = depth.saturating_sub(1);
                    out.push(')');
                    space = true;
                }
                ";" => {
                    out.push(';');
                    out.push('\n');
                    depth = 0;
                    starts_line = true;
                    space = false;
                }
                "." => {
                    out.push('.');
                    space = false;
                }
                other => {
                    if space && !at_line_start {
                        out.push(' ');
                    }
                    out.push_str(other);
                    space = true;
                }
            },
            TokenKind::Keyword => {
                if space && !at_line_start {
                    out.push(' ');
                }
                out.push_str(&case_word(&token.text, case));
                space = true;
            }
            _ => {
                if space && !at_line_start {
                    out.push(' ');
                }
                out.push_str(&token.text);
                space = true;
            }
        }
        at_line_start = starts_line;
        index += 1;
    }

    let mut text = out.trim_end().to_string();
    if !text.is_empty() && !text.ends_with(';') {
        text.push(';');
    }
    text
}

fn case_word(word: &str, case: KeywordCase) -> String {
    match case {
        KeywordCase::Upper => word.to_uppercase(),
        KeywordCase::Lower => word.to_lowercase(),
        KeywordCase::Preserve => word.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub text: String,
    pub kind: CompletionKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Function,
    Table,
    Column,
    Snippet,
}

impl CompletionKind {
    pub fn label(self) -> &'static str {
        match self {
            CompletionKind::Keyword => "keyword",
            CompletionKind::Function => "function",
            CompletionKind::Table => "table",
            CompletionKind::Column => "column",
            CompletionKind::Snippet => "snippet",
        }
    }
}

/// Completion candidates for the word being typed. `prefix` is the partial word before the caret;
/// the catalog supplies table and column names, and tables mentioned in the statement get priority.
pub fn complete(
    prefix: &str,
    statement: &str,
    catalog: Option<&Catalog>,
    dialect: Dialect,
    limit: usize,
) -> Vec<Completion> {
    let needle = prefix.trim().trim_matches(['"', '`']).to_ascii_lowercase();
    let mut out: Vec<Completion> = Vec::new();
    let mentioned: Vec<String> = mentioned_tables(statement);
    if let Some(catalog) = catalog {
        for schema in &catalog.schemas {
            for table in &schema.tables {
                let name = table.name.clone();
                if needle.is_empty() || name.to_ascii_lowercase().starts_with(&needle) {
                    out.push(Completion {
                        text: quote_ident(&name, dialect),
                        kind: CompletionKind::Table,
                        detail: format!("{} · {} columns", table.key(), table.columns.len()),
                    });
                }
                if mentioned.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
                    for column in &table.columns {
                        if needle.is_empty()
                            || column.name.to_ascii_lowercase().starts_with(&needle)
                        {
                            out.push(Completion {
                                text: quote_ident(&column.name, dialect),
                                kind: CompletionKind::Column,
                                detail: format!(
                                    "{}.{} · {}",
                                    table.name, column.name, column.type_name
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
    for k in keywords() {
        if needle.is_empty() || k.starts_with(&needle) {
            out.push(Completion {
                text: k.to_uppercase(),
                kind: CompletionKind::Keyword,
                detail: "keyword".into(),
            });
        }
    }
    for f in functions() {
        if !needle.is_empty() && f.starts_with(&needle) {
            out.push(Completion {
                text: format!("{f}("),
                kind: CompletionKind::Function,
                detail: "function".into(),
            });
        }
    }
    out.sort_by_key(|c| match c.kind {
        CompletionKind::Column => 0,
        CompletionKind::Table => 1,
        CompletionKind::Function => 2,
        CompletionKind::Keyword => 3,
        CompletionKind::Snippet => 4,
    });
    out.truncate(limit.clamp(1, 500));
    out
}

/// Table names mentioned after FROM/JOIN/UPDATE/INTO, used to prioritise column candidates.
/// A dotted `schema.table` yields both parts, in order.
pub fn mentioned_tables(statement: &str) -> Vec<String> {
    let tokens = tokenize(statement);
    let mut out = Vec::new();
    let mut expect = false;
    for token in &tokens {
        if token.kind == TokenKind::Punct && token.text.chars().all(char::is_whitespace) {
            continue;
        }
        if token.text == "." {
            expect = true;
            continue;
        }
        if token.kind == TokenKind::Keyword {
            let lower = token.text.to_lowercase();
            expect = matches!(
                lower.as_str(),
                "from" | "join" | "update" | "into" | "table"
            );
            continue;
        }
        if expect {
            if matches!(token.kind, TokenKind::Word | TokenKind::Str) {
                let name = token.text.trim_matches(['"', '`', '\'']).to_string();
                if !name.is_empty() && !out.contains(&name) {
                    out.push(name);
                }
            }
            expect = false;
        }
    }
    out
}

/// The statement under the caret, for "run statement at cursor".
pub fn statement_at(sql: &str, offset: usize) -> Option<String> {
    statement_under_caret(sql, offset).map(|(_, statement)| statement)
}

/// Index of the caret's statement in `split(sql)`, for "run statement at cursor" from the editor:
/// the caret's own index instead of a hard-coded zero.
pub fn statement_index_at(sql: &str, offset: usize) -> Option<usize> {
    statement_under_caret(sql, offset).map(|(index, _)| index)
}

/// One walk for both: the caret's statement and its position in the script cannot disagree.
fn statement_under_caret(sql: &str, offset: usize) -> Option<(usize, String)> {
    let offset = offset.min(sql.len());
    let byte_end = sql
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .rev()
        .find(|i| *i <= offset)
        .unwrap_or(0);
    let mut cursor = 0usize;
    for (index, statement) in crate::cmd::sql::split(sql).into_iter().enumerate() {
        let start = sql[cursor..].find(&statement).map(|i| cursor + i)?;
        let end = start + statement.len();
        if byte_end >= start && byte_end <= end {
            return Some((index, statement));
        }
        cursor = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nook_model::{Column, Schema, Table};

    fn catalog() -> Catalog {
        let mut users = Table::new(None, "users");
        users.columns.push(Column::new("id", "int"));
        users.columns.push(Column::new("email", "text"));
        let mut orders = Table::new(None, "orders");
        orders.columns.push(Column::new("id", "int"));
        orders.columns.push(Column::new("user_id", "int"));
        Catalog {
            schemas: vec![Schema {
                name: "main".into(),
                tables: vec![users, orders],
                ..Default::default()
            }],
            ..Catalog::default()
        }
    }

    #[test]
    fn formatting_puts_clauses_on_their_own_lines() {
        let out = format(
            "select a,b from t where a=1 and b=2 order by a;",
            KeywordCase::Upper,
            2,
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "SELECT a,");
        assert!(lines[1].trim_start().starts_with("b"));
        assert!(lines.iter().any(|l| l.starts_with("FROM t")));
        assert!(lines.iter().any(|l| l.starts_with("WHERE")));
        assert!(out.ends_with(';'));
    }

    #[test]
    fn formatting_preserves_strings_and_comments() {
        let sql = "select 'select from where' as x -- keep me\nfrom t;";
        let out = format(sql, KeywordCase::Upper, 2);
        assert!(out.contains("'select from where'"));
        assert!(out.contains("-- keep me"));
    }

    #[test]
    fn keyword_case_is_configurable() {
        let out = format("select 1 from t", KeywordCase::Lower, 2);
        assert!(out.starts_with("select"));
        let out = format("SeLeCt 1 FROM t", KeywordCase::Preserve, 2);
        assert!(out.contains("SeLeCt"));
    }

    #[test]
    fn formatting_is_stable_when_repeated() {
        let once = format("select a, b from t where a = 1;", KeywordCase::Upper, 2);
        let twice = format(&once, KeywordCase::Upper, 2);
        assert_eq!(once, twice);
    }

    #[test]
    fn hostile_sql_never_panics() {
        for sql in [
            "",
            "'",
            "\"unterminated",
            "/* open",
            "select (((",
            ";",
            "sélect 1",
        ] {
            let _ = format(sql, KeywordCase::Upper, 2);
            let _ = tokenize(sql);
            let _ = mentioned_tables(sql);
        }
    }

    #[test]
    fn completion_prefers_columns_of_mentioned_tables() {
        let cat = catalog();
        let out = complete(
            "",
            "SELECT * FROM users WHERE e",
            Some(&cat),
            Dialect::Sqlite,
            50,
        );
        assert!(out.iter().any(|c| c.text == "\"email\""));
        assert!(!out.iter().any(|c| c.text == "\"user_id\""));
        let out = complete("ord", "SELECT 1", Some(&cat), Dialect::Sqlite, 50);
        assert!(out.iter().any(|c| c.text == "\"orders\""));
        let out = complete("sel", "SELECT 1", Some(&cat), Dialect::Sqlite, 50);
        assert!(out.iter().any(|c| c.text == "SELECT"));
        let out = complete("count", "", None, Dialect::MySql, 50);
        assert!(out.iter().any(|c| c.text == "count("));
    }

    #[test]
    fn mentioned_tables_reads_the_statement() {
        assert_eq!(
            mentioned_tables("select * from \"public\".users join orders on 1=1"),
            vec!["public", "users", "orders"]
        );
        assert!(mentioned_tables("select 1").is_empty());
    }

    #[test]
    fn statement_at_finds_the_statement_under_the_caret() {
        let sql = "select 1;\nselect 2;\nselect 3;";
        let at = sql.find("select 2").unwrap_or(0) + 3;
        assert_eq!(statement_at(sql, at).as_deref(), Some("select 2;"));
        assert!(statement_at(sql, 0).is_some());
    }

    #[test]
    fn statement_index_at_numbers_the_statement_under_the_caret() {
        let sql = "select 1;\nselect 2;\nselect 3;";
        let at = |needle: &str| sql.find(needle).unwrap_or(0) + 3;
        assert_eq!(statement_index_at(sql, at("select 1")), Some(0));
        assert_eq!(statement_index_at(sql, at("select 2")), Some(1));
        assert_eq!(statement_index_at(sql, at("select 3")), Some(2));
        // A caret past the end sits in the last statement, exactly as `statement_at` reads it.
        assert_eq!(statement_index_at(sql, sql.len()), Some(2));
        assert_eq!(statement_at(sql, sql.len()).as_deref(), Some("select 3;"));
    }

    #[test]
    fn statement_index_at_reads_a_comment_as_the_statement_it_heads() {
        // A dump's header has no semicolon of its own, so it travels with the statement under it.
        let sql = "-- dump of shop\nselect 1;";
        assert_eq!(statement_index_at(sql, 3), Some(0));
        assert_eq!(
            statement_at(sql, 3).as_deref(),
            Some("-- dump of shop\nselect 1;")
        );
        // So does a note between two statements: it heads the one below it.
        let sql = "select 1;\n-- note\nselect 2;";
        assert_eq!(
            statement_index_at(sql, sql.find("-- note").unwrap_or(0) + 1),
            Some(1)
        );
        // A comment chunk that is a statement of its own is dropped by the splitter, so the caret
        // in it belongs to no statement — and says so rather than guessing.
        let sql = "select 1;\n/* note */;\nselect 2;";
        assert_eq!(
            statement_index_at(sql, sql.find("/* note").unwrap_or(0) + 1),
            None
        );
    }

    #[test]
    fn statement_index_at_ignores_a_semicolon_inside_a_string() {
        let sql = "select 'a;b';\nselect 2;";
        assert_eq!(
            statement_index_at(sql, sql.find(";b").unwrap_or(0)),
            Some(0)
        );
        assert_eq!(
            statement_index_at(sql, sql.find("select 2").unwrap_or(0) + 3),
            Some(1)
        );
    }
}
