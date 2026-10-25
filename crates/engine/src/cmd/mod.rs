//! The command surface. Each module owns one area of the app; every command is a function here,
//! reachable by id from the ribbon, the CLI, the control channel and MCP.

pub mod ai;
pub mod app;
pub mod conn;
pub mod diagram;
pub mod grid;
pub mod monitor;
pub mod ops;
pub mod privs;
pub mod schema;
pub mod sql;
pub mod tools;

use nook_core::{Error, Result, Value};
use nook_model::ResultSet;

use crate::{Conn, ConnState, Level, Params, Session, SshSecret, Tab, TableRef, Tunnel};

/// Public entry point for the CLI and MCP: open a connection by index.
pub fn open_now(session: &mut Session, index: usize) -> Result<()> {
    open_conn(session, index)
}

/// Safe mode's gate, at the moment of commit. Returns `Ok(())` when safe mode is off, and otherwise
/// only when the caller presented the token of the connection it is about to write to.
///
/// The connection is passed in rather than read from the params: `grid.apply` writes through its
/// *tab's* connection, and a gate that resolved `active_conn` instead would let a caller hold one
/// connection's token while the write landed in another database. Every other caller passes the
/// connection it is about to act on.
///
/// The gate is keyed on the setting alone, not on "a token exists for this connection": with the
/// latter, enabling safe mode while connected to A would leave B ungated and `apply` could not tell
/// "off" from "on, no token".
pub(crate) fn require_safe_mode(session: &Session, p: &Params, index: usize) -> Result<()> {
    if !session.settings.safe_mode {
        return Ok(());
    }
    let conn_id = session
        .conns
        .get(index)
        .map(|c| c.cfg.id.clone())
        .ok_or_else(|| Error::not_found(format!("no connection at index {index}")))?;
    let presented = p.str_opt("safe_token").unwrap_or_default();
    if crate::profiles::safe_token_matches(&conn_id, &presented) {
        Ok(())
    } else {
        Err(Error::config(format!(
            "safe mode: confirm {conn_id} first (editor.safeMode returns the token once the \
             connection test passes)"
        )))
    }
}

/// The same gate for the commands that resolve their own connection from the params. Kept separate
/// so a caller cannot forget which connection it is gating.
pub(crate) fn require_safe_mode_for_params(session: &Session, p: &Params) -> Result<()> {
    if !session.settings.safe_mode {
        return Ok(());
    }
    let index = match p.conn_id() {
        Some(id) => session.conn_index(&id)?,
        None => session
            .active_conn
            .ok_or_else(|| Error::config("safe mode: no connection is active to confirm"))?,
    };
    require_safe_mode(session, p, index)
}

/// The config a driver dials with: the profile's own, plus the password the keychain holds for it.
/// Both dial sites — `open_conn` here and `conn.test` in the editor — go through this, so a profile
/// whose password was saved cannot reach a driver without it.
pub(crate) fn dial_config(
    cfg: &nook_core::ConnectionConfig,
) -> Result<nook_core::ConnectionConfig> {
    let mut effective = cfg.clone();
    effective.password = crate::profiles::db_password(cfg)?;
    Ok(effective)
}

/// Open a connection: tunnel if asked, connect, read server info, fetch the catalog.
pub(crate) fn open_conn(session: &mut Session, index: usize) -> Result<()> {
    let cfg = session
        .conns
        .get(index)
        .ok_or_else(|| Error::not_found(format!("no connection at index {index}")))?
        .cfg
        .clone();

    if !cfg.kind.implemented() {
        let message = format!(
            "{} is in the catalog but has no driver yet",
            cfg.kind.label()
        );
        if let Some(conn) = session.conns.get_mut(index) {
            conn.state = ConnState::Failed;
            conn.error = Some(message.clone());
        }
        session.log(Level::Warn, message.clone());
        return Err(Error::unsupported(message));
    }

    let mut effective = dial_config(&cfg)?;
    let mut tunnel = None;
    if cfg.ssh.enabled {
        let t = Tunnel::start(&cfg, SshSecret::for_connection(&cfg.id))?;
        effective.ssh.local_port = t.local_port;
        session.log(
            Level::Info,
            format!("ssh tunnel {} → 127.0.0.1:{}", t.remote, t.local_port),
        );
        tunnel = Some(t);
    }

    let driver = match nook_drivers::open(&effective) {
        Ok(d) => d,
        Err(e) => {
            if let Some(conn) = session.conns.get_mut(index) {
                conn.state = ConnState::Failed;
                conn.error = Some(e.to_string());
            }
            return Err(e);
        }
    };
    let mut driver = driver;
    let server = driver.server_info()?;
    let started = std::time::Instant::now();
    let catalog = match driver.catalog() {
        Ok(c) => Some(c),
        Err(e) => {
            session.log(
                Level::Warn,
                format!("connected, but reading the catalog failed: {e}"),
            );
            None
        }
    };
    let ms = started.elapsed().as_millis() as u64;

    let conn: &mut Conn = session.conn_mut(index)?;
    conn.driver = Some(driver);
    conn.server = Some(server.clone());
    conn.catalog = catalog;
    conn.catalog_ms = ms;
    conn.tunnel = tunnel;
    conn.state = ConnState::Connected;
    conn.error = None;
    session.active_conn = Some(index);
    session.log(
        Level::Success,
        format!("connected to {} ({})", server.driver, server.version),
    );
    Ok(())
}

