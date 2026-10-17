//! The app's own SQLite database: where the AI configuration lives.
//!
//! Deliberately not `profiles.json`. A profile is a connection the user manages in the tree; this is
//! state the app keeps about itself. The file is `nook.db` beside the profile store, mode `0600`, and
//! it holds no secret — an API key stays in the login keychain, which is the one place a key can live
//! that this file cannot be read from.
//!
//! SQLite through `rusqlite` directly, rather than the `Driver` trait: `nook_drivers::Driver` speaks
//! catalogues, result sets and tunnels for a *user's* database, which is the wrong shape for a
//! one-row table and would drag connection state into app state.
//!
//! ponytail: the table has no version column. A schema change here is a change to one row, so the
//! migration is rewriting it; add a version and a real migration when the app's state outgrows a
//! single row per concern.

use rusqlite::{params, Connection, OptionalExtension};

use nook_ai::{AiConfig, Provider};
use nook_core::{Error, Result};

use crate::{HistoryEntry, Snippet};

/// Applied on every open, so a file that exists without its table — hand-made, or left by an
/// interrupted first write — is repaired instead of trusted.
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS ai_config (
  id           INTEGER PRIMARY KEY CHECK (id = 1),
  provider     TEXT    NOT NULL,
  model        TEXT    NOT NULL,
  base_url     TEXT    NOT NULL,
  max_tokens   INTEGER NOT NULL,
  temperature  REAL    NOT NULL,
  timeout_secs INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS snippet (
  name       TEXT PRIMARY KEY,
  sql        TEXT NOT NULL,
  tags       TEXT NOT NULL DEFAULT '[]',
  updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS history (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  conn       TEXT NOT NULL,
  sql        TEXT NOT NULL,
  ok         INTEGER NOT NULL,
  elapsed_ms REAL NOT NULL,
  rows       INTEGER NOT NULL,
  at         INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS ui_state (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pinned (
  conn_id   TEXT PRIMARY KEY,
  pin_order INTEGER NOT NULL
);";

/// Bumped when the schema grows. A file at a lower version — or at 0, meaning it predates the
/// version column — gets the full `SCHEMA` applied on open, which is idempotent (`IF NOT EXISTS`).
const USER_VERSION: i64 = 1;

/// The columns of the single row, in the order `load_ai` reads them and `save_ai` writes them.
const COLUMNS: &str = "provider, model, base_url, max_tokens, temperature, timeout_secs";

/// Open the database, creating the file and the schema when they are not there yet.
fn open() -> Result<Connection> {
    let path = crate::profiles::ai_db_path();
    crate::profiles::ensure_dir()?;
    let conn =
        Connection::open(&path).map_err(|e| Error::io(format!("open {}: {e}", path.display())))?;
    // SQLite creates the file at the umask's mode, so an owner-only mode has to be applied rather
    // than assumed. This is the same treatment the profile store gets.
    crate::profiles::restrict_permissions(&path);
    conn.execute_batch(SCHEMA)
        .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
    if version < USER_VERSION {
        conn.pragma_update(None, "user_version", USER_VERSION)
            .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
    }
    Ok(conn)
}

/// The saved AI configuration, or `None` when nothing has been saved yet.
///
/// A missing file reads as nothing saved and is *not* created: a start must not add to the user's
/// config directory just because it looked. A row that cannot be read is an error rather than a
/// half-filled configuration: the caller is the only place that knows how to warn, and a partly-read
/// configuration would be worse than none.
pub fn load_ai() -> Result<Option<AiConfig>> {
    let path = crate::profiles::ai_db_path();
    if !path.exists() {
        return Ok(None);
    }
    let conn = open()?;
    let row = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM ai_config WHERE id = 1"),
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
    let Some((provider, model, base_url, max_tokens, temperature, timeout_secs)) = row else {
        return Ok(None);
    };
    let provider = Provider::parse(&provider).ok_or_else(|| {
        Error::config(format!(
            "{}: unknown AI provider '{provider}'",
            path.display()
        ))
    })?;
    let max_tokens = u32::try_from(max_tokens)
        .map_err(|_| Error::config(format!("{}: max_tokens out of range", path.display())))?;
    let timeout_secs = u64::try_from(timeout_secs)
        .map_err(|_| Error::config(format!("{}: timeout out of range", path.display())))?;
    Ok(Some(AiConfig {
        provider,
        model,
        base_url,
        max_tokens,
        temperature: temperature as f32,
        timeout_secs,
    }))
}

/// Write the configuration as the single row. The upsert is what keeps a second save from becoming a
/// second row, and `CHECK (id = 1)` is what makes that impossible rather than merely intended.
pub fn save_ai(cfg: &AiConfig) -> Result<()> {
    let path = crate::profiles::ai_db_path();
    let max_tokens = i64::from(cfg.max_tokens);
    let timeout_secs = i64::try_from(cfg.timeout_secs)
        .map_err(|_| Error::config("the AI timeout is too large to store"))?;
    let conn = open()?;
    conn.execute(
        &format!(
            "INSERT INTO ai_config (id, {COLUMNS}) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET provider = ?1, model = ?2, base_url = ?3, \
             max_tokens = ?4, temperature = ?5, timeout_secs = ?6"
        ),
        params![
            cfg.provider.as_str(),
            cfg.model.as_str(),
            cfg.base_url.as_str(),
            max_tokens,
            f64::from(cfg.temperature),
            timeout_secs,
        ],
    )
    .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
    Ok(())
}

// --- Snippets ---------------------------------------------------------------

/// Every saved snippet, oldest first (rowid order — save order).
pub fn load_snippets() -> Result<Vec<Snippet>> {
    let conn = open()?;
    let mut stmt = conn
        .prepare("SELECT name, sql, tags FROM snippet ORDER BY rowid")
        .map_err(db)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(db)?;
    let mut out = Vec::new();
    for row in rows {
        let (name, sql, tags) = row.map_err(db)?;
        out.push(Snippet {
            name,
            sql,
            tags: serde_json::from_str(&tags).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Insert or update one snippet by name. Tags are stored as a JSON array.
pub fn save_snippet(name: &str, sql: &str, tags: &[String]) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "INSERT INTO snippet (name, sql, tags, updated_at) VALUES (?1, ?2, ?3, unixepoch()) \
         ON CONFLICT(name) DO UPDATE SET sql = ?2, tags = ?3, updated_at = unixepoch()",
        params![name, sql, serde_json::to_string(tags).unwrap_or_else(|_| "[]".into())],
    )
    .map_err(db)?;
    Ok(())
}

/// Remove one snippet. A name that is not there is a no-op, so the caller stays idempotent.
pub fn delete_snippet(name: &str) -> Result<()> {
    let conn = open()?;
    conn.execute("DELETE FROM snippet WHERE name = ?1", params![name])
        .map_err(db)?;
    Ok(())
}

// --- History ----------------------------------------------------------------

/// Append one history entry.
pub fn push_history(entry: &HistoryEntry) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "INSERT INTO history (conn, sql, ok, elapsed_ms, rows, at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            entry.conn,
            entry.sql,
            i64::from(entry.ok),
            entry.elapsed_ms,
            i64::try_from(entry.rows).unwrap_or(i64::MAX),
            entry.at as i64,
        ],
    )
    .map_err(db)?;
    Ok(())
}

/// Keep only the newest `keep` rows — the stored-file twin of the in-memory cap in `remember()`.
pub fn trim_history(keep: usize) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "DELETE FROM history WHERE id NOT IN \
         (SELECT id FROM history ORDER BY id DESC LIMIT ?1)",
        params![i64::try_from(keep).unwrap_or(i64::MAX)],
    )
    .map_err(db)?;
    Ok(())
}

/// Every kept history entry, oldest first (the caller reverses for display, as `sql.history` does
/// over the in-memory vec).
pub fn load_history() -> Result<Vec<HistoryEntry>> {
    let conn = open()?;
    let mut stmt = conn
        .prepare("SELECT conn, sql, ok, elapsed_ms, rows, at FROM history ORDER BY id")
        .map_err(db)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HistoryEntry {
                conn: r.get(0)?,
                sql: r.get(1)?,
                ok: r.get::<_, i64>(2)? != 0,
                elapsed_ms: r.get(3)?,
                rows: r.get::<_, i64>(4)?.max(0) as usize,
                at: r.get::<_, i64>(5)?.max(0) as u64,
            })
        })
        .map_err(db)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db)?);
    }
    Ok(out)
}

