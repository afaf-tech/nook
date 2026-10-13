//! Server monitor: sessions, variables, status and the per-table dashboard.

use nook_core::{Error, Result, Value};
use serde_json::json;

use crate::{CommandSpec, Level, Params, Session};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("monitor.processes", "Sessions", "Monitor", processes)
            .params(r#"{"conn":"local"}"#)
            .help("The server's process list. SQLite files have none."),
        CommandSpec::new("monitor.kill", "Kill a session", "Monitor", kill)
            .params(r#"{"conn":"local","id":"1234","confirm":true}"#),
        CommandSpec::new("monitor.variables", "Server variables", "Monitor", variables)
            .params(r#"{"filter":"timeout"}"#),
        CommandSpec::new("monitor.status", "Server status", "Monitor", status),
        CommandSpec::new("monitor.dashboard", "Dashboard", "Monitor", dashboard)
            .help("Biggest tables, row estimates and sizes — the numbers Navicat shows on its monitor tab."),
    ]
}

fn processes(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let rows = {
        let driver = session.driver_for(index)?;
        driver.processes()?
    };
    Ok(json!({
        "count": rows.len(),
        "sessions": rows.iter().map(|s| json!({
            "id": s.id, "user": s.user, "database": s.database, "host": s.host,
            "state": s.state, "query": s.query, "elapsed_ms": s.elapsed_ms, "started": s.started,
        })).collect::<Vec<_>>(),
    }))
}

fn kill(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    if session.settings.confirm_destructive && !p.bool_opt("confirm", false) {
        return Err(Error::config(
            "refusing to kill a session without \"confirm\": true",
        ));
    }
    let index = session.resolve_conn(p)?;
    let id = p.str_arg("id")?;
    {
        let driver = session.driver_for(index)?;
        driver.kill_process(&id)?;
    }
    session.log(Level::Warn, format!("session {id} terminated"));
    Ok(json!({ "killed": id }))
}

fn variables(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let filter = p.str_opt("filter").map(|f| f.to_ascii_lowercase());
    let rows = {
        let driver = session.driver_for(index)?;
        driver.variables()?
    };
    let filtered: Vec<serde_json::Value> = rows
        .iter()
        .filter(|(name, _)| {
            filter
                .as_ref()
                .map(|f| name.to_ascii_lowercase().contains(f))
                .unwrap_or(true)
        })
        .take(2000)
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect();
    Ok(json!({ "count": filtered.len(), "total": rows.len(), "variables": filtered }))
}

fn status(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let rows = {
        let driver = session.driver_for(index)?;
        driver.status()?
    };
    Ok(json!({
        "count": rows.len(),
        "status": rows.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
    }))
}

fn dashboard(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let limit = p.int_opt("limit", 25).clamp(1, 500) as usize;
    let (mut tables, server) = {
        let conn = session
            .conns
            .get(index)
            .ok_or_else(|| Error::not_found("no such connection"))?;
        let server = conn.server.clone();
        let mut tables: Vec<(String, Option<i64>, Option<i64>)> = Vec::new();
        if let Some(catalog) = &conn.catalog {
            for schema in &catalog.schemas {
                for table in &schema.tables {
                    tables.push((table.key(), table.row_estimate, table.size_bytes));
                }
            }
        }
        (tables, server)
    };
    tables.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));
    let biggest: Vec<serde_json::Value> = tables
        .iter()
        .take(limit)
        .map(|(name, rows, size)| {
            json!({
                "table": name, "rows": rows, "size_bytes": size,
                "size": size.map(human_bytes),
            })
        })
        .collect();

    // A small numeric table for the UI's bar chart: size in kilobytes per table.
    let chart = nook_model::ResultSet {
        columns: vec!["table".into(), "kilobytes".into()],
        column_types: vec!["text".into(), "numeric".into()],
        rows: tables
            .iter()
            .filter_map(|(name, _, size)| {
                size.map(|s| vec![Value::text(name.clone()), Value::Float(s as f64 / 1024.0)])
            })
            .take(limit)
            .collect(),
        ..nook_model::ResultSet::default()
    };

    Ok(json!({
        "server": server.as_ref().map(|s| json!({
            "version": s.version, "uptime_s": s.uptime_s, "size_bytes": s.size_bytes,
            "size": s.size_bytes.map(human_bytes),
        })),
        "tables": biggest,
        "table_count": tables.len(),
        "chart": chart.to_json(),
    }))
}

fn human_bytes(bytes: i64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitoring_needs_an_open_connection() {
        let mut session = Session::new();
        assert!(processes(&mut session, &Params::none()).is_err());
        assert!(status(&mut session, &Params::none()).is_err());
        assert!(variables(&mut session, &Params::none()).is_err());
        assert!(dashboard(&mut session, &Params::none()).is_err());
    }

    #[test]
    fn killing_needs_confirmation() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"id": "1"})).expect("params");
        let e = kill(&mut session, &p).expect_err("needs confirm");
        assert!(e.message().contains("confirm"));
    }

    #[test]
    fn byte_formatting_reads_naturally() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
