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

/// One placeholder found in a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// The name for `:name` / `${name}`; `None` for a positional `?`.
    pub name: Option<String>,
    /// How many times it appears. Positional slots are counted by index instead.
    pub count: usize,
}

/// The placeholders a script contains, and which style it uses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Params {
    pub style: ParamStyle,
    /// Named placeholders in order of first appearance; empty for positional style.
    pub named: Vec<Placeholder>,
    /// How many `?` slots the script has.
    pub positional: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParamStyle {
    #[default]
    None,
    Colon,
    Positional,
}

impl Params {
    pub fn is_empty(&self) -> bool {
        self.named.is_empty() && self.positional == 0
    }

    pub fn style_name(&self) -> &'static str {
        match self.style {
            ParamStyle::None => "none",
            ParamStyle::Colon => "colon",
            ParamStyle::Positional => "positional",
        }
    }

    /// Every name the script needs a value for, in order.
    pub fn names(&self) -> Vec<String> {
        self.named.iter().filter_map(|p| p.name.clone()).collect()
    }
}

/// Find the parameter placeholders in a script.
///
/// Runs on the token stream, not a regex: `:name` inside a string literal or a comment is text, and
/// `x::int` is a cast — the two things a regex gets wrong. `%name%` is deliberately not a style: it
/// collides with `LIKE '%x%'` and would eat a wildcard.
pub fn parameters(sql: &str) -> Params {
    let tokens = tokenize(sql);
    let mut out = Params::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        // A cast: `::` is two Punct tokens, or one, depending on the tokenizer's run.
        let is_double_colon = token.text == "::"
            || (token.text == ":" && tokens.get(index + 1).map(|t| t.text.as_str()) == Some(":"));
        if is_double_colon {
            index += if token.text == "::" { 1 } else { 2 };
            // Skip the type name that follows so `::text` is never read as `:text`.
            if tokens
                .get(index)
                .map(|t| matches!(t.kind, TokenKind::Word | TokenKind::Keyword))
                == Some(true)
            {
                index += 1;
            }
            continue;
        }
        if token.kind == TokenKind::Punct && token.text == ":" {
            if let Some(next) = tokens.get(index + 1) {
                if next.kind == TokenKind::Word || next.kind == TokenKind::Keyword {
                    // `${name}` tokenizes as `$` (word-ish) + `{` + `name` + `}`; the plain form is
                    // `:` + word.
                    push_named(&mut out, &next.text);
                    index += 2;
                    continue;
                }
            }
        }
        // `$` starts a Word token in this tokenizer, so `${name}` arrives as Word("$") + `{` + Word.
        if token.text == "$" {
            // `${name}`
            if tokens.get(index + 1).map(|t| t.text.as_str()) == Some("{") {
                if let Some(name) = tokens.get(index + 2) {
                    if matches!(name.kind, TokenKind::Word | TokenKind::Keyword)
                        && tokens.get(index + 3).map(|t| t.text.as_str()) == Some("}")
                    {
                        push_named(&mut out, &name.text);
                        index += 4;
                        continue;
                    }
                }
            }
        }
        // A Postgres dollar-quoted body: `$$ … $$` or `$tag$ … $tag$`. The tokenizer reads each
        // delimiter as one Word, so skip to the matching close and treat the body as text.
        if is_dollar_quote(&token.text) {
            let closer = token.text.clone();
            index += 1;
            while index < tokens.len() && tokens[index].text != closer {
                index += 1;
            }
            index += 1;
            continue;
        }
        if token.kind == TokenKind::Punct && token.text == "?" {
            out.positional += 1;
            out.style = ParamStyle::Positional;
            index += 1;
            continue;
        }
        index += 1;
    }
    out
}

/// Whether a token is a dollar-quote delimiter: `$$`, `$tag$`, `$_x$`.
fn is_dollar_quote(text: &str) -> bool {
    text.len() >= 2
        && text.starts_with('$')
        && text.ends_with('$')
        && text[1..text.len() - 1]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
        && !text[1..text.len() - 1].starts_with(|c: char| c.is_ascii_digit())
}

