//! Query editor commands: run, format, complete, explain, history, transactions.

use datacraft_core::{Error, Result};
use datacraft_model::ResultSet;
use serde_json::json;

use crate::cmd::{run_sql, sql_from_params_or_editor};
use crate::sqltext::{self, Completion};
use crate::{CommandSpec, Level, Params, Session, StatementRun, TabKind};

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

/// Open the tab's SQL state for a run: the previous run's results and the facts describing them go,
/// because a run replaces the register rather than appending to it.
fn begin_run(session: &mut Session, statements: Vec<String>) {
    if let Ok(tab) = session.active_tab_mut() {
        if tab.kind == TabKind::Query {
            tab.sql.results.clear();
            tab.sql.statements = statements;
            tab.sql.last_result = None;
            tab.sql.result_label.clear();
            tab.sql.elapsed_ms = 0.0;
        }
    }
}

/// One statement's result folded into the run's aggregate, the rule `Driver::run_script` uses: the
/// latest tabular result wins and a statement with no columns adds its affected count to it.
fn fold(last: &mut ResultSet, rs: &ResultSet) {
    if rs.is_tabular() || last.columns.is_empty() {
        let mut notices = std::mem::take(&mut last.notices);
        notices.extend(rs.notices.iter().cloned());
        *last = rs.clone();
        last.notices = notices;
    } else {
        last.affected += rs.affected;
        last.notices.extend(rs.notices.iter().cloned());
    }
}

/// The single writer of a tab's SQL state: appends the statement's run, and keeps `last_result`,
/// `result_label` and `elapsed_ms` describing the run so far — the aggregate the status bar and the
/// grid read, built by `fold`. A run with no result (the failure that stopped the run) changes only
/// `results`.
fn record_run(session: &mut Session, run: StatementRun, run_so_far: &mut ResultSet) {
    if let Some(result) = run.result.as_ref() {
        fold(run_so_far, result);
    }
    if let Ok(tab) = session.active_tab_mut() {
        if tab.kind == TabKind::Query {
            if run.result.is_some() {
                tab.sql.last_result = Some(run_so_far.clone());
                tab.sql.elapsed_ms = run_so_far.elapsed_ms;
                tab.sql.result_label = format!(
                    "{} rows · {} ms{}",
                    run_so_far.row_count(),
                    run_so_far.elapsed_ms.round(),
                    if run_so_far.truncated {
                        " · truncated"
                    } else {
                        ""
                    }
                );
            }
            tab.sql.results.push(run);
        }
    }
}

/// One statement's line of the run's summary: what it was, what it produced, how long it took, and
/// the driver's own words when it failed. Counts only, so the run never has to be cloned to
/// summarise it.
fn statement_summary(
    index: usize,
    statement: &str,
    result: Option<&ResultSet>,
    elapsed_ms: f64,
    error: Option<String>,
) -> serde_json::Value {
    json!({
        "index": index,
        "statement": statement,
        "rows": result.map(ResultSet::row_count).unwrap_or(0),
        "affected": result.map(|r| r.affected).unwrap_or(0),
        "elapsed_ms": elapsed_ms,
        "error": error,
    })
}

/// Run one statement of the script, record it and return its line of the run's summary. `Err` means
/// the statement failed: the run is already recorded, with `result: None` and the message, so the
/// caller only has to return the error.
fn run_and_record(
    session: &mut Session,
    conn: usize,
    index: usize,
    statement: &str,
    limit: Option<usize>,
    run_so_far: &mut ResultSet,
) -> Result<serde_json::Value> {
    let started = std::time::Instant::now();
    let outcome = run_sql(session, conn, statement, limit);
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    match outcome {
        Ok(result) => {
            let elapsed_ms = result.elapsed_ms;
            let summary = statement_summary(index, statement, Some(&result), elapsed_ms, None);
            record_run(
                session,
                StatementRun {
                    statement: statement.to_string(),
                    result: Some(result),
                    elapsed_ms,
                    error: None,
                },
                run_so_far,
            );
            Ok(summary)
        }
        Err(e) => {
            record_run(
                session,
                StatementRun {
                    statement: statement.to_string(),
                    result: None,
                    elapsed_ms: wall_ms,
                    error: Some(e.to_string()),
                },
                run_so_far,
            );
            Err(e)
        }
    }
}

