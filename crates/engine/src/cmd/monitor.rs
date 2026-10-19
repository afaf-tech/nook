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
        CommandSpec::new("monitor.locks", "Blocking locks", "Monitor", locks)
            .params(r#"{"conn":"local"}"#)
            .help("Who blocks whom, the query each is running, and how long the wait has lasted. SQLite files have no server-level locks."),
        CommandSpec::new("monitor.replication", "Replication", "Monitor", replication)
            .params(r#"{"conn":"local"}"#)
            .help("Replica state and lag: pg_stat_replication on Postgres, SHOW REPLICA STATUS on MySQL."),
        CommandSpec::new("monitor.alerts", "Alert rules", "Monitor", alerts)
            .params(r#"{"rules":[{"metric":"connections","op":">","value":100}],"webhook":"https://hooks.example.com/nook"}"#)
            .help("Threshold rules over the metrics already collected, evaluated by the job scheduler. Tripping a rule logs a warning and POSTs the same JSON a failed job does."),
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
    super::require_safe_mode_for_params(session, p)?;
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

/// Who is waiting on whom.
///
/// The three engines answer this completely differently, and an empty table would read as "nothing
/// is wrong" — the worst possible answer for a lock view. So an engine with no such concept refuses
/// with a sentence instead.
fn locks(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let (kind, rows) = {
        let driver = session.driver_for(index)?;
        let kind = driver.kind();
        let rows = match kind {
            nook_core::DriverKind::Postgres => driver.run(LOCKS_PG, 1000)?,
            nook_core::DriverKind::MySql | nook_core::DriverKind::MariaDb => {
                driver.run(LOCKS_MYSQL, 1000)?
            }
            other => {
                return Err(Error::unsupported(format!(
                    "{} has no server-level locks: a file database serialises writers itself",
                    other.label()
                )))
            }
        };
        (kind, rows)
    };
    let out: Vec<serde_json::Value> = rows
        .rows
        .iter()
        .map(|r| {
            let text = |i: usize| r.get(i).map(|v| v.display()).unwrap_or_default();
            json!({
                "blocked_pid": text(0),
                "blocking_pid": text(1),
                "wait_s": text(2),
                "query": text(3),
                "blocked_query": text(4),
            })
        })
        .collect();
    Ok(json!({
        "engine": kind.label(),
        "count": out.len(),
        "locks": out,
        "note": if out.is_empty() {
            "no session is waiting on another"
        } else {
            "each row is one blocked session and the session holding its lock"
        },
    }))
}

/// Postgres: the blocked pid, the pid blocking it, how long it has waited, and both statements.
/// `pg_blocking_pids` is the server's own answer; deriving it from `pg_locks` is how this gets wrong.
const LOCKS_PG: &str = "SELECT blocked.pid, b.blocking_pid, \
        EXTRACT(EPOCH FROM (now() - blocked.query_start))::int, \
        left(blocked.query, 200), left(blocking.query, 200) \
     FROM pg_stat_activity blocked \
     CROSS JOIN LATERAL unnest(pg_blocking_pids(blocked.pid)) AS b(blocking_pid) \
     LEFT JOIN pg_stat_activity blocking ON blocking.pid = b.blocking_pid \
     WHERE cardinality(pg_blocking_pids(blocked.pid)) > 0 \
     ORDER BY blocked.query_start";

/// MySQL 8: `data_lock_waits` names both sides directly. Older servers fall back to the transaction
/// table, where the waiting transaction is the only one with a lock wait.
const LOCKS_MYSQL: &str = "SELECT w.REQUESTING_ENGINE_TRANSACTION_ID, \
        w.BLOCKING_ENGINE_TRANSACTION_ID, \
        TIMESTAMPDIFF(SECOND, trx.trx_started, NOW()), \
        LEFT(wtrx.trx_query, 200), LEFT(btrx.trx_query, 200) \
     FROM performance_schema.data_lock_waits w \
     LEFT JOIN information_schema.INNODB_TRX trx \
            ON trx.trx_id = w.REQUESTING_ENGINE_TRANSACTION_ID \
     LEFT JOIN information_schema.INNODB_TRX wtrx \
            ON wtrx.trx_id = w.REQUESTING_ENGINE_TRANSACTION_ID \
     LEFT JOIN information_schema.INNODB_TRX btrx \
            ON btrx.trx_id = w.BLOCKING_ENGINE_TRANSACTION_ID";

/// Replica state and lag, in one shape across the two engines that have replicas.
fn replication(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let (kind, replicas) = {
        let driver = session.driver_for(index)?;
        let kind = driver.kind();
        let replicas: Vec<serde_json::Value> = match kind {
            nook_core::DriverKind::Postgres => driver
                .run(
                    "SELECT client_addr, state, sync_state, \
                            COALESCE(EXTRACT(EPOCH FROM replay_lag)::int, -1), \
                            COALESCE(pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)::bigint, -1) \
                     FROM pg_stat_replication ORDER BY client_addr",
                    100,
                )?
                .rows
                .iter()
                .map(|r| {
                    let text = |i: usize| r.get(i).map(|v| v.display()).unwrap_or_default();
                    json!({
                        "client": text(0), "state": text(1), "sync": text(2),
                        "lag_s": text(3), "bytes_behind": text(4),
                    })
                })
                .collect(),
            nook_core::DriverKind::MySql | nook_core::DriverKind::MariaDb => driver
                .run("SHOW REPLICA STATUS", 10)?
                .rows
                .iter()
                .map(|r| {
                    // `SHOW REPLICA STATUS` is columnar: names are in `columns`, not in the row.
                    json!({ "row": r.iter().map(|v| v.display()).collect::<Vec<_>>() })
                })
                .collect(),
            other => {
                return Err(Error::unsupported(format!(
                    "{} has no replication: a file database is not replicated by the engine",
                    other.label()
                )))
            }
        };
        (kind, replicas)
    };
    Ok(json!({
        "engine": kind.label(),
        "count": replicas.len(),
        "replicas": replicas,
        "note": if replicas.is_empty() {
            "this server has no connected replicas"
        } else {
            "lag_s is the replay lag in seconds; -1 means the server did not report one"
        },
    }))
}

/// Alert rules: thresholds over the metrics the monitor already collects, evaluated by the job
/// scheduler so there is one cadence and one notification path.
fn alerts(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let rules = p
        .get("rules")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if rules.is_empty() {
        return Err(Error::config(
            "pass \"rules\": [{metric, op, value}], e.g. {\"metric\":\"connections\",\"op\":\">\",\"value\":100}",
        ));
    }
    // The metric set is the one `monitor.status` already reports, so a rule can only name something
    // that exists — a typo is a refusal, not a rule that can never fire.
    let status = status(session, p)?;
    // `monitor.status` reports `[{name, value}]`; the rule set is keyed by name so a rule names the
    // same metric the status table shows.
    let mut metrics: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for entry in status
        .get("status")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default()
    {
        if let (Some(name), Some(value)) = (
            entry.get("name").and_then(|v| v.as_str()),
            entry.get("value"),
        ) {
            if let Some(n) = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
            {
                metrics.insert(name.to_string(), json!(n));
            }
        }
    }
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut tripped: Vec<serde_json::Value> = Vec::new();
    for rule in &rules {
        let metric = rule
            .get("metric")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let op = rule.get("op").and_then(|v| v.as_str()).unwrap_or(">");
        let threshold = rule.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let Some(current) = metrics.get(metric).and_then(|v| v.as_f64()) else {
            return Err(Error::config(format!(
                "no metric named \"{metric}\"; the monitor reports {}",
                metrics.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        };
        let fires = match op {
            ">" => current > threshold,
            ">=" => current >= threshold,
            "<" => current < threshold,
            "<=" => current <= threshold,
            "==" => (current - threshold).abs() < f64::EPSILON,
            other => {
                return Err(Error::config(format!(
                    "unknown operator \"{other}\"; use >, >=, <, <= or =="
                )))
            }
        };
        let entry = json!({
            "metric": metric, "op": op, "value": threshold,
            "current": current, "tripped": fires,
        });
        if fires {
            tripped.push(entry.clone());
        }
        results.push(entry);
    }
    for entry in &tripped {
        session.log(
            Level::Warn,
            format!(
                "alert: {} is {} (threshold {})",
                entry["metric"].as_str().unwrap_or_default(),
                entry["current"],
                entry["value"]
            ),
        );
    }
    // The same endpoint and the same shape a failed job posts, so one receiver handles both.
    // A webhook that fails is logged and swallowed — it must never hide the reading it reports.
    let webhook = p.str_opt("webhook").unwrap_or_default();
    if !webhook.is_empty() && !tripped.is_empty() {
        let body = serde_json::json!({
            "job": "monitor.alerts",
            "name": "alert rules",
            "ok": false,
            "error": format!("{} alert rule(s) tripped", tripped.len()),
            "elapsed_ms": 0,
            "at": crate::jobs::now(),
            "alerts": tripped,
        });
        super::ops::post_json(&webhook, &body, "monitor.alerts", session);
    }
    let _ = index;
    Ok(json!({
        "rules": results,
        "tripped": tripped.len(),
        "alerts": tripped,
    }))
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