/// Replace matches of `find` in `text`, returning the new text and the byte range of each hit in the
/// **original** text.
///
/// Replace-all builds the result right to left so the earlier offsets stay valid, and a replacement
/// longer than the match never shifts a later hit. `word` uses the same boundary rule completion
/// does, so `user` does not match inside `users`.
pub fn find_replace(
    text: &str,
    find: &str,
    replace: &str,
    all: bool,
    case: bool,
    word: bool,
) -> std::result::Result<(String, Vec<(usize, usize)>), nook_core::Error> {
    if find.is_empty() {
        return Err(nook_core::Error::config(
            "an empty \"find\" would insert the replacement between every character",
        ));
    }
    let hits = find_hits(text, find, case, word);
    let hits: Vec<(usize, usize)> = if all { hits } else { hits.into_iter().take(1).collect() };
    if hits.is_empty() {
        return Ok((text.to_string(), Vec::new()));
    }
    let mut out = text.to_string();
    for &(start, end) in hits.iter().rev() {
        out.replace_range(start..end, replace);
    }
    Ok((out, hits))
}

/// Byte ranges of every match, left to right.
pub fn find_hits(text: &str, find: &str, case: bool, word: bool) -> Vec<(usize, usize)> {
    let hay = if case { text.to_string() } else { text.to_lowercase() };
    let needle = if case { find.to_string() } else { find.to_lowercase() };
    let mut out = Vec::new();
    let mut from = 0;
    while from <= hay.len() {
        let Some(at) = hay[from..].find(&needle) else {
            break;
        };
        let start = from + at;
        let end = start + needle.len();
        let boundary_ok = !word || {
            let before_ok = hay[..start]
                .chars()
                .next_back()
                .map(|c| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(true);
            let after_ok = hay[end..]
                .chars()
                .next()
                .map(|c| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(true);
            before_ok && after_ok
        };
        if boundary_ok {
            out.push((start, end));
        }
        from = start + needle.len().max(1);
    }
    out
}

/// Which placeholder syntax a driver wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindStyle {
    /// `?` — SQLite and MySQL.
    Question,
    /// `$1`, `$2` — Postgres.
    Dollar,
}

/// Rewrite a script's placeholders into the driver's positional form, returning the SQL and the
/// value each position takes: `Some(name)` for a named placeholder, `None` for a `?` slot.
///
/// The rewrite is a rebuild from the token stream, which reproduces the original text exactly except
/// at the placeholders, so a `:name` inside a literal or a comment is left alone.
pub fn bind_placeholders(sql: &str, style: BindStyle) -> (String, Vec<Option<String>>) {
    let tokens = tokenize(sql);
    let mut out = String::with_capacity(sql.len());
    let mut slots: Vec<Option<String>> = Vec::new();
    let next = |slots: &mut Vec<Option<String>>, name: Option<String>| -> String {
        slots.push(name);
        match style {
            BindStyle::Question => "?".to_string(),
            BindStyle::Dollar => format!("${}", slots.len()),
        }
    };
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let double_colon = token.text == "::"
            || (token.text == ":" && tokens.get(index + 1).map(|t| t.text.as_str()) == Some(":"));
        if double_colon {
            let take = if token.text == "::" { 1 } else { 2 };
            for t in &tokens[index..(index + take).min(tokens.len())] {
                out.push_str(&t.text);
            }
            index += take;
            continue;
        }
        if token.kind == TokenKind::Punct && token.text == ":" {
            if let Some(n) = tokens.get(index + 1) {
                if matches!(n.kind, TokenKind::Word | TokenKind::Keyword) {
                    out.push_str(&next(&mut slots, Some(n.text.clone())));
                    index += 2;
                    continue;
                }
            }
        }
        if token.text == "$"
            && tokens.get(index + 1).map(|t| t.text.as_str()) == Some("{")
            && tokens.get(index + 3).map(|t| t.text.as_str()) == Some("}")
        {
            if let Some(n) = tokens.get(index + 2) {
                if matches!(n.kind, TokenKind::Word | TokenKind::Keyword) {
                    out.push_str(&next(&mut slots, Some(n.text.clone())));
                    index += 4;
                    continue;
                }
            }
        }
        if is_dollar_quote(&token.text) {
            let closer = token.text.clone();
            out.push_str(&token.text);
            index += 1;
            while index < tokens.len() && tokens[index].text != closer {
                out.push_str(&tokens[index].text);
                index += 1;
            }
            if index < tokens.len() {
                out.push_str(&tokens[index].text);
                index += 1;
            }
            continue;
        }
        if token.kind == TokenKind::Punct && token.text == "?" {
            out.push_str(&next(&mut slots, None));
            index += 1;
            continue;
        }
        out.push_str(&token.text);
        index += 1;
    }
    (out, slots)
}