pub(crate) fn close_conn(session: &mut Session, index: usize) -> Result<()> {
    let name = {
        let conn = session.conn_mut(index)?;
        if let Some(driver) = conn.driver.as_mut() {
            driver.close();
        }
        conn.driver = None;
        conn.tunnel = None;
        conn.state = ConnState::Disconnected;
        conn.cfg.name.clone()
    };
    session.log(Level::Info, format!("closed {name}"));
    Ok(())
}

/// Re-read the catalog of an open connection.
pub fn refresh_catalog(session: &mut Session, index: usize) -> Result<u64> {
    let driver = session.driver_for(index)?;
    let started = std::time::Instant::now();
    let catalog = driver.catalog()?;
    let ms = started.elapsed().as_millis() as u64;
    let tables = catalog.table_count();
    let conn = session.conn_mut(index)?;
    conn.catalog = Some(catalog);
    conn.catalog_ms = ms;
    session.log(
        Level::Info,
        format!("catalog refreshed: {tables} objects in {ms} ms"),
    );
    Ok(ms)
}

/// Run SQL on a connection, bounding the row count to the session setting.
/// Run one statement with its placeholders bound by the server.
///
/// `values` are the bindings in placeholder order, already resolved: an explicit NULL is
/// `Value::Null`, and a caller that could not resolve a name never reaches here — `sql.run` refuses
/// first and names what is missing. A driver that cannot prepare refuses through the trait's default,
/// which is the point of the seam: there is no client-side substitution to fall back to.
pub fn run_prepared_sql(
    session: &mut Session,
    index: usize,
    sql: &str,
    values: &[Value],
    limit: Option<usize>,
) -> Result<ResultSet> {
    let max = limit.unwrap_or(session.settings.max_rows);
    let started = std::time::Instant::now();
    let rs = {
        let driver = session.driver_for(index)?;
        driver.run_prepared(sql, values, max)?
    };
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let mut rs = rs;
    rs.elapsed_ms = if rs.elapsed_ms > 0.0 {
        rs.elapsed_ms
    } else {
        elapsed
    };
    let conn_name = session
        .conns
        .get(index)
        .map(|c| c.cfg.name.clone())
        .unwrap_or_default();
    session.remember(crate::HistoryEntry {
        conn: conn_name,
        sql: sql.to_string(),
        ok: true,
        elapsed_ms: rs.elapsed_ms,
        rows: rs.row_count(),
        at: crate::jobs::now(),
    });
    Ok(rs)
}

pub fn run_sql(
    session: &mut Session,
    index: usize,
    sql: &str,
    limit: Option<usize>,
) -> Result<ResultSet> {
    let max = limit.unwrap_or(session.settings.max_rows);
    let started = std::time::Instant::now();
    let rs = {
        let driver = session.driver_for(index)?;
        driver.run_script(sql, max)?
    };
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let mut rs = rs;
    rs.elapsed_ms = if rs.elapsed_ms > 0.0 {
        rs.elapsed_ms
    } else {
        elapsed
    };
    let conn_name = session
        .conns
        .get(index)
        .map(|c| c.cfg.name.clone())
        .unwrap_or_default();
    session.remember(crate::HistoryEntry {
        conn: conn_name,
        sql: sql.to_string(),
        ok: true,
        elapsed_ms: rs.elapsed_ms,
        rows: rs.row_count(),
        at: crate::jobs::now(),
    });
    Ok(rs)
}

