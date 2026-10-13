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

use nook_core::{Error, Result};
use nook_model::ResultSet;

use crate::{Conn, ConnState, Level, Session, SshSecret, Tab, TableRef, Tunnel};

/// Public entry point for the CLI and MCP: open a connection by index.
pub fn open_now(session: &mut Session, index: usize) -> Result<()> {
    open_conn(session, index)
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

pub(crate) fn dialect_name(d: nook_core::Dialect) -> &'static str {
    match d {
        nook_core::Dialect::Sqlite => "sqlite",
        nook_core::Dialect::Postgres => "postgres",
        nook_core::Dialect::MySql => "mysql",
    }
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