/// Drop every history row.
pub fn clear_history() -> Result<()> {
    let conn = open()?;
    conn.execute("DELETE FROM history", []).map_err(db)?;
    Ok(())
}

// --- UI state (tab snapshots) and pinned connections -------------------------

/// Read one `ui_state` value by key.
pub fn load_ui_state(key: &str) -> Result<Option<String>> {
    let conn = open()?;
    let value = conn
        .query_row("SELECT value FROM ui_state WHERE key = ?1", params![key], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .map_err(db)?;
    Ok(value)
}

/// Write one `ui_state` value by key. One writer per fact: the key is the whole contract.
pub fn save_ui_state(key: &str, value: &str) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "INSERT INTO ui_state (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )
    .map_err(db)?;
    Ok(())
}

/// Pinned connection ids, first-painted first.
pub fn load_pins() -> Result<Vec<String>> {
    let conn = open()?;
    let mut stmt = conn
        .prepare("SELECT conn_id FROM pinned ORDER BY pin_order")
        .map_err(db)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db)?);
    }
    Ok(out)
}

/// Replace the pin set with `ids`, in the given order. Passing the current ids reorders.
pub fn save_pins(ids: &[String]) -> Result<()> {
    let conn = open()?;
    conn.execute("DELETE FROM pinned", []).map_err(db)?;
    for (order, id) in ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO pinned (conn_id, pin_order) VALUES (?1, ?2)",
            params![id, order as i64],
        )
        .map_err(db)?;
    }
    Ok(())
}