/// The one place the grid's SQL is written: the select behind one page of a table.
pub fn page_select(
    table: &TableRef,
    dialect: nook_core::Dialect,
    filter: &str,
    order: &str,
    size: usize,
    offset: usize,
) -> String {
    let mut sql = format!(
        "SELECT * FROM {}",
        nook_core::quote_qualified(table.schema.as_deref(), &table.table, dialect)
    );
    if !filter.trim().is_empty() {
        sql.push_str(&format!(" WHERE {}", filter.trim()));
    }
    if !order.trim().is_empty() {
        sql.push_str(&format!(" ORDER BY {}", order.trim()));
    }
    let size = size.clamp(1, 100_000);
    sql.push_str(&format!(" LIMIT {size} OFFSET {offset}"));
    sql
}

/// `SELECT` for the grid's current page.
pub fn grid_select(tab: &Tab) -> Result<(String, TableRef)> {
    let table = tab
        .grid
        .table
        .clone()
        .ok_or_else(|| Error::config("this data tab is not bound to a table"))?;
    let dialect = tab
        .payload
        .get("dialect")
        .and_then(|v| v.as_str())
        .and_then(parse_dialect)
        .unwrap_or(nook_core::Dialect::Sqlite);
    let size = tab.grid.page_size.clamp(1, 100_000);
    let offset = tab.grid.page.saturating_mul(size);
    let sql = page_select(
        &table,
        dialect,
        &tab.grid.filter,
        &tab.grid.order,
        size,
        offset,
    );
    Ok((sql, table))
}

/// The first rows of a table, for the object inspector's preview: the grid's own select without the
/// tab a grid normally needs, and without a history entry — a preview is not a query the user wrote.
///
/// ponytail: in-process only. The control channel has no preview verb because no agent has needed
/// one; add `grid.preview` to `cmd::grid` when one does.
pub fn preview_table(
    session: &mut Session,
    index: usize,
    table: &TableRef,
    limit: usize,
) -> Result<ResultSet> {
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref().map(|d| d.dialect()))
        .unwrap_or(nook_core::Dialect::Sqlite);
    let limit = limit.clamp(1, 500);
    let sql = page_select(table, dialect, "", "", limit, 0);
    let started = std::time::Instant::now();
    let mut result = session.driver_for(index)?.run_script(&sql, limit)?;
    if result.elapsed_ms <= 0.0 {
        result.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
    Ok(result)
}

pub fn parse_dialect(name: &str) -> Option<nook_core::Dialect> {
    match name {
        "sqlite" => Some(nook_core::Dialect::Sqlite),
        "postgres" | "postgresql" => Some(nook_core::Dialect::Postgres),
        "mysql" | "mariadb" => Some(nook_core::Dialect::MySql),
        _ => None,
    }
}

/// Reload the grid page of a data tab from the database.
pub fn load_grid(session: &mut Session, tab_index: usize) -> Result<()> {
    let conn_index = session
        .tabs
        .get(tab_index)
        .and_then(|t| t.conn)
        .ok_or_else(|| Error::config("this tab has no connection"))?;
    // Re-bind the table shape first: the key and columns come from the catalog, which a schema edit
    // (`schema.setPrimaryKey`, `schema.addColumn`, …) just changed behind this tab. Without the
    // re-bind a grid that became editable would stay read-only until the tab was reopened.
    if let Some(reference) = session.tabs[tab_index].grid.table.clone() {
        bind_grid_table(session, tab_index, reference)?;
    }
    // A query result that was made editable reloads by running its own statement again: the rows on
    // screen are that statement's, and replacing them with a `SELECT *` of the table would be a
    // different question answered in the same pane.
    if let Some(editable) = session.tabs[tab_index].sql.editable.clone() {
        let statement = session
            .tabs
            .get(tab_index)
            .and_then(|tab| tab.sql.statements.get(editable.statement))
            .cloned();
        if let Some(statement) = statement {
            let result = run_sql(session, conn_index, &statement, None);
            let tab = session
                .tabs
                .get_mut(tab_index)
                .ok_or_else(|| Error::not_found("tab is gone"))?;
            match result {
                Ok(rs) => {
                    tab.grid.result = rs;
                    tab.grid.error = None;
                    tab.grid.elapsed_ms = tab.grid.result.elapsed_ms;
                }
                Err(e) => {
                    tab.grid.error = Some(e.to_string());
                    tab.grid.result = ResultSet::default();
                }
            }
            return Ok(());
        }
    }
    let (sql, table) = grid_select(
        session
            .tabs
            .get(tab_index)
            .ok_or_else(|| Error::not_found("tab is gone"))?,
    )?;
    let result = run_sql(session, conn_index, &sql, None);
    let conn_index_again = conn_index;
    let tab = session
        .tabs
        .get_mut(tab_index)
        .ok_or_else(|| Error::not_found("tab is gone"))?;
    match result {
        Ok(rs) => {
            tab.grid.result = rs;
            tab.grid.error = None;
            tab.grid.table = Some(table);
            tab.grid.elapsed_ms = tab.grid.result.elapsed_ms;
        }
        Err(e) => {
            tab.grid.error = Some(e.to_string());
            tab.grid.result = ResultSet::default();
        }
    }
    let _ = conn_index_again;
    Ok(())
}