fn push_named(out: &mut Params, name: &str) {
    out.style = ParamStyle::Colon;
    match out.named.iter_mut().find(|p| p.name.as_deref() == Some(name)) {
        Some(existing) => existing.count += 1,
        None => out.named.push(Placeholder {
            name: Some(name.to_string()),
            count: 1,
        }),
    }
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

/// The table a `SELECT` reads from, when that is all it does: one table, no join, no `GROUP BY`,
/// no `DISTINCT`, no `UNION`, and no aggregate in the projection.
///
/// This is the editability rule for a query's results, so it is written to refuse: every construct
/// whose rows do not correspond one-to-one with a row of a table is rejected by name. A client that
/// quietly made a `GROUP BY` result editable would one day write one value over the group.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The table as written, with any `schema.` prefix kept.
    pub table: String,
    /// The projection's column names, or `None` for `*` (and for `t.*`).
    pub columns: Option<Vec<String>>,
}

impl Source {
    /// The schema part, when the statement named one.
    pub fn schema(&self) -> Option<&str> {
        self.table
            .split_once('.')
            .map(|(schema, _)| schema)
            .filter(|s| !s.is_empty())
    }

    /// The bare table name, without the schema prefix.
    pub fn name(&self) -> &str {
        match self.table.split_once('.') {
            Some((_, name)) => name,
            None => &self.table,
        }
    }
}

