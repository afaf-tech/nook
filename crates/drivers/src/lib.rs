#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L2: the database drivers. One trait, three servers, plus the data-movement jobs built on it.

pub mod backup;
pub mod mysql;
pub mod pg;
pub mod sqlite;
pub mod transfer;
pub mod xlsx;

use nook_core::{quote_qualified, ConnectionConfig, Dialect, DriverKind, Error, Result};
use nook_model::{Catalog, Column, DbProcess, DbUser, ResultSet, ServerInfo, Table};

/// Rows a single fetch may hold before the driver stops reading. Grid pages ask for far fewer.
pub const MAX_ROWS: usize = 50_000;

/// Everything Nook can ask of a database. Introspection methods default to "not supported here"
/// so a new driver only writes what it can actually do.
pub trait Driver: Send {
    fn kind(&self) -> DriverKind;
    fn dialect(&self) -> Dialect;
    /// Which database this session is pointed at right now.
    fn current_database(&self) -> Option<String>;
    fn server_info(&mut self) -> Result<ServerInfo>;

    /// Run one statement and return whatever it produced.
    fn run(&mut self, sql: &str, limit: usize) -> Result<ResultSet>;

    /// Run several statements, returning the last tabular result and every notice.
    fn run_script(&mut self, sql: &str, limit: usize) -> Result<ResultSet> {
        let mut last = ResultSet::default();
        for stmt in crate::split_script(sql) {
            let rs = self.run(&stmt, limit)?;
            if rs.is_tabular() || last.columns.is_empty() {
                last.notices.extend(rs.notices.clone());
                let notices = std::mem::take(&mut last.notices);
                last = rs;
                last.notices.extend(notices);
            } else {
                last.affected += rs.affected;
                last.notices.extend(rs.notices);
            }
        }
        Ok(last)
    }

    fn catalog(&mut self) -> Result<Catalog>;

    /// Page of a table, ordered and filtered by the grid.
    fn fetch_table(
        &mut self,
        schema: Option<&str>,
        table: &str,
        limit: usize,
        offset: usize,
        order_by: Option<&str>,
        filter: Option<&str>,
    ) -> Result<ResultSet> {
        let mut sql = format!(
            "SELECT * FROM {}",
            quote_qualified(schema, table, self.dialect())
        );
        if let Some(f) = filter {
            if !f.trim().is_empty() {
                sql.push_str(&format!(" WHERE {}", f.trim()));
            }
        }
        if let Some(o) = order_by {
            if !o.trim().is_empty() {
                sql.push_str(&format!(" ORDER BY {}", o.trim()));
            }
        }
        sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
        self.run(&sql, limit)
    }

    /// Approximate row count for the status bar; drivers override with something cheap.
    fn count_rows(&mut self, schema: Option<&str>, table: &str) -> Result<u64> {
        let sql = format!(
            "SELECT COUNT(*) FROM {}",
            quote_qualified(schema, table, self.dialect())
        );
        let rs = self.run(&sql, 1)?;
        Ok(rs
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0) as u64)
    }

    /// The `CREATE` statement behind a table, straight from the server.
    fn table_ddl(&mut self, schema: Option<&str>, table: &str) -> Result<String> {
        let qualified = quote_qualified(schema, table, self.dialect());
        Err(Error::unsupported(format!(
            "{} cannot show DDL for {qualified}",
            self.kind().label()
        )))
    }

    /// Rebuild a table with new column definitions, for schema changes the server cannot ALTER in
    /// place. The statements run in order, inside one transaction. SQLite is the engine that needs
    /// it (a primary key cannot be added in place); the SQL engines answer through `ALTER TABLE`.
    fn rebuild_table(
        &mut self,
        _schema: Option<&str>,
        _table: &Table,
        _new_columns: &[Column],
    ) -> Result<Vec<String>> {
        Err(Error::unsupported(format!(
            "{} does not need a table rebuild",
            self.kind().label()
        )))
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        Err(Error::unsupported(format!(
            "{} has no database list",
            self.kind().label()
        )))
    }

    fn use_database(&mut self, database: &str) -> Result<()> {
        Err(Error::unsupported(format!(
            "{} cannot switch database to {database}",
            self.kind().label()
        )))
    }

    // ---- server monitor ----

    fn processes(&mut self) -> Result<Vec<DbProcess>> {
        Err(Error::unsupported(format!(
            "{} has no process list",
            self.kind().label()
        )))
    }

    fn kill_process(&mut self, id: &str) -> Result<()> {
        Err(Error::unsupported(format!(
            "{} cannot kill session {id}",
            self.kind().label()
        )))
    }

    fn variables(&mut self) -> Result<Vec<(String, String)>> {
        Err(Error::unsupported(format!(
            "{} has no server variables",
            self.kind().label()
        )))
    }

    fn status(&mut self) -> Result<Vec<(String, String)>> {
        Err(Error::unsupported(format!(
            "{} has no server status",
            self.kind().label()
        )))
    }

    // ---- privileges ----

    fn users(&mut self) -> Result<Vec<DbUser>> {
        Err(Error::unsupported(format!(
            "{} file databases have no users",
            self.kind().label()
        )))
    }

    fn grants(&mut self, user: &DbUser) -> Result<Vec<String>> {
        Err(Error::unsupported(format!(
            "{} cannot list grants for {}",
            self.kind().label(),
            user.display()
        )))
    }

    fn explain(&mut self, sql: &str) -> Result<ResultSet> {
        self.run(&format!("EXPLAIN {sql}"), 1000)
    }

    // ---- transactions ----

    fn begin(&mut self) -> Result<()> {
        self.run("BEGIN", 0).map(|_| ())
    }
    fn commit(&mut self) -> Result<()> {
        self.run("COMMIT", 0).map(|_| ())
    }
    fn rollback(&mut self) -> Result<()> {
        self.run("ROLLBACK", 0).map(|_| ())
    }
    fn in_transaction(&self) -> bool {
        false
    }

    /// Drop the connection, best effort.
    fn close(&mut self) {}
}

