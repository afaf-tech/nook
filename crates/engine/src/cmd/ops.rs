//! Data movement and automation: transfer, synchronization, backup/restore and scheduled jobs.

use std::path::PathBuf;

use nook_core::{DriverKind, Error, Result};
use nook_drivers::backup;
use nook_drivers::transfer::{self, TransferOptions};
use serde_json::json;

use crate::cmd::grid::expand;
use crate::jobs::{Job, JobKind, Schedule};
use crate::{CommandSpec, Level, Params, Session};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("transfer.data", "Data transfer", "Transfer", transfer_data)
            .params(r#"{"from":"local","to":"staging","tables":["users"],"create_tables":true,"truncate_target":true}"#)
            .help("Copy tables between any two open connections, across engines. \"tables\": [\"*\"] for all."),
        CommandSpec::new("transfer.preview", "Preview a transfer", "Transfer", preview_transfer)
            .params(r#"{"from":"local","to":"staging","tables":["*"]}"#)
            .help("What would be created and copied, without touching the target."),
        CommandSpec::new("sync.data", "Data synchronization", "Transfer", sync_data)
            .params(r#"{"from":"prod","to":"staging","tables":["*"],"dry_run":true}"#)
            .help("Row-level comparison by primary key; dry_run reports without applying."),
        CommandSpec::new("diff.structure", "Compare structures", "Sync", diff_structure)
            .params(r#"{"from":"prod","to":"staging"}"#)
            .help("The DDL that would make the target look like the source."),
        CommandSpec::new("sync.structure", "Structure synchronization", "Sync", sync_structure)
            .params(r#"{"from":"prod","to":"staging","apply":false,"allow_destructive":false}"#),
        CommandSpec::new("backup.dump", "Backup", "Backup", dump)
            .params(r#"{"conn":"local","path":"~/backups/shop.sql","schema_only":false}"#)
            .help("SQLite writes a portable .sql script; Postgres and MySQL use pg_dump/mysqldump."),
        CommandSpec::new("backup.restore", "Restore", "Backup", restore)
            .params(r#"{"conn":"local","path":"~/backups/shop.sql"}"#),
        CommandSpec::new("backup.list", "List backups", "Backup", list_backups)
            .params(r#"{"dir":"~/backups"}"#),
        CommandSpec::new("job.list", "Scheduled jobs", "Automation", job_list),
        CommandSpec::new("job.create", "Create a job", "Automation", job_create)
            .params(r#"{"id":"nightly","name":"Nightly backup","kind":"backup","conn":"local","params":{"path":"~/backups/shop.sql"},"schedule":"03:00"}"#)
            .help("kind: query, backup, restore, transfer, sync_data, sync_structure, datagen, report. schedule: \"03:00\", \"every 15m\" or \"manual\"."),
        CommandSpec::new("job.delete", "Delete a job", "Automation", job_delete).params(r#"{"id":"nightly"}"#),
        CommandSpec::new("job.setEnabled", "Enable or disable a job", "Automation", job_set_enabled)
            .params(r#"{"id":"nightly","enabled":false}"#),
        CommandSpec::new("job.run", "Run a job now", "Automation", job_run).params(r#"{"id":"nightly"}"#),
        CommandSpec::new("job.runDue", "Run every due job", "Automation", job_run_due)
            .help("Called by the app's timer and by `nook-cli job serve`."),
        CommandSpec::new("job.logs", "Job log", "Automation", job_logs)
            .params(r#"{"id":"nightly","limit":20}"#),
        CommandSpec::new("job.notifications", "Webhook on failure", "Automation", job_notifications)
            .params(r#"{"id":"nightly","webhook":"https://hooks.example.com/nook"}"#)
            .help("List, set or clear a job's failure webhook. An empty webhook clears it. Email is a webhook to a mail relay."),
    ]
}

fn conn_id_at(session: &Session, index: usize) -> Result<String> {
    session
        .conns
        .get(index)
        .map(|c| c.cfg.id.clone())
        .ok_or_else(|| Error::not_found(format!("no connection at {index}")))
}

/// The second connection of a two-sided operation.
fn pair(session: &Session, p: &Params) -> Result<(usize, usize)> {
    let from = match p.str_opt("from").or_else(|| p.str_opt("source")) {
        Some(id) => session.conn_index(&id)?,
        None => session
            .active_conn
            .ok_or_else(|| Error::config("open a connection first"))?,
    };
    let to = match p.str_opt("to").or_else(|| p.str_opt("target")) {
        Some(id) => session.conn_index(&id)?,
        None => return Err(Error::config("pass \"to\" with the target connection")),
    };
    Ok((from, to))
}

fn tables_of(p: &Params) -> Vec<String> {
    let list = p.list("tables");
    if list.is_empty() {
        vec!["*".to_string()]
    } else {
        list
    }
}

fn transfer_options(p: &Params) -> Result<TransferOptions> {
    Ok(TransferOptions {
        create_tables: p.bool_opt("create_tables", true),
        drop_target: p.bool_opt("drop_target", false),
        truncate_target: p.bool_opt("truncate_target", false),
        include_data: p.bool_opt("include_data", true),
        include_indexes: p.bool_opt("include_indexes", true),
        filter: p.str_opt("filter").unwrap_or_default(),
        fail_fast: p.bool_opt("fail_fast", false),
        max_rows: p.int_opt("max_rows", 1_000_000).clamp(1, 100_000_000) as usize,
        column_mapping: super::tools::parse_mapping(&p.list("columnMapping"))?.unwrap_or_default(),
    })
}

fn transfer_data(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (from, to) = pair(session, p)?;
    let catalog = session.catalog_of(from)?.clone();
    let tables = tables_of(p);
    let options = transfer_options(p)?;
    if !options.column_mapping.is_empty() && (tables.len() != 1 || tables[0] == "*") {
        return Err(Error::config(
            "columnMapping applies to one table; name it in \"tables\" instead of \"*\"",
        ));
    }
    let dst_schema = p.str_opt("dst_schema");
    let mut log: Vec<String> = Vec::new();
    let report = {
        let (src, dst) = session.two_drivers(from, to)?;
        transfer::transfer(
            src,
            dst,
            &catalog,
            &tables,
            dst_schema.as_deref(),
            &options,
            &mut |line: &str| log.push(line.to_string()),
        )?
    };
    let _ = crate::cmd::refresh_catalog(session, to);
    session.log(Level::Success, format!("transfer: {}", report.summary()));
    Ok(json!({
        "summary": report.summary(),
        "rows": report.rows(),
        "tables": report.tables.iter().map(|t| json!({
            "table": t.table, "rows": t.rows, "statements": t.statements,
            "skipped": t.skipped, "error": t.error,
        })).collect::<Vec<_>>(),
        "failed": report.errors().len(),
        "elapsed_ms": report.elapsed_ms,
        "log": log,
    }))
}

fn preview_transfer(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (from, to) = pair(session, p)?;
    let catalog = session.catalog_of(from)?.clone();
    let target_catalog = session.catalog_of(to)?.clone();
    let tables = tables_of(p);
    let dst_schema = p.str_opt("dst_schema");
    let mut plan = Vec::new();
    for schema in &catalog.schemas {
        for table in &schema.tables {
            if table.kind != nook_model::TableKind::Table {
                continue;
            }
            if !(tables.iter().any(|t| t == "*")
                || tables.iter().any(|t| t == &table.name || t == &table.key()))
            {
                continue;
            }
            let exists = target_catalog
                .find_table(dst_schema.as_deref(), &table.name)
                .is_some();
            let rows = {
                let driver = session.driver_for(from)?;
                driver
                    .count_rows(table.schema.as_deref(), &table.name)
                    .unwrap_or(0)
            };
            let target_rows = if exists {
                let driver = session.driver_for(to)?;
                driver
                    .count_rows(dst_schema.as_deref(), &table.name)
                    .unwrap_or(0)
            } else {
                0
            };
            plan.push(json!({
                "table": table.key(),
                "source_rows": rows,
                "target_exists": exists,
                "target_rows": target_rows,
                "columns": table.columns.len(),
                "primary_key": table.primary_key(),
            }));
        }
    }
    Ok(json!({
        "from": conn_id_at(session, from)?,
        "to": conn_id_at(session, to)?,
        "tables": plan,
        "total_source_rows": plan.iter().filter_map(|t| t["source_rows"].as_u64()).sum::<u64>(),
    }))
}

fn sync_data(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (from, to) = pair(session, p)?;
    let catalog = session.catalog_of(from)?.clone();
    let tables = tables_of(p);
    let dry_run = p.bool_opt("dry_run", false);
    let batch = p.int_opt("batch", 500).clamp(1, 20_000) as usize;
    let dst_schema = p.str_opt("dst_schema");
    let mut log: Vec<String> = Vec::new();
    let report = {
        let (src, dst) = session.two_drivers(from, to)?;
        transfer::sync_data(
            src,
            dst,
            &catalog,
            &tables,
            dst_schema.as_deref(),
            batch,
            dry_run,
            &mut |line: &str| log.push(line.to_string()),
        )?
    };
    if !dry_run {
        let _ = crate::cmd::refresh_catalog(session, to);
    }
    session.log(Level::Info, format!("sync: {}", report.summary()));
    Ok(json!({
        "summary": report.summary(),
        "inserts": report.inserts(),
        "updates": report.updates(),
        "deletes": report.deletes(),
        "applied": report.applied,
        "dry_run": dry_run,
        "elapsed_ms": report.elapsed_ms,
        "log": log,
    }))
}

/// Structure diff between the source catalog and the target catalog of two open connections.
fn structure_changes(
    session: &mut Session,
    p: &Params,
) -> Result<(usize, usize, Vec<nook_model::Change>, nook_core::Dialect)> {
    let (from, to) = pair(session, p)?;
    let source = session.catalog_of(from)?.clone();
    let target = session.catalog_of(to)?.clone();
    let dialect = {
        let driver = session.driver_for(to)?;
        driver.dialect()
    };
    let mut changes = nook_model::diff_catalogs(&source, &target, dialect);
    if !p.bool_opt("include_drops", true) {
        changes.retain(|c| {
            !matches!(
                c.kind,
                nook_model::ChangeKind::DropTable | nook_model::ChangeKind::DropColumn
            )
        });
    }
    Ok((from, to, changes, dialect))
}

fn diff_structure(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (from, to, changes, _) = structure_changes(session, p)?;
    Ok(json!({
        "from": conn_id_at(session, from)?,
        "to": conn_id_at(session, to)?,
        "count": changes.len(),
        "changes": changes.iter().map(|c| json!({
            "kind": c.kind.label(), "object": c.object, "sql": c.sql,
            "destructive": c.destructive(), "manual": c.manual, "note": c.note,
        })).collect::<Vec<_>>(),
    }))
}

fn sync_structure(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (_, to, changes, _) = structure_changes(session, p)?;
    let apply = p.bool_opt("apply", false);
    let allow_destructive = p.bool_opt("allow_destructive", false);
    if !apply {
        return diff_structure(session, p);
    }
    let mut applied = Vec::new();
    let mut skipped = Vec::new();
    for change in &changes {
        if change.manual || change.sql.trim().is_empty() {
            skipped.push(json!({ "object": change.object, "reason": change.note }));
            continue;
        }
        if change.destructive() && !allow_destructive {
            skipped.push(json!({
                "object": change.object,
                "reason": "destructive; pass \"allow_destructive\": true to run it",
            }));
            continue;
        }
        match crate::cmd::run_sql(session, to, &change.sql, Some(1)) {
            Ok(_) => applied.push(json!({ "object": change.object, "sql": change.sql })),
            Err(e) => {
                skipped.push(json!({ "object": change.object, "reason": e.to_string() }));
                if p.bool_opt("fail_fast", false) {
                    break;
                }
            }
        }
    }
    let _ = crate::cmd::refresh_catalog(session, to);
    session.log(
        Level::Success,
        format!(
            "structure sync: {} applied, {} skipped",
            applied.len(),
            skipped.len()
        ),
    );
    Ok(json!({ "applied": applied, "skipped": skipped, "planned": changes.len() }))
}

fn dump(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let kind = session.conns[index].cfg.kind;
    let path = expand(
        &p.str_opt("path")
            .unwrap_or_else(|| format!("{}.sql", session.conns[index].cfg.name)),
        &session.home,
    );
    let tables = tables_of(p);
    let schema_only = p.bool_opt("schema_only", false);
    let cfg = session.conns[index].cfg.clone();

    if kind == DriverKind::Sqlite {
        let catalog = session.catalog_of(index)?.clone();
        let mut log = Vec::new();
        let max_rows = session.settings.max_rows;
        let script = {
            let driver = session.driver_for(index)?;
            transfer::dump_script(
                driver,
                &catalog,
                &tables,
                schema_only,
                max_rows,
                &mut |line: &str| log.push(line.to_string()),
            )?
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
        }
        std::fs::write(&path, &script)
            .map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
        session.log(
            Level::Success,
            format!("backup written: {}", path.display()),
        );
        return Ok(json!({
            "path": path.to_string_lossy(),
            "bytes": script.len(),
            "statements": nook_drivers::split_script(&script).len(),
            "engine": "in-process SQL script",
            "schema_only": schema_only,
        }));
    }

    let plan = backup::backup_plan(&cfg, &tables, schema_only, &path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
    }
    let outcome = backup::execute(&plan, Some(&path))?;
    session.log(
        Level::Success,
        format!(
            "backup written: {} ({} bytes)",
            path.display(),
            outcome.bytes
        ),
    );
    Ok(json!({
        "path": path.to_string_lossy(),
        "bytes": outcome.bytes,
        "command": outcome.command,
        "engine": kind.label(),
        "elapsed_ms": outcome.elapsed_ms,
    }))
}

fn restore(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let kind = session.conns[index].cfg.kind;
    let path = expand(&p.str_arg("path")?, &session.home);
    if !path.is_file() {
        return Err(Error::not_found(format!(
            "{} is not a file",
            path.display()
        )));
    }
    let cfg = session.conns[index].cfg.clone();
    if kind == DriverKind::Sqlite {
        let script = std::fs::read_to_string(&path)
            .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
        let statements = nook_drivers::split_script(&script);
        let count = statements.len();
        crate::cmd::run_sql(session, index, &script, Some(1))?;
        let _ = crate::cmd::refresh_catalog(session, index);
        session.log(Level::Success, format!("restored {count} statements"));
        return Ok(json!({
            "path": path.to_string_lossy(),
            "statements": count,
            "engine": "in-process SQL script",
        }));
    }
    let plan = backup::restore_plan(&cfg, &path)?;
    let outcome = backup::execute(&plan, None)?;
    let _ = crate::cmd::refresh_catalog(session, index);
    session.log(Level::Info, format!("restore ran: {}", outcome.command));
    Ok(json!({
        "path": path.to_string_lossy(),
        "command": outcome.command,
        "elapsed_ms": outcome.elapsed_ms,
        "stderr": outcome.stderr,
    }))
}

fn list_backups(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let dir = expand(
        &p.str_opt("dir").unwrap_or_else(|| "~/backups".into()),
        &session.home,
    );
    if !dir.is_dir() {
        return Ok(
            json!({ "dir": dir.to_string_lossy(), "files": [], "note": "directory does not exist yet" }),
        );
    }
    let mut files: Vec<serde_json::Value> = Vec::new();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| Error::io(format!("read {}: {e}", dir.display())))?;
    for entry in entries.flatten().take(500) {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                files.push(json!({
                    "name": entry.file_name().to_string_lossy(),
                    "bytes": meta.len(),
                    "modified": meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                }));
            }
        }
    }
    files.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    Ok(json!({ "dir": dir.to_string_lossy(), "files": files }))
}

// ------------------------------------------------------------------ jobs

fn job_list(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    Ok(json!({
        "store": crate::profiles::jobs_path().to_string_lossy(),
        "jobs": session.jobs.jobs.iter().map(|j| json!({
            "id": j.id, "name": j.name, "kind": j.kind.label(),
            "command": j.command_id(), "conn": j.conn, "target": j.target,
            "schedule": j.schedule.describe(), "enabled": j.enabled,
            "runs": j.runs, "failures": j.failures,
            "last_run": j.last_run, "last_status": j.last_status,
        })).collect::<Vec<_>>(),
        "log_lines": session.jobs.log.len(),
    }))
}

fn job_create(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let name = p.str_opt("name").unwrap_or_else(|| "job".into());
    let id = p.str_opt("id").unwrap_or_else(|| {
        name.to_ascii_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect()
    });
    let kind = JobKind::parse(&p.str_opt("kind").unwrap_or_else(|| "query".into()))
        .ok_or_else(|| Error::config("kind must be query, backup, restore, transfer, sync_data, sync_structure, datagen or report"))?;
    let mut job = Job::new(id.clone(), name, kind);
    job.conn = p
        .str_opt("conn")
        .or_else(|| session.active_conn.map(|i| session.conns[i].cfg.id.clone()))
        .unwrap_or_default();
    job.target = p.str_opt("target").unwrap_or_default();
    job.params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    job.schedule = Schedule::parse(&p.str_opt("schedule").unwrap_or_else(|| "manual".into()))?;
    job.enabled = p.bool_opt("enabled", true);
    job.webhook = p.str_opt("webhook").unwrap_or_default();
    job.then = p.list("then");
    session.jobs.add(job)?;
    validate_chain(&session.jobs, &id)?;
    session.log(Level::Success, format!("job '{id}' created"));
    job_list(session, p)
}

fn job_delete(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let id = p.str_arg("id")?;
    let job = session.jobs.remove(&id)?;
    Ok(json!({ "removed": job.name }))
}

fn job_set_enabled(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let id = p.str_arg("id")?;
    let enabled = p.bool_opt("enabled", true);
    let job = session
        .jobs
        .get_mut(&id)
        .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
    job.enabled = enabled;
    session.jobs.save()?;
    Ok(json!({ "id": id, "enabled": enabled }))
}

/// Every `then` target must exist, and the chain graph must be acyclic — checked from the job
/// that just gained an edge, so a bad edge is rejected at create time, before it can run.
fn validate_chain(store: &crate::jobs::JobStore, start: &str) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![start.to_string()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            return Err(Error::config(format!(
                "job '{start}' would chain into itself"
            )));
        }
        let job = store
            .get(&id)
            .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
        for next in &job.then {
            stack.push(next.clone());
        }
    }
    Ok(())
}

/// Run one job by dispatching its command with the job's parameters.
pub fn run_job(session: &mut Session, id: &str) -> Result<String> {
    let job = session
        .jobs
        .get(id)
        .cloned()
        .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
    let started = std::time::Instant::now();
    let mut map = match job.params.clone() {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if !job.conn.is_empty() {
        map.insert("conn".into(), json!(job.conn));
    }
    if !job.target.is_empty() {
        map.insert("to".into(), json!(job.target));
    }
    let result = session.run(job.command_id(), serde_json::Value::Object(map));
    let ms = started.elapsed().as_millis();
    let at = crate::jobs::now();
    match result {
        Ok(value) => {
            let message = value
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    value
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|p| format!("wrote {p}"))
                        .unwrap_or_else(|| "ok".to_string())
                });
            session.jobs.record(&job.id, true, message.clone(), ms, at);
            // The chain runs after a success, in order, and a failure stops it. Each chained
            // run is recorded in its own job's log by the recursive run_job call.
            for next in &job.then {
                run_job(session, next)?;
            }
            Ok(message)
        }
        Err(e) => {
            session.jobs.record(&job.id, false, e.to_string(), ms, at);
            if !job.webhook.is_empty() {
                notify_webhook(&job, &e, ms, at, session);
            }
            Err(e)
        }
    }
}

/// POST the job's failure as JSON to its webhook via the system curl — the same convention the
/// AI client uses (`NOOK_CURL_BIN` names a fake in tests). A webhook failure is logged and
/// swallowed: it must never mask the job's own error.
fn notify_webhook(job: &Job, error: &Error, ms: u128, at: u64, session: &mut Session) {
    let body = json!({
        "job": job.id,
        "name": job.name,
        "ok": false,
        "error": error.to_string(),
        "elapsed_ms": ms,
        "at": at,
    });
    let bin = std::env::var("NOOK_CURL_BIN").unwrap_or_else(|_| "curl".to_string());
    let args = [
        "-sS".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        "content-type: application/json".to_string(),
        "--data-binary".to_string(),
        "@-".to_string(),
        "--max-time".to_string(),
        "10".to_string(),
        job.webhook.clone(),
    ];
    let payload = serde_json::to_string(&body).unwrap_or_default();
    match std::process::Command::new(&bin)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(payload.as_bytes());
            }
            match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => session.log(
                    Level::Warn,
                    format!("webhook for '{}' failed with {status}", job.id),
                ),
                Err(e) => session.log(
                    Level::Warn,
                    format!("webhook for '{}' did not finish: {e}", job.id),
                ),
            }
        }
        Err(e) => session.log(
            Level::Warn,
            format!("webhook for '{}' could not run: {e}", job.id),
        ),
    }
}

fn job_notifications(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let id = p.str_arg("id")?;
    match p.str_opt("webhook") {
        None => {
            let job = session
                .jobs
                .get(&id)
                .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
            Ok(json!({ "id": id, "webhook": job.webhook, "then": job.then }))
        }
        Some(webhook) => {
            session
                .jobs
                .get_mut(&id)
                .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?
                .webhook = webhook;
            session.jobs.save()?;
            session.log(Level::Success, format!("webhook for '{id}' updated"));
            let webhook = session
                .jobs
                .get(&id)
                .map(|j| j.webhook.clone())
                .unwrap_or_default();
            Ok(json!({ "id": id, "webhook": webhook }))
        }
    }
}

fn job_run(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let id = p.str_arg("id")?;
    let started = std::time::Instant::now();
    let message = run_job(session, &id)?;
    Ok(json!({
        "job": id,
        "ok": true,
        "summary": message,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

fn job_run_due(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let now = crate::jobs::now();
    let due = session.jobs.ids_due(now);
    let mut results = Vec::new();
    for id in &due {
        let outcome = run_job(session, id);
        results.push(match outcome {
            Ok(message) => json!({ "job": id, "ok": true, "summary": message }),
            Err(e) => json!({ "job": id, "ok": false, "error": e.to_string() }),
        });
    }
    Ok(json!({ "at": now, "due": due.len(), "results": results }))
}

fn job_logs(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let limit = p.int_opt("limit", 20).clamp(1, 500) as usize;
    let id = p.str_opt("id");
    let entries: Vec<serde_json::Value> = session
        .jobs
        .recent_log(id.as_deref(), limit)
        .into_iter()
        .map(|e| {
            json!({
                "job": e.job, "at": e.at, "ok": e.ok, "message": e.message, "ms": e.ms,
            })
        })
        .collect();
    Ok(json!({ "entries": entries }))
}

/// Helper used by the UI and the CLI: a backup path inside the default backup directory.
pub fn default_backup_path(name: &str) -> PathBuf {
    crate::profiles::config_dir().join("backups").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_need_a_known_kind_and_schedule() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        let p =
            Params::from_json(json!({"id": "a", "name": "A", "kind": "nonsense"})).expect("params");
        assert!(job_create(&mut session, &p).is_err());
        let p = Params::from_json(
            json!({"id": "b", "name": "B", "kind": "backup", "schedule": "soon"}),
        )
        .expect("params");
        assert!(job_create(&mut session, &p).is_err());
    }

    #[test]
    fn creating_running_and_logging_a_query_job() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        let p = Params::from_json(json!({
            "id": "noop", "name": "Noop", "kind": "query",
            "params": {"sql": "select 1"}, "schedule": "manual"
        }))
        .expect("params");
        job_create(&mut session, &p).expect("create");
        let list = job_list(&mut session, &Params::none()).expect("list");
        let jobs = list["jobs"].as_array().cloned().unwrap_or_default();
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        assert_eq!(jobs[0]["id"], "noop");
        assert_eq!(jobs[0]["command"], "sql.run");
        // The job has no connection, so running it fails cleanly and is recorded as a failure.
        let e = run_job(&mut session, "noop");
        assert!(e.is_err());
        let job = session.jobs.get("noop").expect("job");
        assert_eq!(job.runs, 1);
        assert_eq!(job.failures, 1);
        let logs = job_logs(&mut session, &Params::none()).expect("logs");
        assert_eq!(logs["entries"].as_array().map(Vec::len), Some(1));
        let p = Params::from_json(json!({"id": "noop", "enabled": false})).expect("params");
        job_set_enabled(&mut session, &p).expect("disable");
        assert!(!session.jobs.get("noop").expect("job").enabled);
        assert_eq!(
            job_delete(
                &mut session,
                &Params::from_json(json!({"id": "noop"})).expect("params")
            )
            .expect("delete")["removed"],
            "Noop"
        );
    }

    #[test]
    fn transfer_and_sync_need_two_connections() {
        let mut session = Session::new();
        assert!(transfer_data(&mut session, &Params::none()).is_err());
        let p = Params::from_json(json!({"to": "nope"})).expect("params");
        assert!(pair(&session, &p).is_err());
        assert!(sync_data(&mut session, &Params::none()).is_err());
        assert!(diff_structure(&mut session, &Params::none()).is_err());
    }

    #[test]
    fn backing_up_a_closed_connection_is_an_error() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"conn": "missing"})).expect("params");
        assert!(dump(&mut session, &p).is_err());
        assert!(restore(&mut session, &p).is_err());
    }

    #[test]
    fn failing_job_posts_its_status_to_the_webhook() {
        let (_guard, log) = crate::test_support::fake_curl();
        let mut session = Session::new();
        let p = Params::from_json(json!({
            "id": "failing", "name": "Failing", "kind": "query",
            "params": {"sql": "select 1"},
            "webhook": "https://hooks.example.com/nook"
        }))
        .expect("params");
        job_create(&mut session, &p).expect("create");
        assert!(run_job(&mut session, "failing").is_err());
        let calls = std::fs::read_to_string(&log).expect("the fake curl ran");
        assert!(calls.contains("hooks.example.com/nook"), "{calls}");
        assert!(calls.contains("POST"), "{calls}");
        assert!(calls.contains("\"job\":\"failing\""), "{calls}");
        assert!(calls.contains("\"ok\":false"), "{calls}");
    }

    #[test]
    fn a_successful_job_runs_its_chain_and_a_failure_stops_it() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        for (id, sql) in [("first", "select 1"), ("second", "select 2"), ("third", "select 3")] {
            let p = Params::from_json(json!({
                "id": id, "name": id, "kind": "query",
                "params": {"sql": sql}
            }))
            .expect("params");
            job_create(&mut session, &p).expect("create");
        }
        // first -> second: `then` is part of job.create, so delete and recreate first with it.
        let p = Params::from_json(json!({"id": "first"})).expect("params");
        job_delete(&mut session, &p).expect("remove first");
        let p = Params::from_json(json!({
            "id": "first", "name": "first", "kind": "query",
            "params": {"sql": "select 1"}, "then": ["second"]
        }))
        .expect("params");
        job_create(&mut session, &p).expect("recreate with chain");
        // All three jobs have no connection, so every run fails: first's failure must stop the
        // chain before second ever runs.
        assert!(run_job(&mut session, "first").is_err());
        assert_eq!(session.jobs.get("first").expect("job").runs, 1);
        assert_eq!(session.jobs.get("second").expect("job").runs, 0);
        assert_eq!(session.jobs.get("third").expect("job").runs, 0);

        // A chain target that does not exist is rejected at create time.
        let p = Params::from_json(json!({
            "id": "bad", "name": "bad", "kind": "query",
            "params": {"sql": "select 1"}, "then": ["no-such-job"]
        }))
        .expect("params");
        assert!(job_create(&mut session, &p).is_err());
    }

    #[test]
    fn old_job_json_without_the_new_fields_loads() {
        let legacy = r#"{
            "id": "old", "name": "Old", "kind": "query",
            "conn": "", "target": "", "params": {},
            "schedule": {"kind": "manual"}, "enabled": true,
            "last_run": null, "last_status": null, "runs": 3, "failures": 1
        }"#;
        let job: crate::jobs::Job = serde_json::from_str(legacy).expect("legacy job loads");
        assert!(job.webhook.is_empty());
        assert!(job.then.is_empty());
    }

    #[test]
    fn listing_missing_backup_directory_is_not_an_error() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"dir": "/tmp/nook-no-such-dir"})).expect("params");
        let out = list_backups(&mut session, &p).expect("list");
        assert_eq!(out["files"].as_array().map(Vec::len), Some(0));
    }
}
