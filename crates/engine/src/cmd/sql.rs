//! Query editor commands: run, format, complete, explain, history, transactions.

use datacraft_core::{Error, Result};
use serde_json::json;

use crate::cmd::{run_sql, sql_from_params_or_editor};
use crate::sqltext::{self, Completion};
use crate::{CommandSpec, Level, Params, Session, TabKind};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("sql.newQuery", "New query", "Query", new_query)
            .params(r#"{"sql":"select 1"}"#)
            .shortcut("cmd+n"),
        CommandSpec::new("sql.run", "Run", "Query", run)
            .params(r#"{"sql":"select * from users","limit":500}"#)
            .shortcut("cmd+enter")
            .help("Runs every statement in the script. Without \"sql\" the editor text is used."),
        CommandSpec::new(
            "sql.runStatement",
            "Run statement at cursor",
            "Query",
            run_statement,
        )
        .params(r#"{"index":0}"#),
        CommandSpec::new(
            "sql.runSelection",
            "Run the selection",
            "Query",
            run_selection,
        )
        .params(r#"{"start":0,"end":12}"#),
        CommandSpec::new("sql.statements", "List statements", "Query", statements),
        CommandSpec::new("sql.format", "Format SQL", "Query", format_sql).shortcut("cmd+shift+f"),
        CommandSpec::new("sql.complete", "Complete the word", "Query", complete)
            .params(r#"{"prefix":"sel","sql":"SELECT * FROM users WHERE "}"#),
        CommandSpec::new("sql.explain", "Explain", "Query", explain)
            .params(r#"{"sql":"select * from users","analyze":false}"#),
        CommandSpec::new("sql.history", "Query history", "Query", history)
            .params(r#"{"limit":20}"#),
        CommandSpec::new("sql.clearHistory", "Clear history", "Query", clear_history),
        CommandSpec::new(
            "sql.transaction",
            "Transaction control",
            "Query",
            transaction,
        )
        .params(r#"{"action":"begin"}"#),
        CommandSpec::new(
            "sql.runInTransaction",
            "Run atomically",
            "Query",
            run_in_transaction,
        )
        .params(r#"{"sql":"update …; delete …;"}"#)
        .help("BEGIN, run every statement, COMMIT — or ROLLBACK at the first failure."),
        CommandSpec::new("sql.stop", "Stop", "Query", stop).help(
            "Cancelling a running statement is not possible yet; each run is bounded by max_rows.",
        ),
    ]
}

fn new_query(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = match p.conn_id() {
        Some(id) => Some(session.conn_index(&id)?),
        None => session.active_conn,
    };
    let index = session.add_tab(TabKind::Query, "Query", conn);
    if let Some(sql) = p.str_opt("sql") {
        if let Some(tab) = session.tabs.get_mut(index) {
            tab.sql.text = sql;
        }
    }
    Ok(json!({ "tab": index, "title": session.tabs[index].title }))
}

fn store_result(
    session: &mut Session,
    result: &datacraft_model::ResultSet,
    statements: Vec<String>,
) {
    if let Ok(tab) = session.active_tab_mut() {
        if tab.kind == TabKind::Query {
            tab.sql.last_result = Some(result.clone());
            tab.sql.error = None;
            tab.sql.elapsed_ms = result.elapsed_ms;
            tab.sql.statements = statements;
            tab.sql.result_label = format!(
                "{} rows · {} ms{}",
                result.row_count(),
                result.elapsed_ms.round(),
                if result.truncated {
                    " · truncated"
                } else {
                    ""
                }
            );
        }
    }
}

fn summarize(result: &datacraft_model::ResultSet, statements: &[String]) -> serde_json::Value {
    json!({
        "columns": result.columns,
        "types": result.column_types,
        "rows": result.row_count(),
        "affected": result.affected,
        "last_insert_id": result.last_insert_id,
        "elapsed_ms": result.elapsed_ms,
        "truncated": result.truncated,
        "statements": statements.len(),
        // Arrays, so a CLI or an agent can print them positionally; "objects" keeps the names.
        "data": result.rows.iter().map(|row| {
            serde_json::Value::Array(row.iter().map(|v| v.to_json()).collect())
        }).collect::<Vec<_>>(),
        "objects": result.to_json()["rows"],
    })
}

fn run(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let sql = sql_from_params_or_editor(session, p)?;
    let limit = match p.int_opt("limit", 0) {
        n if n > 0 => Some(n.clamp(1, 5_000_000) as usize),
        _ => None,
    };
    let statements = crate::cmd::sql::split(&sql);
    let result = run_sql(session, conn, &sql, limit)?;
    store_result(session, &result, statements.clone());
    session.log(
        Level::Info,
        format!(
            "{} statement(s) · {} rows · {} ms",
            statements.len(),
            result.row_count(),
            result.elapsed_ms.round()
        ),
    );
    Ok(summarize(&result, &statements))
}

fn run_statement(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let sql = sql_from_params_or_editor(session, p)?;
    let statements = split(&sql);
    if statements.is_empty() {
        return Err(Error::config("there is no statement to run"));
    }
    let index = p.int_opt("index", 0);
    let picked = statements
        .get(index.max(0) as usize)
        .ok_or_else(|| {
            Error::not_found(format!(
                "no statement {index}; the editor has {}",
                statements.len()
            ))
        })?
        .clone();
    let conn = session.resolve_conn(p)?;
    let result = run_sql(session, conn, &picked, None)?;
    store_result(session, &result, vec![picked.clone()]);
    Ok(json!({
        "statement": picked,
        "rows": result.row_count(),
        "affected": result.affected,
        "elapsed_ms": result.elapsed_ms,
        "data": result.to_json()["rows"],
    }))
}

fn run_selection(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let sql = sql_from_params_or_editor(session, p)?;
    let start = p.int_opt("start", 0).max(0) as usize;
    let end = p.int_opt("end", sql.len() as i64).max(start as i64) as usize;
    let statement = sqltext::statement_at(&sql, start)
        .ok_or_else(|| Error::config("no statement under the selection"))?;
    let _ = end;
    let params = p.with("sql", json!(statement));
    run(session, &params)
}

fn statements(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let sql = sql_from_params_or_editor(session, p)?;
    let list = split(&sql);
    Ok(json!({ "count": list.len(), "statements": list }))
}

fn format_sql(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let sql = sql_from_params_or_editor(session, p)?;
    let case = session.settings.keyword_case;
    let formatted = sqltext::format(&sql, case, 2);
    if p.bool_opt("replace", true) {
        if let Ok(tab) = session.active_tab_mut() {
            tab.sql.text = formatted.clone();
        }
    }
    Ok(json!({ "sql": formatted, "lines": formatted.lines().count() }))
}

fn complete(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let prefix = p.str_opt("prefix").unwrap_or_default();
    let statement = p
        .str_opt("sql")
        .or_else(|| session.active_tab().ok().map(|t| t.sql.text.clone()))
        .unwrap_or_default();
    let limit = p.int_opt("limit", 50).clamp(1, 500) as usize;
    let dialect = session
        .active_conn
        .and_then(|i| session.conns.get(i))
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .unwrap_or(datacraft_core::Dialect::Sqlite);
    let catalog = session
        .active_conn
        .and_then(|i| session.conns.get(i))
        .and_then(|c| c.catalog.as_ref());
    let items = sqltext::complete(&prefix, &statement, catalog, dialect, limit);
    Ok(json!({
        "prefix": prefix,
        "candidates": items.iter().map(|c: &Completion| json!({
            "text": c.text, "kind": c.kind.label(), "detail": c.detail,
        })).collect::<Vec<_>>()
    }))
}

fn explain(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let sql = sql_from_params_or_editor(session, p)?;
    let analyze = p.bool_opt("analyze", false);
    let conn = session.resolve_conn(p)?;
    let statement = split(&sql)
        .first()
        .cloned()
        .ok_or_else(|| Error::config("there is nothing to explain"))?;
    let plan = {
        let driver = session.driver_for(conn)?;
        if analyze {
            driver.run(&format!("EXPLAIN ANALYZE {statement}"), 500)?
        } else {
            driver.explain(&statement)?
        }
    };
    Ok(json!({
        "statement": statement,
        "columns": plan.columns,
        "plan": plan.rows.iter().map(|r| r.iter().map(|v| v.display()).collect::<Vec<_>>()).collect::<Vec<_>>(),
    }))
}

fn history(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let limit = p.int_opt("limit", 20).clamp(1, 1000) as usize;
    let filter = p.conn_id();
    let entries: Vec<serde_json::Value> = session
        .history
        .iter()
        .rev()
        .filter(|h| filter.as_deref().map(|f| h.conn == f).unwrap_or(true))
        .take(limit)
        .map(|h| {
            json!({
                "conn": h.conn, "sql": h.sql, "ok": h.ok,
                "elapsed_ms": h.elapsed_ms, "rows": h.rows, "at": h.at,
            })
        })
        .collect();
    Ok(json!({ "history": entries, "total": session.history.len() }))
}

fn clear_history(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let n = session.history.len();
    session.history.clear();
    Ok(json!({ "cleared": n }))
}

fn transaction(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let action = p.str_arg("action")?.to_ascii_lowercase();
    let driver = session.driver_for(conn)?;
    match action.as_str() {
        "begin" | "start" => driver.begin()?,
        "commit" => driver.commit()?,
        "rollback" => driver.rollback()?,
        other => {
            return Err(Error::config(format!(
                "unknown transaction action '{other}'; try begin, commit or rollback"
            )))
        }
    }
    let open = driver.in_transaction();
    Ok(json!({ "action": action, "in_transaction": open }))
}

fn run_in_transaction(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let sql = sql_from_params_or_editor(session, p)?;
    let statements = split(&sql);
    if statements.is_empty() {
        return Err(Error::config("nothing to run"));
    }
    session.driver_for(conn)?.begin()?;
    let mut applied = Vec::new();
    let mut last = datacraft_model::ResultSet::default();
    for statement in &statements {
        match run_sql(session, conn, statement, None) {
            Ok(rs) => {
                last = rs;
                applied.push(statement.clone());
            }
            Err(e) => {
                let _ = session.driver_for(conn).and_then(|d| d.rollback());
                session.log(
                    Level::Error,
                    format!(
                        "rolled back after {} of {} statements",
                        applied.len(),
                        statements.len()
                    ),
                );
                return Err(e);
            }
        }
    }
    session.driver_for(conn)?.commit()?;
    session.log(
        Level::Success,
        format!("{} statements committed", applied.len()),
    );
    Ok(json!({
        "committed": applied.len(),
        "rows": last.row_count(),
        "affected": last.affected,
    }))
}

fn stop(_session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    Err(Error::unsupported(
        "statement cancellation is not implemented; every run is bounded by the max_rows preference",
    ))
}

/// Split a script into statements, keeping the trailing semicolons.
pub fn split(sql: &str) -> Vec<String> {
    datacraft_drivers::split_script(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_matches_the_driver_splitter() {
        let parts = split("select 1; select 2;\n-- c;\nselect 3");
        assert_eq!(parts.len(), 3);
        assert!(parts[2].contains("select 3"));
    }

    #[test]
    fn formatting_without_sql_reports_a_clear_error() {
        let mut session = Session::new();
        let e = format_sql(&mut session, &Params::none());
        assert!(e.is_err());
    }

    #[test]
    fn history_reads_back_and_clears() {
        let mut session = Session::new();
        session.remember(crate::HistoryEntry {
            conn: "local".into(),
            sql: "select 1".into(),
            ok: true,
            elapsed_ms: 1.0,
            rows: 1,
            at: 5,
        });
        let out = history(&mut session, &Params::none()).expect("history");
        assert_eq!(out["history"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            clear_history(&mut session, &Params::none()).expect("clear")["cleared"],
            1
        );
    }

    #[test]
    fn running_without_a_connection_or_sql_is_an_error() {
        let mut session = Session::new();
        assert!(run(&mut session, &Params::none()).is_err());
        let p = Params::from_json(json!({"sql": "select 1"})).expect("params");
        let e = run(&mut session, &p);
        assert!(e.is_err());
        assert!(e
            .expect_err("no connection")
            .message()
            .contains("connection"));
    }

    #[test]
    fn explain_without_sql_is_an_error() {
        let mut session = Session::new();
        assert!(explain(&mut session, &Params::none()).is_err());
    }

    #[test]
    fn completion_works_without_a_catalog() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"prefix": "sel"})).expect("params");
        let out = complete(&mut session, &p).expect("complete");
        assert!(out["candidates"].as_array().map(Vec::len).unwrap_or(0) > 0);
    }
}