/// Open a connection for the profile. Enterprise kinds are catalogued but not implemented.
pub fn open(cfg: &ConnectionConfig) -> Result<Box<dyn Driver>> {
    match cfg.kind {
        DriverKind::Sqlite => Ok(Box::new(sqlite::Sqlite::open(cfg)?)),
        DriverKind::Postgres => Ok(Box::new(pg::Postgres::open(cfg)?)),
        DriverKind::MySql | DriverKind::MariaDb => Ok(Box::new(mysql::MySql::open(cfg)?)),
        other => Err(Error::unsupported(format!(
            "{} connections are listed in the catalog but not implemented yet",
            other.label()
        ))),
    }
}

/// Split a script into single statements, respecting quotes, dollar quoting and comments.
/// One place does this so the editor, the CLI, the importer and the dump restorer all agree.
pub fn split_script(sql: &str) -> Vec<String> {
    let bytes: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut dollar_tag: Option<String> = None;
    // SQLite trigger bodies: `CREATE TRIGGER … BEGIN …; END;`. A `;` inside the body must not
    // split, so depth counts unquoted `BEGIN` words. A `BEGIN` that *starts* a statement is the
    // transaction keyword and closes itself at the following `;` — depth is reset for those.
    let mut begin_depth: usize = 0;
    let mut word: String = String::new();
    let mut at_statement_start = true;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied().unwrap_or('\0');
        if in_line_comment {
            current.push(c);
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            current.push(c);
            if c == '*' && next == '/' {
                current.push(next);
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(tag) = dollar_tag.clone() {
            if c == '$' {
                let rest: String = bytes[i..].iter().take(tag.len() + 2).collect();
                if rest.starts_with(&tag) {
                    for _ in 0..tag.len() {
                        current.push(bytes.get(i).copied().unwrap_or(' '));
                        i += 1;
                    }
                    dollar_tag = None;
                    continue;
                }
            }
            current.push(c);
            i += 1;
            continue;
        }
        if in_single {
            current.push(c);
            if c == '\'' {
                if next == '\'' {
                    current.push(next);
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            current.push(c);
            if c == '"' {
                if next == '"' {
                    current.push(next);
                    i += 2;
                    continue;
                }
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                current.push(c);
            }
            '"' => {
                in_double = true;
                current.push(c);
            }
            '-' if next == '-' => {
                in_line_comment = true;
                current.push(c);
                current.push(next);
                i += 2;
                continue;
            }
            '/' if next == '*' => {
                in_block_comment = true;
                current.push(c);
                current.push(next);
                i += 2;
                continue;
            }
            '$' => {
                // Dollar-quoted string: $$…$$ or $tag$…$tag$
                let mut j = i + 1;
                let mut tag = String::from("$");
                while j < bytes.len() && (bytes[j].is_alphanumeric() || bytes[j] == '_') {
                    tag.push(bytes[j]);
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == '$' {
                    tag.push('$');
                    dollar_tag = Some(tag.clone());
                    current.push_str(&tag);
                    i = j + 1;
                    continue;
                }
                current.push(c);
            }
            ';' => {
                // A transaction `BEGIN …;` closes itself: its depth is not a trigger body.
                if begin_depth == 1 && word.trim().eq_ignore_ascii_case("begin") {
                    begin_depth = 0;
                }
                // `END;` may butt against the semicolon with no whitespace between.
                if word.eq_ignore_ascii_case("end") && begin_depth > 0 {
                    begin_depth -= 1;
                }
                if begin_depth == 0 {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() && !is_comment_only(trimmed) {
                        out.push(format!("{trimmed};"));
                    }
                    current.clear();
                    word.clear();
                    at_statement_start = true;
                } else {
                    current.push(c);
                    word.clear();
                }
            }
            c if c.is_alphanumeric() || c == '_' => {
                word.push(c);
                current.push(c);
            }
            c if c.is_whitespace() => {
                // A `BEGIN` that starts its statement is the transaction keyword; a `BEGIN`
                // anywhere else opens (or nests) a trigger body. `CASE` opens a balanced block
                // that its `END` closes, so it must count as a depth opener too — otherwise the
                // body's `END` is consumed by the CASE and the split lands early.
                if word.eq_ignore_ascii_case("begin") {
                    if at_statement_start && begin_depth == 0 {
                        // stays the transaction keyword: not counted
                    } else {
                        begin_depth += 1;
                    }
                } else if word.eq_ignore_ascii_case("case") && begin_depth > 0 {
                    begin_depth += 1;
                } else if word.eq_ignore_ascii_case("end") && begin_depth > 0 {
                    begin_depth -= 1;
                }
                // Leading whitespace between statements must not end the "statement start"
                // window — only a word does. `;` resets it, `word` tracking decides the rest.
                if !word.is_empty() {
                    at_statement_start = false;
                }
                word.clear();
                current.push(c);
            }
            _ => {
                // `END)` / `END,` / `END…` — the word is followed by punctuation, and this arm
                // must still pop the depth before the word is dropped.
                if word.eq_ignore_ascii_case("end") && begin_depth > 0 {
                    begin_depth -= 1;
                }
                word.clear();
                current.push(c);
            }
        }
        i += 1;
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() && !is_comment_only(trimmed) {
        out.push(trimmed.to_string());
    }
    out
}

/// A dump script carries `-- comment` lines between statements; replaying those is an error in
/// every engine, so they are dropped here.
fn is_comment_only(sql: &str) -> bool {
    let mut rest = sql;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("--") {
            match after.split_once('\n') {
                Some((_, tail)) => rest = tail,
                None => return true,
            }
            continue;
        }
        if let Some(after) = rest.strip_prefix("/*") {
            match after.split_once("*/") {
                Some((_, tail)) => rest = tail,
                None => return true,
            }
            continue;
        }
        return rest.is_empty();
    }
}

/// Refuse a write on a profile marked read-only, before it reaches the server.
pub(crate) fn guard_read_only(read_only: bool, sql: &str) -> Result<()> {
    if !read_only {
        return Ok(());
    }
    let head = sql.trim_start().lines().next().unwrap_or("").trim();
    let lower = head.to_ascii_lowercase();
    let writes = lower.starts_with("insert")
        || lower.starts_with("update")
        || lower.starts_with("delete")
        || lower.starts_with("drop")
        || lower.starts_with("create")
        || lower.starts_with("alter")
        || lower.starts_with("truncate")
        || lower.starts_with("grant")
        || lower.starts_with("revoke")
        || lower.starts_with("comment")
        || lower.starts_with("call")
        || lower.starts_with("do")
        || lower.starts_with("kill")
        || lower.starts_with("use ");
    if writes {
        return Err(Error::query(format!(
            "connection is read-only; refused: {}",
            if head.len() > 80 { &head[..80] } else { head }
        )));
    }
    Ok(())
}

/// Host/port/database summary used in messages and the title bar.
pub fn describe_target(cfg: &ConnectionConfig) -> String {
    match cfg.kind {
        DriverKind::Sqlite => cfg.file.clone(),
        _ => format!("{}@{}:{}/{}", cfg.user, cfg.host, cfg.port, cfg.database),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_statements_around_strings_comments_and_dollars() {
        let script = "SELECT ';' AS a;\n-- not a split;\nSELECT 2;\n/* x; y */ SELECT 3;";
        let parts = split_script(script);
        assert_eq!(parts.len(), 3, "{parts:?}");
        assert!(parts[0].contains("';'"));
        assert!(parts[2].contains("SELECT 3"));
        // A comment stays attached to the statement after it, so nothing is lost on replay.
        assert!(parts[1].contains("-- not a split"));
    }

    #[test]
    fn dollar_quoted_bodies_stay_intact() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN; RETURN 1; END; $$ LANGUAGE plpgsql;\nSELECT 1;";
        let parts = split_script(sql);
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(parts[0].contains("RETURN 1; END;"));
    }

    /// SQLite trigger bodies are `BEGIN … END;` with semicolons inside — the same shape a
    /// rebuild plan re-emits from sqlite_master. Splitting at the body semicolons produces
    /// fragments the engine rejects.
    #[test]
    fn trigger_bodies_stay_intact() {
        let sql = "CREATE TRIGGER t AFTER UPDATE ON posts BEGIN UPDATE posts SET title = title WHERE id = NEW.id; END;\nSELECT 1;";
        let parts = split_script(sql);
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(
            parts[0].contains("WHERE id = NEW.id; END;"),
            "the trigger body must survive as one statement: {parts:?}"
        );
    }

    /// `BEGIN IMMEDIATE;` starts a statement, so it is the transaction keyword, not a trigger
    /// body: it must close at its own semicolon and every statement after it must split normally.
    #[test]
    fn transaction_begin_immediate_closes_at_its_semicolon() {
        let sql = "BEGIN IMMEDIATE;\nSELECT 1;\nSELECT 2;\nCOMMIT;";
        let parts = split_script(sql);
        assert_eq!(parts.len(), 4, "{parts:?}");
        assert!(parts[0].contains("BEGIN IMMEDIATE"));
        // The same with the BEGIN mid-script: leading whitespace/newline after a prior `;` must
        // not hide the statement start from the keyword check.
        let mid = "SELECT 0;\nBEGIN IMMEDIATE;\nSELECT 1;\nCOMMIT;";
        let parts = split_script(mid);
        assert_eq!(parts.len(), 4, "{parts:?}");
        assert!(parts[1].contains("BEGIN IMMEDIATE"));
    }

    /// `CASE … END` inside a trigger body must not close the body early: the body's own END is
    /// what ends it.
    #[test]
    fn case_end_inside_a_trigger_body_does_not_close_it() {
        let sql = "CREATE TRIGGER t AFTER UPDATE ON posts BEGIN UPDATE posts SET title = CASE WHEN NEW.id > 1 THEN 'a' ELSE 'b' END WHERE id = NEW.id; END;\nSELECT 1;";
        let parts = split_script(sql);
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(
            parts[0].contains("WHEN NEW.id > 1") && parts[0].trim_end().ends_with("END;"),
            "the trigger body stays one statement: {parts:?}"
        );
    }

    /// `END` followed by punctuation — `END)` from a CASE in a parenthesised expression, or
    /// `END,` — must still pop the depth: the `_` arm sees the word and must not drop it.
    #[test]
    fn end_before_punctuation_still_closes_its_block() {
        let sql = "SELECT 0;\nCREATE TRIGGER t AFTER UPDATE ON posts BEGIN UPDATE posts SET total = (CASE WHEN NEW.id > 1 THEN 1 ELSE 2 END) + 1; END;\nSELECT 1;";
        let parts = split_script(sql);
        assert_eq!(parts.len(), 3, "{parts:?}");
        assert!(
            parts[1].contains("END) + 1; END;"),
            "the body with END) stays one statement: {parts:?}"
        );
        assert_eq!(parts[2], "SELECT 1;", "statements after the trigger split normally");
    }

    #[test]
    fn comment_only_chunks_are_dropped() {
        assert!(split_script("-- just a note\n").is_empty());
        assert!(split_script("/* block */").is_empty());
        assert_eq!(split_script("select 1; -- trailing note").len(), 1);
    }

    #[test]
    fn empty_and_whitespace_scripts_produce_nothing() {
        assert!(split_script("   \n\t ").is_empty());
        assert!(split_script(";;").is_empty());
        assert_eq!(split_script("select 1").len(), 1);
    }

    #[test]
    fn unterminated_quote_does_not_hang_or_panic() {
        let parts = split_script("SELECT 'unterminated");
        assert_eq!(parts.len(), 1);
    }
}