/// Bring a data tab back in step with the catalog before its rows are edited.
///
/// The binding is taken when the tab opens — `GridState::columns`, `GridState::primary_key` and
/// the page beside them describe the table as it was then. A schema edit made since, in the designer,
/// the SQL editor or the CLI, leaves the tab planning against columns that are gone, and a stale
/// `WHERE` is worse than an error: SQLite reads a double-quoted name that matches no column as a
/// string literal, so the statement matched no row while the grid reported it applied.
///
/// Every schema path in this process refreshes the catalog after it writes — `schema.*` here, and
/// `sql.run` for a script that may have changed the shape — so the catalog is what this reads. A
/// change made by another process is not seen until the next refresh, which is what `grid.apply`'s
/// own check covers: it refuses an instruction that changed no row rather than report a no-op.
///
/// ponytail: the page is fetched again only when it no longer describes the table, so a schema change
/// costs one round trip and a stable schema costs a catalog lookup. Reloading smartly (keeping the
/// scroll position, say) waits for a user who notices the jump.
pub(crate) fn sync_grid_table(session: &mut Session, tab_index: usize) -> Result<()> {
    let reference = session
        .tabs
        .get(tab_index)
        .and_then(|tab| tab.grid.table.clone())
        .ok_or_else(|| Error::config("this data tab is not bound to a table"))?;
    // Re-bind first — a schema edit since the tab was bound must be seen even when the page still
    // describes the table. `stale_page` is then judged against the freshly bound columns: a page
    // whose columns no longer exist must reload, and its `load_grid` re-binds before it selects.
    bind_grid_table(session, tab_index, reference)?;
    // A query-sourced grid's projection is the statement's own (`select id from users`), so it is
    // legitimately narrower than the table and would look stale on every edit. Worse, the reload
    // that follows re-runs the statement: an unordered one may return its rows in another order, so
    // the row index resolved after the reload can name a different record than the cell the user
    // edited. The statement's rows are the page; never judge them against the table's shape.
    if session.tabs[tab_index].sql.editable.is_some() {
        return Ok(());
    }
    let stale_page = {
        let tab = session
            .tabs
            .get(tab_index)
            .ok_or_else(|| Error::not_found("tab is gone"))?;
        let current: Vec<String> = tab
            .table
            .as_ref()
            .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default();
        !current.is_empty() && current != tab.grid.result.columns
    };
    if stale_page {
        load_grid(session, tab_index)?;
    }
    Ok(())
}

/// Attach a table's key information to a grid so edits can be planned.
pub fn bind_grid_table(session: &mut Session, tab_index: usize, table: TableRef) -> Result<()> {
    let conn_index = session
        .tabs
        .get(tab_index)
        .and_then(|t| t.conn)
        .ok_or_else(|| Error::config("this tab has no connection"))?;
    let info = session
        .conns
        .get(conn_index)
        .and_then(|c| c.catalog.as_ref())
        .and_then(|cat| cat.find_table(table.schema.as_deref(), &table.table))
        .cloned();
    let dialect = session
        .conns
        .get(conn_index)
        .and_then(|c| c.driver.as_ref().map(|d| d.dialect()))
        .unwrap_or(nook_core::Dialect::Sqlite);
    let tab = session
        .tabs
        .get_mut(tab_index)
        .ok_or_else(|| Error::not_found("tab is gone"))?;
    tab.grid.table = Some(table.clone());
    // The hidden list is a stored preference: read it once here, so the paint path reads a field
    // rather than the store, and a reopened table comes back with the same columns hidden.
    let conn_id = session
        .conns
        .get(conn_index)
        .map(|c| c.cfg.id.clone())
        .unwrap_or_default();
    tab.grid.hidden = hidden_columns(&conn_id, &table).unwrap_or_default();
    // Widths are re-sent whole on every `ResizeEnd`, and `grid.columnWidths` rejects a name the
    // table does not have. A column dropped since the width was stored would therefore poison the
    // cache and make every later drag fail to save, so stale names are filtered out here — once,
    // at the only place the cache is filled.
    tab.grid.widths = column_widths(&conn_id, &table)
        .unwrap_or_default()
        .into_iter()
        .filter(|(name, _)| info.as_ref().is_some_and(|t| t.column(name).is_some()))
        .collect();
    tab.payload
        .insert("dialect".into(), serde_json::json!(dialect_name(dialect)));
    match info {
        Some(t) => {
            tab.grid.columns = t.columns.clone();
            tab.grid.primary_key = t.primary_key();
            tab.table = Some(t);
        }
        None => {
            tab.grid.columns.clear();
            tab.grid.primary_key.clear();
            tab.table = None;
        }
    }
    Ok(())
}

