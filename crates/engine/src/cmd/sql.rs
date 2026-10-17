//! Query editor commands: run, format, complete, explain, history, transactions.

use nook_core::{Error, Result};
use nook_model::ResultSet;
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
/// because a run replaces the register rather than appending to it. `ran_script` records the text the
/// request was issued against — the `sql` the command resolved, which is the tab's own text when the
/// command was given none — so the register can tell whether the editor has moved on since the run,
/// and a run whose source differed from the editor still says so.
fn begin_run(session: &mut Session, ran_script: &str, statements: Vec<String>) {
    if let Ok(tab) = session.active_tab_mut() {
        if tab.kind == TabKind::Query {
            tab.sql.results.clear();
            tab.sql.statements = statements;
            tab.sql.ran_script = ran_script.to_owned();
            tab.sql.last_result = None;
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

/// The single writer of a tab's SQL state: appends the statement's run, and keeps `last_result` and
/// `elapsed_ms` describing the run so far — the aggregate the status bar reads, built by `fold`. A run
/// with no result (the failure that stopped the run) changes only `results`.
fn record_run(session: &mut Session, run: StatementRun, run_so_far: &mut ResultSet) {
    if let Some(result) = run.result.as_ref() {
        fold(run_so_far, result);
    }
    if let Ok(tab) = session.active_tab_mut() {
        if tab.kind == TabKind::Query {
            if run.result.is_some() {
                tab.sql.last_result = Some(run_so_far.clone());
                tab.sql.elapsed_ms = run_so_far.elapsed_ms;
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
            // A statement can change the shape of the database — `ALTER TABLE … RENAME COLUMN` typed
            // in the editor is the one users type — and everything that reads the catalog has to see
            // it, whichever of the run paths issued it.
            refresh_if_the_shape_changed(session, conn, statement);
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

/// Read the connection's catalog again when a statement may have changed the shape of the database.
///
/// `ALTER TABLE … RENAME COLUMN` typed in the editor is the one users type, and every reader of the
/// catalog — an open data tab planning an edit, the tree, the inspector — would otherwise describe
/// the old shape until someone refreshed by hand. One helper so no run path can forget it: the
/// statement funnel `run_and_record` and the atomic run both call it.
fn refresh_if_the_shape_changed(session: &mut Session, conn: usize, statement: &str) {
    if may_change_the_shape(statement) {
        let _ = super::refresh_catalog(session, conn);
    }
}

/// Whether a statement may have changed the shape of the database, in which case the connection's
/// catalog is read again after it runs.
///
/// ponytail: the leading keyword decides, so a statement that merely starts with one of these words
/// (a comment-free `CREATE TEMP VIEW` nobody looks at) costs one catalog read it did not need. Being
/// wrong the other way is what leaves an open data tab planning edits against columns that are gone.
fn may_change_the_shape(statement: &str) -> bool {
    let first = sqltext::tokenize(statement)
        .into_iter()
        .find(|token| {
            matches!(
                token.kind,
                sqltext::TokenKind::Word | sqltext::TokenKind::Keyword
            )
        })
        .map(|token| token.text.to_ascii_uppercase());
    matches!(
        first.as_deref(),
        Some(
            "ALTER"
                | "CREATE"
                | "DROP"
                | "RENAME"
                | "TRUNCATE"
                | "COMMENT"
                | "GRANT"
                | "REVOKE"
                | "VACUUM"
                | "REINDEX"
        )
    )
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
    let sql = sql_from_params_or_editor(session, p)?;
    let statements = split(&sql);
    run_statements(session, p, &sql, statements)
}

/// Run a request's statements and record them under `ran_script`: the text the request was issued
/// against, which `begin_run` stamps for the register's stale rule. `statements` are what actually
/// runs — the text's whole split for `sql.run`, and the one statement a selection resolves to for the
/// paths that cut a statement out of that text — so the stamp stays the source and never the cut.
fn run_statements(
    session: &mut Session,
    p: &Params,
    ran_script: &str,
    statements: Vec<String>,
) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let limit = match p.int_opt("limit", 0) {
        n if n > 0 => Some(n.clamp(1, 5_000_000) as usize),
        _ => None,
    };
    begin_run(session, ran_script, statements.clone());
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
    // The register is stamped with the text the request was issued against, not with the statement
    // cut out of it: `Register::stale` compares that against the editor's whole text, and the caret's
    // statement is not the text the editor holds. `sql` is that text — the parameter when the command
    // was given one, the tab's own otherwise.
    begin_run(session, &sql, vec![picked.clone()]);
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
    // The register is stamped with the text the selection was taken from, not with the statement cut
    // out of it: `Register::stale` compares that stamp with the editor's whole text, so a cut reports
    // an unchanged script as `stale · the script changed since this run` the moment it runs. The
    // executed text stays the statement, which is what the run's results and history describe.
    run_statements(session, p, &sql, vec![statement])
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
        .unwrap_or(nook_core::Dialect::Sqlite);
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
    let out = json!({
        "statement": statement,
        "columns": plan.columns,
        "plan": plan.rows.iter().map(|r| r.iter().map(|v| v.display()).collect::<Vec<_>>()).collect::<Vec<_>>(),
    });
    // The plan is a result like any other, recorded through the same writer every other run goes
    // through: the tab's register then has a statement, a duration and a result set to show. Explain
    // used to fill a field only the UI read, which is why the plan had nowhere to appear once the
    // editor's own result table went.
    let run = crate::StatementRun {
        statement: if analyze {
            format!("EXPLAIN ANALYZE {statement}")
        } else {
            format!("EXPLAIN {statement}")
        },
        elapsed_ms: plan.elapsed_ms,
        result: Some(plan),
        error: None,
    };
    let mut run_so_far = ResultSet::default();
    // The same stamp the other runs leave: the text the request was issued against, so the register
    // can tell whether the editor has moved on since — not the statement the explain was reduced to.
    begin_run(session, &sql, vec![run.statement.clone()]);
    record_run(session, run, &mut run_so_far);
    Ok(out)
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
    // The store is the durable twin: clear it too, or the next start restores what the user just
    // wiped. A failed clear is reported, not fatal — the in-memory history is already gone.
    if let Err(e) = crate::store::clear_history() {
        session.log(Level::Warn, format!("history clear: {}", e.message()));
    }
    Ok(json!({ "cleared": n }))
}

fn transaction(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let action = p.str_arg("action")?.to_ascii_lowercase();
    let open = {
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
        driver.in_transaction()
    };
    // A transaction typed in the editor ends here, and the catalog is read again: the statements that
    // ran inside it already refreshed it, and a rollback has just undone the DDL they saw — the tree,
    // the inspector and the next grid edit would otherwise describe a column the database does not
    // have. A commit is refreshed too, because MySQL commits DDL implicitly and a `rollback` there is
    // not a promise that nothing changed. `begin` changes no shape.
    if action != "begin" && action != "start" {
        let _ = super::refresh_catalog(session, conn);
    }
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
                // The rollback undid the statements that ran, and the DDL among them may already have
                // been read into the catalog — or, on MySQL, committed for real. Reading the schema
                // again is the only answer that matches the database in both cases.
                let _ = super::refresh_catalog(session, conn);
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
    // Committed, so the shape readers have to see is the one this transaction just made. Nothing here
    // is read mid-transaction: a rollback is the early return above, which reads the schema back
    // itself, and a statement that changed the shape but failed is not in this list.
    for statement in &statements {
        refresh_if_the_shape_changed(session, conn, statement);
    }
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
    nook_drivers::split_script(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::scratch_config;
    use nook_model::ResultSet;

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

    /// Explain is a run like any other: it leaves the plan in the tab's results, as a statement of its
    /// own with a duration, so the register paints it in a tab. It used to write a field only the
    /// editor read, which left the plan invisible once that table went.
    #[test]
    fn explain_records_the_plan_as_the_tabs_own_run() {
        let (_guard, mut session, tab) = live_session();
        session.tabs[tab].sql.text = "select 1 as one;\n".to_owned();
        let p = Params::from_json(json!({"statement": "select 1 as one;"})).expect("params");
        let out = explain(&mut session, &p).expect("explain");
        assert_eq!(out["statement"], "select 1 as one;");
        let runs = &session.tabs[tab].sql.results;
        assert_eq!(runs.len(), 1, "the plan is the tab's one run");
        assert_eq!(
            runs[0].statement, "EXPLAIN select 1 as one;",
            "the tab names what was explained"
        );
        let plan = runs[0].result.as_ref().expect("the plan is a result set");
        assert!(!plan.columns.is_empty(), "the plan has its own columns");
        assert!(!plan.rows.is_empty(), "the plan has its own rows");
        assert_eq!(runs[0].error, None);
        assert_eq!(
            session.tabs[tab].sql.ran_script, "select 1 as one;\n",
            "the register is stamped with the text the request came from, not the statement cut out of it"
        );
    }

    /// The register answers "has the text moved on since this run?" by comparing the editor's whole
    /// text with the text the run was issued against. The single-statement paths cut their statement
    /// out of that text and used to stamp the register with the cut, so an unchanged,
    /// newline-terminated script read as stale the moment `Run statement` ran the statement in it.
    #[test]
    fn a_single_statement_run_stamps_the_text_the_request_came_from() {
        let (_guard, mut session, tab) = live_session();
        session.tabs[tab].sql.text = "select 1 as one;\n".to_owned();
        let p = Params::from_json(json!({"index": 0})).expect("params");
        run_statement(&mut session, &p).expect("the statement runs");
        assert_eq!(
            session.tabs[tab].sql.ran_script, "select 1 as one;\n",
            "the register is stamped with the text the request came from, not the statement it cut out"
        );

        // And a run given a text of its own is stamped with that text, so the register can still say
        // the editor has moved on since it: the statement came from the parameter, not from the tab.
        let p =
            Params::from_json(json!({"index": 0, "sql": "select 2 as two;\n"})).expect("params");
        run_statement(&mut session, &p).expect("the parameter's statement runs");
        assert_eq!(
            session.tabs[tab].sql.ran_script, "select 2 as two;\n",
            "the register is stamped with the text the request came from"
        );
    }

    /// `Run selection` cuts its statement out of the text the request came from, so the register has
    /// to be stamped with that text rather than with the cut, exactly as `Run statement` and explain
    /// are: an unchanged, newline-terminated script used to read `stale · the script changed since
    /// this run` the moment its own selection ran, because the register compares the editor's whole
    /// text. The statement still runs — the results and history are the selected statement's.
    #[test]
    fn a_selection_run_stamps_the_text_the_selection_came_from() {
        let (_guard, mut session, tab) = live_session();
        session.tabs[tab].sql.text = "select 1 as one;\nselect 2 as two;\n".to_owned();
        let p = Params::from_json(json!({"start": 0})).expect("params");
        let out = run_selection(&mut session, &p).expect("the selection runs");
        assert_eq!(
            out["results"][0]["statement"], "select 1 as one;",
            "the statement the selection cut out is what runs"
        );
        assert_eq!(
            session.tabs[tab].sql.results[0].statement, "select 1 as one;",
            "the register's own run names the statement that ran, not the whole text"
        );
        assert_eq!(
            session.tabs[tab].sql.ran_script, "select 1 as one;\nselect 2 as two;\n",
            "the register is stamped with the text the selection came from, not the statement cut out of it"
        );

        // And a selection given a text of its own is stamped with that text, so the register can still
        // say the editor has moved on since it: the statement came from the parameter, not the tab.
        let p =
            Params::from_json(json!({"start": 0, "sql": "select 3 as three;\n"})).expect("params");
        run_selection(&mut session, &p).expect("the parameter's selection runs");
        assert_eq!(
            session.tabs[tab].sql.ran_script, "select 3 as three;\n",
            "the register is stamped with the text the request came from"
        );
    }

    /// The stamp every run path owes the register, checked across all four entry points at once:
    /// `SqlState::ran_script` is the text the request resolved to — the `sql` parameter when the
    /// command was given one, the tab's own editor text otherwise — and never the statement a path
    /// cut out of that text. The executed statement, the tab's `results[0]` and the history entry
    /// still name what actually ran, so the stronger stamp costs no truth elsewhere. Three review
    /// rounds fixed this one path at a time; this pins the siblings against the same rule.
    #[test]
    fn every_run_path_stamps_the_source_and_still_runs_the_statement() {
        let (_guard, mut session, tab) = live_session();

        // `sql.run`, given a script: the whole parameter is the stamp, both statements run, and the
        // history records each statement as it always did.
        let script = "select 1 as one;\nselect 2 as two;\n";
        let out = session
            .run("sql.run", json!({"sql": script}))
            .expect("run with sql");
        assert_eq!(
            session.tabs[tab].sql.ran_script, script,
            "sql.run stamps the script the request came from"
        );
        assert_eq!(out["results"][0]["statement"], "select 1 as one;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement,
            "select 1 as one;"
        );
        let tail: Vec<&str> = session
            .history
            .iter()
            .rev()
            .take(2)
            .map(|h| h.sql.as_str())
            .collect();
        assert_eq!(tail, vec!["select 2 as two;", "select 1 as one;"]);

        // `sql.run`, from the editor: with no `sql` the tab's own text is both the source and the stamp.
        let editor = "select 3 as three;\n";
        session.tabs[tab].sql.text = editor.to_owned();
        let out = session
            .run("sql.run", json!({}))
            .expect("run from the editor");
        assert_eq!(
            session.tabs[tab].sql.ran_script, editor,
            "sql.run with no sql stamps the editor's text"
        );
        assert_eq!(out["results"][0]["statement"], "select 3 as three;");
        assert_eq!(
            session.history.last().map(|h| h.sql.as_str()),
            Some("select 3 as three;"),
            "history still names the statement that ran"
        );

        // `sql.runStatement`, given a script: the caret's statement runs while the whole script is
        // the stamp, so the register does not read the unchanged editor as stale.
        let script = "select 4 as four;\nselect 5 as five;\n";
        let out = session
            .run("sql.runStatement", json!({"index": 1, "sql": script}))
            .expect("run the statement");
        assert_eq!(
            session.tabs[tab].sql.ran_script, script,
            "sql.runStatement stamps the text the statement was cut from, not the cut"
        );
        assert_eq!(out["statement"], "select 5 as five;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement,
            "select 5 as five;"
        );
        assert_eq!(
            session.history.last().map(|h| h.sql.as_str()),
            Some("select 5 as five;"),
            "history still names the statement that ran"
        );

        // `sql.runStatement`, from the editor: the pick obeys the index, the stamp stays the text.
        let editor = "select 6 as six;\nselect 7 as seven;\n";
        session.tabs[tab].sql.text = editor.to_owned();
        let out = session
            .run("sql.runStatement", json!({"index": 0}))
            .expect("run the caret's statement");
        assert_eq!(
            session.tabs[tab].sql.ran_script, editor,
            "sql.runStatement with no sql stamps the editor's whole text"
        );
        assert_eq!(out["statement"], "select 6 as six;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement,
            "select 6 as six;"
        );

        // `sql.runSelection`, given a script: the selected statement runs, the text it was selected
        // from is the stamp. This is the assertion the reverted stamp fails.
        let script = "select 8 as eight;\nselect 9 as nine;\n";
        let out = session
            .run("sql.runSelection", json!({"start": 0, "sql": script}))
            .expect("run the selection");
        assert_eq!(
            session.tabs[tab].sql.ran_script, script,
            "sql.runSelection stamps the text the selection came from, not the statement cut out of it"
        );
        assert_eq!(out["results"][0]["statement"], "select 8 as eight;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement,
            "select 8 as eight;"
        );
        assert_eq!(
            session.history.last().map(|h| h.sql.as_str()),
            Some("select 8 as eight;"),
            "history still names the statement that ran"
        );

        // `sql.runSelection`, from the editor: same rule, the tab's text as the source and the stamp.
        let editor = "select 10 as ten;\nselect 11 as eleven;\n";
        session.tabs[tab].sql.text = editor.to_owned();
        let out = session
            .run("sql.runSelection", json!({"start": 0}))
            .expect("run the editor's selection");
        assert_eq!(
            session.tabs[tab].sql.ran_script, editor,
            "sql.runSelection with no sql stamps the editor's whole text"
        );
        assert_eq!(out["results"][0]["statement"], "select 10 as ten;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement,
            "select 10 as ten;"
        );

        // `sql.explain`, given a script: the plan is the run, the statement it explains is the run's
        // name, and the stamp is again the text the request came from.
        let history_before = session.history.len();
        let script = "select 12 as twelve;\n";
        let out = session
            .run("sql.explain", json!({"sql": script}))
            .expect("explain with sql");
        assert_eq!(
            session.tabs[tab].sql.ran_script, script,
            "sql.explain stamps the text the request came from, not the statement it reduced to"
        );
        assert_eq!(out["statement"], "select 12 as twelve;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement, "EXPLAIN select 12 as twelve;",
            "the register names what the plan run was"
        );
        // ponytail: explain plans through the driver, never through `run_sql`, so it writes no
        // history entry to check; pinned as an absence. If explain ever records one, assert its text.
        assert_eq!(
            session.history.len(),
            history_before,
            "explain writes no history entry"
        );

        // `sql.explain`, from the editor: the tab's text is both the source and the stamp.
        let editor = "select 13 as thirteen;\n";
        session.tabs[tab].sql.text = editor.to_owned();
        let out = session
            .run("sql.explain", json!({}))
            .expect("explain from the editor");
        assert_eq!(
            session.tabs[tab].sql.ran_script, editor,
            "sql.explain with no sql stamps the editor's text"
        );
        assert_eq!(out["statement"], "select 13 as thirteen;");
        assert_eq!(
            session.tabs[tab].sql.results[0].statement,
            "EXPLAIN select 13 as thirteen;"
        );
    }

    #[test]
    fn completion_works_without_a_catalog() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"prefix": "sel"})).expect("params");
        let out = complete(&mut session, &p).expect("complete");
        assert!(out["candidates"].as_array().map(Vec::len).unwrap_or(0) > 0);
    }

    /// The column names one connection's catalog holds for a table.
    fn catalog_columns(session: &Session, conn: &str, table: &str) -> Vec<String> {
        let index = session
            .conn_index(conn)
            .expect("the connection is registered");
        session.conns[index]
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.find_table(None, table))
            .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    }

    /// A rollback in the editor undoes the DDL that the run path already refreshed the catalog for.
    /// The catalog then describes a column the database no longer has, and the next grid edit, the
    /// tree and the inspector all read it: `row 0 has no value for key memo` is that stale shape.
    #[test]
    fn a_rollback_in_the_editor_puts_the_catalog_back() {
        let (_guard, mut session, _tab) = live_session();
        session
            .run(
                "sql.run",
                json!({"sql": "create table t (id integer primary key, note text)"}),
            )
            .expect("create");
        session
            .run("sql.transaction", json!({"action": "begin"}))
            .expect("begin");
        session
            .run(
                "sql.run",
                json!({"sql": "alter table t rename column note to memo"}),
            )
            .expect("rename");

        session
            .run("sql.transaction", json!({"action": "rollback"}))
            .expect("rollback");

        assert_eq!(
            catalog_columns(&session, "local", "t"),
            vec!["id".to_string(), "note".to_string()],
            "the rollback put the database back and the catalog has to follow it"
        );
    }

    /// The atomic run's own rollback: the statements before the failing one are undone, so the shape
    /// the readers see must be the one the database has, not the one the transaction attempted.
    #[test]
    fn a_failed_atomic_run_leaves_the_catalog_on_the_shape_the_database_kept() {
        let (_guard, mut session, _tab) = live_session();
        session
            .run(
                "sql.run",
                json!({"sql": "create table t (id integer primary key, note text)"}),
            )
            .expect("create");

        let failed = session.run(
            "sql.runInTransaction",
            json!({"sql": "alter table t rename column note to memo; select * from nope;"}),
        );
        assert!(failed.is_err(), "the second statement cannot run");

        assert_eq!(
            catalog_columns(&session, "local", "t"),
            vec!["id".to_string(), "note".to_string()],
            "the transaction was rolled back, so the catalog must not keep its shape"
        );
    }

    #[test]
    fn a_shape_changing_keyword_is_read_past_a_comment() {
        assert!(may_change_the_shape(
            "ALTER TABLE users RENAME COLUMN id TO uid"
        ));
        assert!(may_change_the_shape(
            "  -- renamed later\ncreate index ix on users (email)"
        ));
        assert!(may_change_the_shape("drop table users;"));
        assert!(!may_change_the_shape("select * from users"));
        assert!(!may_change_the_shape(
            "with t as (select 1) select * from t"
        ));
        assert!(!may_change_the_shape("insert into users values (1, 'a')"));
        assert!(!may_change_the_shape(""));
    }

    #[test]
    fn a_run_that_changes_the_shape_refreshes_the_catalog() {
        let (_guard, mut session, _tab) = live_session();
        session
            .run(
                "sql.run",
                json!({"sql": "create table t (id integer primary key, note text)"}),
            )
            .expect("create");
        assert_eq!(
            catalog_columns(&session, "local", "t"),
            vec!["id".to_string(), "note".to_string()],
            "a table the editor just created is in the catalog"
        );

        session
            .run(
                "sql.run",
                json!({"sql": "alter table t rename column note to memo"}),
            )
            .expect("rename");
        assert_eq!(
            catalog_columns(&session, "local", "t"),
            vec!["id".to_string(), "memo".to_string()],
            "and a rename reads the new name"
        );

        // A statement that only reads leaves the catalog where it was.
        session
            .run("sql.run", json!({"sql": "select * from t"}))
            .expect("select");
        assert_eq!(
            catalog_columns(&session, "local", "t"),
            vec!["id".to_string(), "memo".to_string()]
        );
    }

    #[test]
    fn every_run_path_refreshes_after_a_rename() {
        // The editor has more than one way to run a statement, and the shape the readers see must not
        // depend on which one the user picked.
        for (command, params) in [
            (
                "sql.run",
                json!({"sql": "alter table t rename column note to memo"}),
            ),
            ("sql.runStatement", json!({"index": 0})),
            ("sql.runSelection", json!({"start": 0, "end": 40})),
            (
                "sql.runInTransaction",
                json!({"sql": "alter table t rename column note to memo"}),
            ),
        ] {
            let (_guard, mut session, tab) = live_session();
            session
                .run(
                    "sql.run",
                    json!({"sql": "create table t (id integer primary key, note text)"}),
                )
                .expect("create");

            if command == "sql.runStatement" || command == "sql.runSelection" {
                // Both cut a statement out of the tab's text, so the text is where it comes from, and
                // the caret has to sit inside the statement the selection starts at.
                session.tabs[tab].sql.text =
                    "select 1;\nalter table t rename column note to memo;\n".to_owned();
                let params = if command == "sql.runStatement" {
                    json!({"index": 1})
                } else {
                    json!({"start": 11, "end": 50})
                };
                session.run(command, params).expect(command);
            } else {
                session.run(command, params).expect(command);
            }

            assert_eq!(
                catalog_columns(&session, "local", "t"),
                vec!["id".to_string(), "memo".to_string()],
                "{command} left the catalog on the old shape"
            );
        }
    }
}