/// The single table a `SELECT` reads, or the reason it is not editable.
///
/// Returns `Err` with a sentence rather than `None`: "why is this result read-only" is the question
/// the register's footer has to answer, and a bare `None` cannot.
pub fn single_select_source(sql: &str) -> Result<Source, String> {
    // Comments and whitespace carry no meaning for this question, and a comment holding the word
    // `union` must not read as a union.
    let tokens: Vec<Token> = tokenize(sql)
        .into_iter()
        .filter(|t| t.kind != TokenKind::Comment)
        .filter(|t| !(t.kind == TokenKind::Punct && t.text.trim().is_empty()))
        .collect();
    let words: Vec<String> = tokens.iter().map(|t| t.text.clone()).collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    let find = |needle: &str| lower.iter().position(|w| w == needle);

    // Before the first word: a statement that opens with `WITH` is a CTE, and "only a SELECT" would
    // be a confusing way to say so.
    if lower.first().map(String::as_str) == Some("with") {
        return Err("this result is not editable: a CTE names a derived table".into());
    }
    if !matches!(lower.first().map(String::as_str), Some("select")) {
        return Err("only a SELECT can be made editable".into());
    }
    for (needle, why) in [
        ("union", "a UNION combines two statements"),
        ("intersect", "an INTERSECT combines two statements"),
        ("except", "an EXCEPT combines two statements"),
        ("group", "a GROUP BY folds many rows into one"),
        ("having", "a HAVING filters groups, not rows"),
        ("distinct", "DISTINCT drops the rows it deduplicates"),
        ("join", "a JOIN reads from more than one table"),
        ("over", "a window function reads more than one row"),
    ] {
        if find(needle).is_some() {
            return Err(format!("this result is not editable: {why}"));
        }
    }
    let Some(from) = find("from") else {
        return Err("this result is not editable: the statement has no FROM".into());
    };
    // A second SELECT after the first is a subquery: the rows it yields are that query's, not the
    // outer table's, and there is no single table to write back to.
    if lower.iter().skip(1).any(|w| w == "select") {
        return Err("this result is not editable: it reads from a subquery".into());
    }
    // An aggregate anywhere in the projection, e.g. `count(*)`.
    let aggregates = [
        "count",
        "sum",
        "avg",
        "min",
        "max",
        "total",
        "group_concat",
        "string_agg",
        "array_agg",
    ];
    if lower[1..from]
        .iter()
        .any(|w| aggregates.contains(&w.as_str()))
    {
        return Err("this result is not editable: an aggregate folds many rows into one".into());
    }
    // The table after FROM: `name`, `schema.name`, or a quoted name.
    let mut i = from + 1;
    let mut table = match words.get(i) {
        Some(name) if !is_keyword(name) => {
            i += 1;
            name.trim_matches(['"', '`', '\'']).to_string()
        }
        _ => return Err("this result is not editable: FROM names no table".into()),
    };
    if words.get(i).map(String::as_str) == Some(".") {
        if let Some(name) = words.get(i + 1) {
            if !is_keyword(name) {
                table = format!("{table}.{}", name.trim_matches(['"', '`', '\'']));
                i += 2;
            }
        }
    }
    // A comma after the table is a second table, which the JOIN check above cannot see.
    if words.get(i).map(String::as_str) == Some(",") {
        return Err("this result is not editable: it reads from more than one table".into());
    }
    // The projection: everything between SELECT and FROM.
    let projection: Vec<&str> = words[1..from].iter().map(String::as_str).collect();
    let columns = if projection.len() == 1 && projection[0] == "*" {
        None
    } else if projection.len() == 3 && projection[2] == "*" && projection[1] == "." {
        // `t.*` — every column of the one table, which is the same as `*` here.
        None
    } else {
        let mut out = Vec::new();
        for part in projection.join(" ").split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err("this result is not editable: the projection has an empty item".into());
            }
            // `expr AS name` takes the alias; `expr name` the trailing word. Anything with a
            // function call or an operator is not a plain column, and an unnamed expression has no
            // column to write back to.
            let name = match part.rsplit_once(" as ") {
                Some((_, alias)) => alias.trim(),
                None => {
                    let last = part.split_whitespace().last().unwrap_or(part);
                    // A call, an arithmetic operator or a literal is not a column: writing the
                    // result back would need to invert an expression, which this does not do.
                    let is_plain = !part.contains('(')
                        && !part.contains(')')
                        && !part
                            .chars()
                            .any(|c| matches!(c, '+' | '-' | '/' | '%' | '*' | '|' | '&'))
                        && !part.chars().next().is_some_and(|c| c.is_ascii_digit())
                        && !part.starts_with('\'');
                    if !is_plain {
                        return Err(
                            "this result is not editable: the projection has an expression".into(),
                        );
                    }
                    last
                }
            };
            let name = name.trim_matches(['"', '`', '\'']);
            if name.is_empty() || name.contains(' ') {
                return Err("this result is not editable: a projected column has no name".into());
            }
            out.push(name.to_string());
        }
        Some(out)
    };
    Ok(Source { table, columns })
}