/// The columns this table's view hides, as stored under `hidden:<conn>:<table>`. One reader, so the
/// key is built in exactly one place.
pub fn hidden_columns(conn_id: &str, table: &TableRef) -> Result<Vec<String>> {
    let key = format!("hidden:{conn_id}:{}", table.key());
    let stored = crate::store::load_ui_state(&key)?;
    Ok(stored
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

/// The column width overrides this table's view applies, as stored under `widths:<conn>:<table>`.
/// One reader, so the key is built in exactly one place.
pub fn column_widths(conn_id: &str, table: &TableRef) -> Result<Vec<(String, f32)>> {
    let key = format!("widths:{conn_id}:{}", table.key());
    let stored = crate::store::load_ui_state(&key)?;
    Ok(stored
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
            obj.into_iter()
                .filter_map(|(k, v)| v.as_f64().map(|w| (k, w as f32)))
                .collect()
        })
        .unwrap_or_default())
}

pub(crate) fn dialect_name(d: nook_core::Dialect) -> &'static str {
    match d {
        nook_core::Dialect::Sqlite => "sqlite",
        nook_core::Dialect::Postgres => "postgres",
        nook_core::Dialect::MySql => "mysql",
    }
}

/// The row a key belongs to on this page, or `None` when the page does not hold that record.
///
/// A page moves under a pending edit — a sort, a filter, a page turn — so the row an edit was made on
/// cannot say where its record is now. Its key can. Everything that has to point at a pending edit on
/// the page must go through here rather than through `PendingEdit::row`, which is only the record's
/// place at the time of the edit.
pub fn row_of_key(
    result: &ResultSet,
    primary_key: &[String],
    key: &[nook_core::Value],
) -> Option<usize> {
    // A length mismatch is a key set that changed since the edit was made; there is nothing to match.
    if key.is_empty() || primary_key.len() != key.len() {
        return None;
    }
    // The columns are resolved once, not through `value_at` per row: the table's dirty-cell highlight
    // runs this per pending edit on every frame, against a page the picker allows up to 5000 rows.
    let indices: Option<Vec<usize>> = primary_key
        .iter()
        .map(|name| result.columns.iter().position(|column| column == name))
        .collect();
    let indices = indices?;
    (0..result.rows.len()).find(|row| {
        indices
            .iter()
            .zip(key)
            .all(|(index, expected)| result.rows[*row].get(*index) == Some(expected))
    })
}

/// Primary-key values for a grid row, used when planning an edit.
pub(crate) fn pk_values(tab: &Tab, row: usize) -> Result<Vec<(String, nook_core::Value)>> {
    let mut out = Vec::new();
    for column in &tab.grid.primary_key {
        let value = tab
            .grid
            .result
            .value_at(row, column)
            .cloned()
            .ok_or_else(|| Error::internal(format!("row {row} has no value for key {column}")))?;
        out.push((column.clone(), value));
    }
    if out.is_empty() {
        return Err(Error::unsupported(format!(
            "{} has no primary key; Nook will not guess which row to change",
            tab.grid
                .table
                .as_ref()
                .map(TableRef::key)
                .unwrap_or_else(|| "this table".into())
        )));
    }
    Ok(out)
}

/// Where a command should read its SQL from: the `sql` parameter, else the active tab's editor.
pub(crate) fn sql_from_params_or_editor(
    session: &Session,
    params: &crate::Params,
) -> Result<String> {
    if let Some(sql) = params.str_opt("sql") {
        if !sql.trim().is_empty() {
            return Ok(sql);
        }
    }
    let tab = session.active_tab()?;
    if tab.sql.text.trim().is_empty() {
        return Err(Error::config(
            "nothing to run: pass \"sql\" or type into the editor first",
        ));
    }
    Ok(tab.sql.text.clone())
}