fn summarize(
    result: &ResultSet,
    statements: &[String],
    results: Vec<serde_json::Value>,
) -> serde_json::Value {
    json!({
        "columns": result.columns,
        "types": result.column_types,
        "rows": result.row_count(),
        "affected": result.affected,
        "last_insert_id": result.last_insert_id,
        "elapsed_ms": result.elapsed_ms,
        "truncated": result.truncated,
        "statements": statements.len(),
        // One entry per statement of this run, in run order; the failed one carries the message.
        "results": results,
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
    let statements = split(&sql);
    begin_run(session, statements.clone());
    let mut run_so_far = ResultSet::default();
    let mut results: Vec<serde_json::Value> = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        results.push(run_and_record(
            session,
            conn,
            index,
            statement,
            limit,
            &mut run_so_far,
        )?);
    }
    session.log(
        Level::Info,
        format!(
            "{} statement(s) · {} rows · {} ms",
            statements.len(),
            run_so_far.row_count(),
            run_so_far.elapsed_ms.round()
        ),
    );
    Ok(summarize(&run_so_far, &statements, results))
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
    begin_run(session, vec![picked.clone()]);
    let mut run_so_far = ResultSet::default();
    run_and_record(session, conn, 0, &picked, None, &mut run_so_far)?;
    Ok(json!({
        "statement": picked,
        "rows": run_so_far.row_count(),
        "affected": run_so_far.affected,
        "elapsed_ms": run_so_far.elapsed_ms,
        "data": run_so_far.to_json()["rows"],
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
    let mut last = ResultSet::default();
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
    use crate::test_support::scratch_config;
    use datacraft_model::ResultSet;

    /// A real session: one SQLite connection on `:memory:` and a query tab, the editor's own path to
    /// a run. The guard keeps the scratch config directory to this test for its whole life, and the
    /// index is the query tab's, since the session starts on its Welcome tab.
    fn live_session() -> (std::sync::MutexGuard<'static, ()>, Session, usize) {
        let guard = scratch_config();
        let mut session = Session::new();
        session
            .run(
                "conn.new",
                json!({"name": "local", "kind": "sqlite", "file": ":memory:"}),
            )
            .expect("new connection");
        session
            .run("conn.open", json!({"conn": "local"}))
            .expect("open connection");
        let tab = session.run("sql.newQuery", json!({})).expect("query tab")["tab"]
            .as_u64()
            .unwrap_or(0) as usize;
        (guard, session, tab)
    }

    /// The summary `sql.run` would have returned for the runs the tab holds, so a failed run can be
    /// read the same way a successful one is.
    fn summary_of(runs: &[crate::StatementRun], statements: &[String]) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = runs
            .iter()
            .enumerate()
            .map(|(index, run)| {
                statement_summary(
                    index,
                    &run.statement,
                    run.result.as_ref(),
                    run.elapsed_ms,
                    run.error.clone(),
                )
            })
            .collect();
        summarize(&ResultSet::default(), statements, entries)
    }

    #[test]
    fn a_script_records_one_run_per_statement() {
        let (_guard, mut session, tab) = live_session();
        let out = session
            .run(
                "sql.run",
                json!({"sql": "select 1;\nselect 2;", "limit": 100}),
            )
            .expect("run");
        let sql = &session.tabs[tab].sql;
        assert_eq!(sql.results.len(), 2);
        assert_eq!(sql.results[0].statement, "select 1;");
        assert_eq!(
            sql.results[0].result.as_ref().map(ResultSet::row_count),
            Some(1)
        );
        assert!(sql.results[0].error.is_none());
        assert_eq!(sql.results[1].statement, "select 2;");
        assert_eq!(
            sql.results[1].result.as_ref().map(ResultSet::row_count),
            Some(1)
        );
        let sum = sql.results[0].elapsed_ms + sql.results[1].elapsed_ms;
        assert!((sql.total_ms() - sum).abs() < 1e-9);
        assert!(sql.error().is_none());
        // The summary carries one entry per statement, in run order.
        assert_eq!(out["statements"], 2);
        let entries = out["results"].as_array().cloned().unwrap_or_default();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["index"], 0);
        assert_eq!(entries[0]["statement"], "select 1;");
        assert_eq!(entries[1]["index"], 1);
        assert_eq!(entries[1]["statement"], "select 2;");
        assert_eq!(entries[1]["rows"], 1);
        assert!(entries[1]["error"].is_null());
        // The run's own aggregate still means what it always meant.
        assert_eq!(out["rows"], 1);
        // `sql.history` records one entry per statement, the granularity `sql.runStatement` had.
        assert_eq!(session.history.len(), 2);
    }

    #[test]
    fn a_failed_statement_stops_the_run_and_is_recorded() {
        let (_guard, mut session, tab) = live_session();
        let script = "select 1;\nselect from;";
        let e = session
            .run("sql.run", json!({"sql": script, "limit": 100}))
            .expect_err("the second statement is not SQL");
        assert!(!e.message().is_empty());
        let sql = &session.tabs[tab].sql;
        assert_eq!(sql.results.len(), 2);
        assert_eq!(
            sql.results[0].result.as_ref().map(ResultSet::row_count),
            Some(1)
        );
        let failed = &sql.results[1];
        assert_eq!(failed.statement, "select from;");
        assert!(failed.result.is_none());
        assert!(failed.error.is_some());
        assert_eq!(sql.error(), failed.error.as_deref());
        // The failed run's summary still carries both entries, the last one with the driver's words.
        let summary = summary_of(&sql.results, &split(script));
        let entries = summary["results"].as_array().cloned().unwrap_or_default();
        assert_eq!(entries.len(), 2);
        assert!(entries[0]["error"].is_null());
        assert_eq!(entries[1]["rows"], 0);
        assert!(entries[1]["error"].is_string());
    }

    #[test]
    fn a_second_run_replaces_the_results() {
        let (_guard, mut session, tab) = live_session();
        session
            .run("sql.run", json!({"sql": "select 1;\nselect 2;"}))
            .expect("first run");
        session
            .run("sql.run", json!({"sql": "select 3;"}))
            .expect("second run");
        let sql = &session.tabs[tab].sql;
        assert_eq!(sql.results.len(), 1);
        assert_eq!(sql.results[0].statement, "select 3;");
        assert_eq!(sql.statements, vec!["select 3;".to_string()]);
        assert_eq!(session.history.len(), 3);
    }

    #[test]
    fn a_statement_with_no_columns_keeps_its_affected_count() {
        let (_guard, mut session, tab) = live_session();
        let out = session
            .run(
                "sql.run",
                json!({"sql": "create table t (x integer);\ninsert into t values (1);\nupdate t set x = 2;"}),
            )
            .expect("run");
        let sql = &session.tabs[tab].sql;
        assert_eq!(sql.results.len(), 3);
        let update = &sql.results[2];
        let result = update
            .result
            .as_ref()
            .expect("the update produced a result");
        assert!(result.columns.is_empty());
        assert_eq!(result.affected, 1);
        assert_eq!(update.error, None);
        // The run's aggregate folds the affected counts, the way the driver's own script runner does.
        assert_eq!(out["affected"], 1);
    }

    #[test]
    fn a_run_without_a_query_tab_still_reports_every_statement() {
        // The CLI and the MCP surface call `sql.run` with no tab open, so the summary is the only
        // place their caller can read a run from: it must never depend on the tab holding it.
        let _guard = scratch_config();
        let mut session = Session::new();
        session
            .run(
                "conn.new",
                json!({"name": "local", "kind": "sqlite", "file": ":memory:"}),
            )
            .expect("new connection");
        session
            .run("conn.open", json!({"conn": "local"}))
            .expect("open connection");
        let out = session
            .run("sql.run", json!({"sql": "select 1;\nselect 2;"}))
            .expect("run");
        assert_eq!(out["results"].as_array().map(Vec::len), Some(2));
        assert_eq!(out["results"][1]["statement"], "select 2;");
        assert_eq!(out["results"][1]["rows"], 1);
        assert_eq!(out["rows"], 1);
    }

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