fn is_keyword(word: &str) -> bool {
    keywords().iter().any(|k| k.eq_ignore_ascii_case(word))
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
mod editability_tests {
    use super::*;

    fn source(sql: &str) -> Result<Source, String> {
        single_select_source(sql)
    }

    #[test]
    fn a_plain_select_names_its_one_table_and_its_projection() {
        assert_eq!(
            source("select * from users").expect("editable"),
            Source {
                table: "users".into(),
                columns: None
            }
        );
        assert_eq!(
            source("SELECT id, email FROM users").expect("editable"),
            Source {
                table: "users".into(),
                columns: Some(vec!["id".into(), "email".into()])
            }
        );
        assert_eq!(
            source("select u.id from public.users u").expect("editable"),
            Source {
                table: "public.users".into(),
                columns: Some(vec!["id".into()])
            }
        );
        assert_eq!(
            source("select * from \"order items\"")
                .expect("editable")
                .table,
            "order items"
        );
    }

    #[test]
    fn every_shape_that_is_not_one_row_per_table_row_is_refused_by_name() {
        for (sql, needle) in [
            (
                "select status, count(*) from orders group by status",
                "GROUP BY",
            ),
            ("select distinct city from people", "DISTINCT"),
            ("select * from a join b on a.id = b.a_id", "JOIN"),
            ("select * from a, b", "more than one table"),
            ("select * from a union select * from b", "UNION"),
            ("select max(total) from orders", "aggregate"),
            ("select 1", "no FROM"),
            (
                "select id from t where id in (select id from u)",
                "editable",
            ),
            ("update t set a = 1", "only a SELECT"),
            ("with x as (select 1) select * from x", "CTE"),
        ] {
            let err = source(sql).expect_err(sql);
            assert!(
                err.contains(needle),
                "{sql:?} should be refused with {needle:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn a_comment_holding_a_keyword_does_not_make_the_select_uneditable() {
        // `-- union` is a comment, not a set operation.
        let out = source("select * from users -- union all\n").expect("editable");
        assert_eq!(out.table, "users");
    }

    #[test]
    fn an_expression_without_a_name_is_refused_rather_than_guessed() {
        let err = source("select id + 1 from users").expect_err("an expression is not a column");
        assert!(err.contains("expression"), "{err}");
    }

    #[test]
    fn a_schema_prefix_is_kept_and_split_where_it_is_asked_for() {
        let out = source("select * from analytics.events").expect("editable");
        assert_eq!(out.schema(), Some("analytics"));
        assert_eq!(out.name(), "events");
        let out = source("select * from events").expect("editable");
        assert_eq!(out.schema(), None);
        assert_eq!(out.name(), "events");
    }
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

    #[test]
    fn parameters_are_found_on_the_token_stream_not_by_regex() {
        let p = parameters("select * from u where email = :email");
        assert_eq!(p.style_name(), "colon");
        assert_eq!(p.names(), vec!["email"]);

        // A cast is not a parameter. This is the classic bug a regex has.
        let p = parameters("select * from u where x::int = 1 and y = :email");
        assert_eq!(p.names(), vec!["email"]);

        // Neither is text inside a literal or a comment.
        assert!(parameters("select 'a:name' as x").is_empty());
        assert!(parameters("-- :x\nselect 1").is_empty());
        assert!(parameters("/* :x */ select 1").is_empty());
    }

    #[test]
    fn a_repeated_name_is_one_parameter_with_a_count() {
        let p = parameters("select * from u where a = :x or b = :x or c = :y");
        assert_eq!(p.names(), vec!["x", "y"]);
        assert_eq!(p.named[0].count, 2);
        assert_eq!(p.named[1].count, 1);
    }

    #[test]
    fn positional_slots_are_counted_and_the_brace_alias_works() {
        let p = parameters("select * from u where a = ? and b = ?");
        assert_eq!(p.style_name(), "positional");
        assert_eq!(p.positional, 2);
        assert!(p.named.is_empty());

        let p = parameters("select * from u where a = ${name}");
        assert_eq!(p.names(), vec!["name"]);

        // A `?` inside a literal is text, not a slot.
        assert!(parameters("select 'a?b' as x").is_empty());
    }

    #[test]
    fn a_parameter_after_a_dollar_quoted_body_is_still_found() {
        let p = parameters("select $$ :notme $$ as body, :real as x");
        assert_eq!(p.names(), vec!["real"]);
    }

    #[test]
    fn binding_rewrites_only_the_placeholders() {
        let (sql, slots) = bind_placeholders(
            "select * from u where a = :x and b = :x and c = 'p:q'",
            BindStyle::Dollar,
        );
        assert_eq!(sql, "select * from u where a = $1 and b = $2 and c = 'p:q'");
        assert_eq!(
            slots,
            vec![Some("x".to_string()), Some("x".to_string())],
            "a repeated name fills two positions"
        );

        let (sql, slots) = bind_placeholders("select ? as a, x::int as b, ? as c", BindStyle::Question);
        assert_eq!(sql, "select ? as a, x::int as b, ? as c");
        assert_eq!(slots, vec![None, None]);
    }
}