fn db(e: rusqlite::Error) -> Error {
    Error::config(format!("{}: {e}", crate::profiles::ai_db_path().display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AiConfig {
        AiConfig {
            provider: Provider::Kimi,
            model: "moonshot-v1-8k".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            max_tokens: 4096,
            temperature: 0.7,
            timeout_secs: 45,
        }
    }

    #[test]
    fn a_read_on_a_fresh_install_creates_nothing() {
        let _guard = crate::test_support::scratch_config();
        assert_eq!(load_ai().expect("empty store"), None, "nothing saved yet");
        assert!(
            !crate::profiles::ai_db_path().exists(),
            "a read must not create the file: opening the app adds nothing to {}/",
            crate::profiles::config_dir().display()
        );
    }

    #[test]
    fn a_configuration_round_trips_through_the_database() {
        let _guard = crate::test_support::scratch_config();
        assert_eq!(load_ai().expect("empty store"), None, "nothing saved yet");
        save_ai(&cfg()).expect("save");
        assert_eq!(load_ai().expect("load").as_ref(), Some(&cfg()));
    }

    #[test]
    fn a_second_save_replaces_the_row_rather_than_adding_one() {
        let _guard = crate::test_support::scratch_config();
        save_ai(&cfg()).expect("save");
        let mut changed = cfg();
        changed.provider = Provider::Ollama;
        changed.model = "llama3".into();
        save_ai(&changed).expect("save again");
        assert_eq!(load_ai().expect("load").as_ref(), Some(&changed));
        // The table itself refuses a second row, so "one row" is a property of the file.
        let conn = Connection::open(crate::profiles::ai_db_path()).expect("open");
        let extra = conn.execute(
            &format!(
                "INSERT INTO ai_config (id, {COLUMNS}) VALUES (2, 'ollama', '', '', 1, 0.0, 1)"
            ),
            [],
        );
        assert!(extra.is_err(), "a second row was allowed: {extra:?}");
    }

    #[test]
    fn the_database_is_readable_by_its_owner_only() {
        let _guard = crate::test_support::scratch_config();
        save_ai(&cfg()).expect("save");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(crate::profiles::ai_db_path())
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
        }
    }

    #[test]
    fn a_row_that_cannot_be_read_is_an_error_not_a_half_configuration() {
        let _guard = crate::test_support::scratch_config();
        save_ai(&cfg()).expect("save");
        // A hand-edited row: the column types are right, the provider is not one this build knows.
        let conn = Connection::open(crate::profiles::ai_db_path()).expect("open");
        conn.execute("UPDATE ai_config SET provider = 'opemai' WHERE id = 1", [])
            .expect("break the row");
        let error = load_ai().expect_err("an unknown provider is not a configuration");
        assert!(error.message().contains("opemai"), "{}", error.message());
    }

    #[test]
    fn a_file_that_lost_its_table_is_repaired_on_open() {
        let _guard = crate::test_support::scratch_config();
        // The file exists and has no schema at all — what an interrupted first write would leave.
        let path = crate::profiles::ai_db_path();
        Connection::open(&path).expect("create an empty database");
        save_ai(&cfg()).expect("save into a database with no table");
        assert_eq!(load_ai().expect("load").as_ref(), Some(&cfg()));
    }

    #[test]
    fn a_snippet_round_trips_through_the_database() {
        let _guard = crate::test_support::scratch_config();
        save_snippet("Count rows", "select count(*) from t", &["stats".into()])
            .expect("save");
        save_snippet("Top 100", "select * from t limit 100", &[]).expect("save");
        let first = load_snippets().expect("load");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].name, "Count rows");
        assert_eq!(first[0].sql, "select count(*) from t");
        assert_eq!(first[0].tags, vec!["stats".to_string()]);
        // An update to an existing name replaces the row, never adds one.
        save_snippet("Count rows", "select count(*) from t2", &[]).expect("resave");
        let second = load_snippets().expect("load");
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].sql, "select count(*) from t2");
    }

    #[test]
    fn a_snippet_survives_a_reopen_of_the_file() {
        let _guard = crate::test_support::scratch_config();
        save_snippet("x", "select 1", &[]).expect("save");
        // A second open of the database is the process-shaped boundary: the same code path a new
        // process takes, without forking in a unit test.
        let conn = open().expect("reopen");
        drop(conn);
        let loaded = load_snippets().expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "x");
    }

    #[test]
    fn delete_snippet_removes_the_row() {
        let _guard = crate::test_support::scratch_config();
        save_snippet("x", "select 1", &[]).expect("save");
        delete_snippet("x").expect("delete");
        assert!(load_snippets().expect("load").is_empty());
        // Deleting what is not there is a no-op, not an error: the caller is idempotent.
        delete_snippet("x").expect("delete again");
    }

    #[test]
    fn history_round_trips_and_trims_to_the_limit() {
        let _guard = crate::test_support::scratch_config();
        for i in 0..10 {
            push_history(&HistoryEntry {
                conn: "local".into(),
                sql: format!("select {i}"),
                ok: true,
                elapsed_ms: 1.0,
                rows: i,
                at: i as u64,
            })
            .expect("push");
        }
        trim_history(3).expect("trim");
        let kept = load_history().expect("load");
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[kept.len() - 1].sql, "select 9", "newest kept");
        assert_eq!(kept[0].sql, "select 7");
        clear_history().expect("clear");
        assert!(load_history().expect("load").is_empty());
    }

    #[test]
    fn ui_state_and_pins_round_trip() {
        let _guard = crate::test_support::scratch_config();
        assert_eq!(load_ui_state("tabs").expect("load"), None);
        save_ui_state("tabs", r#"{"tabs":[],"active":0}"#).expect("save");
        assert_eq!(
            load_ui_state("tabs").expect("load").as_deref(),
            Some(r#"{"tabs":[],"active":0}"#)
        );
        save_pins(&["stage".to_string(), "local".to_string()]).expect("save");
        assert_eq!(load_pins().expect("load"), vec!["stage".to_string(), "local".to_string()]);
        save_pins(&[]).expect("clear pins");
        assert!(load_pins().expect("load").is_empty());
    }

    #[test]
    fn user_version_migrates_a_file_that_has_only_ai_config() {
        let _guard = crate::test_support::scratch_config();
        // A file written by the pre-phase-1 build: the ai_config table, nothing else, user_version 0.
        let path = crate::profiles::ai_db_path();
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS ai_config (id INTEGER PRIMARY KEY CHECK (id = 1), \
                 provider TEXT NOT NULL, model TEXT NOT NULL, base_url TEXT NOT NULL, \
                 max_tokens INTEGER NOT NULL, temperature REAL NOT NULL, timeout_secs INTEGER NOT NULL)",
            )
            .expect("create the old schema");
        }
        assert!(load_snippets().expect("migrate on open").is_empty());
        let version: i64 = Connection::open(&path)
            .expect("open")
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("user_version");
        assert!(version >= 1, "user_version {version}");
    }
}
