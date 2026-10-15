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
)";

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
}
