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
        CommandSpec::new("transfer.columnMapping", "Column mapping", "Transfer", transfer_mapping)
            .params(r#"{"map":[{"source":"mail","target":"email"}]}"#)
            .help("Normalises a mapping editor's rows into the columnMapping form transfer.data and the imports take."),
        CommandSpec::new("transfer.speedTest", "Time one page", "Transfer", speed_test)
            .params(r#"{"from":"local","to":"staging","table":"orders","rows":2000}"#)
            .help("Reads one page and writes it to a scratch table that is rolled back; reports rows/s and the batch size to pass to transfer.data."),
        CommandSpec::new("sync.dataDelete", "Delete rows missing from the source", "Sync", sync_data_delete)
            .params(r#"{"from":"prod","to":"staging","tables":["orders"],"delete_missing":true,"confirm":true,"dry_run":true}"#)
            .help("The delete half of the data sync, on its own. Dry run by default; confirm:true to apply."),
        CommandSpec::new("sync.conflictRules", "Conflict rules", "Sync", sync_conflict_rules)
            .params(r#"{"rules":[{"column":"total","on_conflict":"newer","by":"updated_at"}]}"#)
            .help("Validate and report per-column resolution: source, target, newer (by a timestamp) or skip."),
        CommandSpec::new("sync.bidirectional", "Bi-directional sync", "Sync", sync_bidirectional)
            .params(r#"{"a":"local","b":"staging","tables":["orders"],"confirm":true,"dry_run":true}"#)
            .help("Two passes with conflict rules deciding. Dry run by default; confirm:true to apply. A row changed on both sides is reported, never silently overwritten."),
        CommandSpec::new("backup.dump", "Backup", "Backup", dump)
            .params(r#"{"conn":"local","path":"~/backups/shop.sql","schema_only":false}"#)
            .help("SQLite writes a portable .sql script; Postgres and MySQL use pg_dump/mysqldump."),
        CommandSpec::new("backup.compression", "Compressed backups", "Backup", backup_compressed)
            .params(r#"{"conn":"local","path":"~/backups/shop.sql","compress":true}"#)
            .help("A dump piped through gzip, so the file is a normal .gz every other tool reads."),
        CommandSpec::new("backup.encryption", "Encrypted backups", "Backup", backup_encrypted)
            .params(r#"{"conn":"local","path":"~/backups/shop.sql","encrypt":"age","recipient":"age1…"}"#)
            .help("age or gpg; a missing binary is refused before any file is renamed."),
        CommandSpec::new("backup.local", "Local backup copy", "Backup", backup_local)
            .params(r#"{"conn":"local","path":"~/backups/shop.sql","keep":{"dir":"~/backups/offsite","copies":7}}"#)
            .help("Copies the finished dump to a second location and prunes older copies there."),
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
        CommandSpec::new("job.chain", "List or set a job's chain", "Automation", job_chain)
            .params(r#"{"id":"nightly","then":["followup"]}"#)
            .help("List a job's `then` chain, or set it (an empty array clears). Targets must exist and the chain must stay acyclic; `job.create` accepts the same field."),
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

/// Normalise a mapping editor's rows into `["src=target"]` entries.
///
/// The UI and the CLI both hold a list of pairs; `transfer.data` and the imports take the
/// `source=target` strings. One command does the conversion so the three callers cannot drift.
fn transfer_mapping(_session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let rows = p.get("map").and_then(|v| v.as_array()).ok_or_else(|| {
        Error::config("pass \"map\": [{\"source\":\"mail\",\"target\":\"email\"}]")
    })?;
    let mut entries = Vec::with_capacity(rows.len());
    let mut seen_targets: Vec<String> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let source = row
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::config(format!("map[{i}] has no \"source\"")))?;
        let target = row
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::config(format!("map[{i}] has no \"target\"")))?;
        if source.trim().is_empty() || target.trim().is_empty() {
            return Err(Error::config(format!(
                "map[{i}] has an empty source or target"
            )));
        }
        if seen_targets.iter().any(|t| t == target) {
            // Two sources into one target is not a mapping, it is a race for the column.
            return Err(Error::config(format!(
                "two sources map to '{target}'; a column takes one source"
            )));
        }
        seen_targets.push(target.to_string());
        entries.push(format!("{}={}", source.trim(), target.trim()));
    }
    Ok(json!({
        "columnMapping": entries,
        "count": entries.len(),
        "note": "pass this list as \"columnMapping\" to transfer.data or an import",
    }))
}

/// One timed page against the destination, rolled back.
fn speed_test(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (from, to) = pair(session, p)?;
    let table = p
        .str_opt("table")
        .ok_or_else(|| Error::config("pass \"table\""))?;
    let rows = p.int_opt("rows", 2_000).clamp(1, 50_000) as usize;
    let report = {
        let (src, dst) = session.two_drivers(from, to)?;
        transfer::speed_probe(src, dst, None, &table, rows)?
    };
    Ok(json!({
        "from": conn_id_at(session, from)?,
        "to": conn_id_at(session, to)?,
        "table": table,
        "rows": report.rows,
        "read_ms": report.read_ms,
        "write_ms": report.write_ms,
        "rows_per_second": report.rows_per_second.round(),
        "suggested_batch": report.suggested_batch,
        "note": report.note,
        "wrote": 0,
    }))
}

/// The delete half of the data sync, on its own.
///
/// Dry run by default and `confirm: true` to apply: a command whose whole job is deleting rows must
/// not delete them because someone forgot a flag.
fn sync_data_delete(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (from, to) = pair(session, p)?;
    let catalog = session.catalog_of(from)?.clone();
    let tables = tables_of(p);
    let dst_schema = p.str_opt("dst_schema");
    let max_rows = p.int_opt("max_rows", 5_000_000).clamp(1, 100_000_000) as usize;
    let dry_run = p.bool_opt("dry_run", true) || !p.bool_opt("confirm", false);
    let plan = {
        let (src, dst) = session.two_drivers(from, to)?;
        transfer::plan_rows(src, dst, &catalog, &tables, dst_schema.as_deref(), max_rows)?
    };
    let mut deletes: Vec<(String, Vec<(String, nook_core::Value)>)> = Vec::new();
    let mut skipped: Vec<serde_json::Value> = Vec::new();
    for diff in &plan {
        if let Some(reason) = &diff.skipped {
            skipped.push(json!({"table": diff.table, "reason": reason}));
            continue;
        }
        for change in &diff.changes {
            if change.kind == transfer::RowChangeKind::Delete {
                deletes.push((diff.table.clone(), change.key.clone()));
            }
        }
    }
    let mut applied = 0usize;
    if !dry_run {
        let dialect = {
            let driver = session.driver_for(to)?;
            driver.dialect()
        };
        let statements: Vec<String> = deletes
            .iter()
            .map(|(table, key)| {
                let where_clause = key
                    .iter()
                    .map(|(c, v)| {
                        if v.is_null() {
                            format!("{} IS NULL", nook_core::quote_ident(c, dialect))
                        } else {
                            format!(
                                "{} = {}",
                                nook_core::quote_ident(c, dialect),
                                nook_core::sql_literal(v, dialect)
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ");
                format!(
                    "DELETE FROM {} WHERE {where_clause};",
                    nook_core::quote_qualified(dst_schema.as_deref(), table, dialect)
                )
            })
            .collect();
        let driver = session.driver_for(to)?;
        driver.begin()?;
        for stmt in &statements {
            if let Err(e) = session.driver_for(to)?.run(stmt, 0) {
                let _ = session.driver_for(to).and_then(|d| d.rollback());
                return Err(e);
            }
            applied += 1;
        }
        session.driver_for(to)?.commit()?;
        let _ = crate::cmd::refresh_catalog(session, to);
    }
    Ok(json!({
        "from": conn_id_at(session, from)?,
        "to": conn_id_at(session, to)?,
        "deletes": deletes.len(),
        "applied": applied,
        "dry_run": dry_run,
        "skipped": skipped,
        "note": if dry_run {
            "dry run: nothing was deleted. Pass \"confirm\": true to apply"
        } else {
            "the rows missing from the source were deleted from the target"
        },
    }))
}

/// Parse and validate the per-column rules, and say what each one will do.
fn sync_conflict_rules(_session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let rules = conflict_rules(p)?;
    Ok(json!({
        "rules": rules.iter().map(|r| json!({
            "column": r.column,
            "on_conflict": match r.on_conflict {
                transfer::OnConflict::Source => "source",
                transfer::OnConflict::Target => "target",
                transfer::OnConflict::Newer => "newer",
                transfer::OnConflict::Skip => "skip",
            },
            "by": r.by,
        })).collect::<Vec<_>>(),
        "count": rules.len(),
        "note": "pass these to sync.bidirectional; sync.data and sync.dataDelete take a single \
                 direction and do not need them",
    }))
}

fn conflict_rules(p: &Params) -> Result<Vec<transfer::ConflictRule>> {
    let Some(rows) = p.get("rules").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let column = row
            .get("column")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::config(format!("rules[{i}] has no \"column\"")))?;
        let on_conflict = row
            .get("on_conflict")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::config(format!("rules[{i}] has no \"on_conflict\"")))?;
        let parsed = transfer::OnConflict::parse(on_conflict)?;
        let by = row.get("by").and_then(|v| v.as_str()).map(str::to_string);
        if parsed == transfer::OnConflict::Newer && by.is_none() {
            return Err(Error::config(format!(
                "rules[{i}] uses on_conflict: newer and needs \"by\": a timestamp column"
            )));
        }
        out.push(transfer::ConflictRule {
            column: column.to_string(),
            on_conflict: parsed,
            by,
        });
    }
    Ok(out)
}

/// Two passes, with the rules deciding. Never a silent last-writer-wins.
fn sync_bidirectional(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let a = match p.str_opt("a").or_else(|| p.str_opt("from")) {
        Some(id) => session.conn_index(&id)?,
        None => session
            .active_conn
            .ok_or_else(|| Error::config("pass \"a\" and \"b\""))?,
    };
    let b = match p.str_opt("b").or_else(|| p.str_opt("to")) {
        Some(id) => session.conn_index(&id)?,
        None => return Err(Error::config("pass \"b\" with the second connection")),
    };
    let tables = tables_of(p);
    let rules = conflict_rules(p)?;
    // Both halves of a sync write; both need the same gate. `dry_run: false` alone must never be
    // enough to change two databases.
    let dry_run = p.bool_opt("dry_run", true) || !p.bool_opt("confirm", false);
    let batch = p.int_opt("batch", 500).clamp(1, 20_000) as usize;
    let a_catalog = session.catalog_of(a)?.clone();
    let b_catalog = session.catalog_of(b)?.clone();
    let mut log: Vec<String> = Vec::new();
    let report = {
        let (da, db) = session.two_drivers(a, b)?;
        transfer::bidirectional(
            da,
            db,
            &a_catalog,
            &b_catalog,
            &transfer::BiPlan {
                tables: &tables,
                rules: &rules,
                batch,
                dry_run,
            },
            &mut |line: &str| log.push(line.to_string()),
        )?
    };
    if !dry_run {
        let _ = crate::cmd::refresh_catalog(session, a);
        let _ = crate::cmd::refresh_catalog(session, b);
    }
    Ok(json!({
        "a": conn_id_at(session, a)?,
        "b": conn_id_at(session, b)?,
        "summary": report.summary(),
        "a_to_b": report.a_to_b,
        "b_to_a": report.b_to_a,
        "conflicts": report.conflicts.iter().map(|c| json!({
            "key": c.key.iter().map(|(k, v)| json!({k: v.display()})).collect::<Vec<_>>(),
            "column": c.column,
            "source": c.source.display(),
            "target": c.target.display(),
            "reason": c.reason,
        })).collect::<Vec<_>>(),
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

/// The three backup ids are the same dump with a filter forced on.
///
/// They exist as their own ids because the parity catalog names them, and because a caller asking
/// for "an encrypted backup" should get an error when it cannot be encrypted rather than a plain
/// file with a `.age` name. Each delegates to `dump`, which owns the one dump path.
fn backup_compressed(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let mut params = p.raw().clone();
    params.insert("compress".into(), json!(true));
    dump(session, &Params(params))
}

fn backup_encrypted(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    if p.str_opt("encrypt").filter(|e| !e.is_empty()).is_none() {
        return Err(Error::config(
            "pass \"encrypt\": \"age\" or \"gpg\", and \"recipient\": the key the file is addressed to",
        ));
    }
    if p.str_opt("recipient").filter(|r| !r.is_empty()).is_none() {
        return Err(Error::config(
            "pass \"recipient\": the age or GPG key id the backup is encrypted to",
        ));
    }
    dump(session, p)
}

fn backup_local(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let keep = p.get("keep");
    if keep
        .and_then(|k| k.get("dir"))
        .and_then(|d| d.as_str())
        .is_none()
    {
        return Err(Error::config(
            "pass \"keep\": {\"dir\": \"~/backups/offsite\", \"copies\": 7}",
        ));
    }
    dump(session, p)
}

/// `compress`/`encrypt`/`keep` from the parameters.
fn backup_filters(p: &Params, home: &std::path::Path) -> Result<backup::Filters> {
    let keep = p.get("keep");
    let keep_dir = keep
        .and_then(|k| k.get("dir"))
        .and_then(|d| d.as_str())
        .map(|d| expand(d, home));
    let copies = keep
        .and_then(|k| k.get("copies"))
        .and_then(|c| c.as_u64())
        .map(|c| c as usize);
    let encrypt = p.str_opt("encrypt").filter(|e| !e.is_empty());
    Ok(backup::Filters {
        compress: p.bool_opt("compress", false),
        encrypt,
        recipient: p.str_opt("recipient"),
        keep_dir,
        copies,
    })
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
        // Not `settings.max_rows`: that is the grid's display cap. A backup that silently stopped
        // at a screenful would be a restore that loses rows. The limit is the caller's to set.
        let max_rows = p.int_opt("max_rows", 5_000_000).clamp(1, 100_000_000) as usize;
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
        let filters = backup_filters(p, &session.home)?;
        let final_path = if filters.is_empty() {
            path.clone()
        } else {
            backup::apply_filters(&path, &filters)?
        };
        let bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
        let truncated: Vec<String> = script
            .lines()
            .filter(|l| l.contains("truncated at"))
            .map(str::to_string)
            .collect();
        if !truncated.is_empty() {
            session.log(
                Level::Warn,
                format!(
                    "backup is incomplete: {} table(s) hit the {max_rows}-row limit",
                    truncated.len()
                ),
            );
        }
        session.log(
            Level::Success,
            format!("backup written: {}", final_path.display()),
        );
        return Ok(json!({
            "path": final_path.to_string_lossy(),
            "bytes": bytes,
            "truncated": truncated,
            "statements": nook_drivers::split_script(&script).len(),
            "engine": "in-process SQL script",
            "schema_only": schema_only,
            "compressed": filters.compress,
            "encrypted": filters.encrypt.clone(),
            "kept": filters.keep_dir.as_ref().map(|d| d.to_string_lossy().to_string()),
        }));
    }

    let plan = backup::backup_plan(&cfg, &tables, schema_only, &path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
    }
    let outcome = backup::execute(&plan, Some(&path))?;
    let filters = backup_filters(p, &session.home)?;
    let final_path = if filters.is_empty() {
        path.clone()
    } else {
        backup::apply_filters(&path, &filters)?
    };
    let bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    session.log(
        Level::Success,
        format!("backup written: {} ({bytes} bytes)", final_path.display()),
    );
    Ok(json!({
        "path": final_path.to_string_lossy(),
        "bytes": bytes,
        "command": outcome.command,
        "engine": kind.label(),
        "elapsed_ms": outcome.elapsed_ms,
        "compressed": filters.compress,
        "encrypted": filters.encrypt.clone(),
        "kept": filters.keep_dir.as_ref().map(|d| d.to_string_lossy().to_string()),
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
    let then = p.list("then");
    let candidate = crate::jobs::Job { then, ..job };
    // Validate before `add`: `add` persists, so a rejected chain must not leave the job on disk.
    // The candidate shadows any same-id job in the view the validator walks.
    let others: Vec<crate::jobs::Job> = session
        .jobs
        .ids()
        .into_iter()
        .filter(|other| other != &id)
        .filter_map(|other| session.jobs.get(&other).cloned())
        .collect();
    validate_chain(std::iter::once(&candidate).chain(others.iter()))?;
    session.jobs.add(candidate)?;
    session.log(Level::Success, format!("job '{id}' created"));
    job_list(session, p)
}

fn job_delete(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let id = p.str_arg("id")?;
    // Resolve to the canonical id first: `then` entries and lookups match id *or* name, so a
    // name-based delete must compare against the stored id, not the caller's raw string.
    let canonical = session
        .jobs
        .get(&id)
        .map(|j| j.id.clone())
        .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
    // A dangling `then` edge would only explode at run time, so refuse while referenced.
    if let Some(holder) = session
        .jobs
        .ids()
        .into_iter()
        .filter(|other| other != &canonical)
        .find(|other| {
            session
                .jobs
                .get(other)
                .is_some_and(|j| j.then.iter().any(|t| t == &canonical || t == &id))
        })
    {
        return Err(Error::config(format!(
            "job '{id}' is chained by '{holder}'; clear that chain first (job.chain)"
        )));
    }
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

/// Every `then` target must exist, and the chain graph must be acyclic — checked before the job
/// is stored. `jobs` is the post-save view: the candidate first, then every other job. A revisit
/// on the current DFS path is a cycle; a node finished via an earlier branch is fine, so legal
/// diamonds (A→B, A→C, B→D, C→D) pass.
fn validate_chain<'a>(jobs: impl IntoIterator<Item = &'a crate::jobs::Job>) -> Result<()> {
    let jobs: Vec<crate::jobs::Job> = jobs.into_iter().cloned().collect();
    fn walk(
        id: &str,
        jobs: &[crate::jobs::Job],
        path: &mut Vec<String>,
        done: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        let job = jobs
            .iter()
            .find(|j| j.id == id)
            .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
        if path.iter().any(|seen| seen == id) {
            return Err(Error::config(format!("job '{id}' would chain into itself")));
        }
        path.push(id.to_string());
        for next in &job.then {
            walk(next, jobs, path, done)?;
        }
        path.pop();
        done.insert(id.to_string());
        Ok(())
    }
    let mut path = Vec::new();
    let mut done = std::collections::HashSet::new();
    for job in &jobs {
        walk(&job.id, &jobs, &mut path, &mut done)?;
    }
    Ok(())
}

/// Run one job by dispatching its command with the job's parameters.
pub fn run_job(session: &mut Session, id: &str) -> Result<String> {
    run_job_tick(session, id, &mut Vec::new(), &mut None)
}

fn run_job_on_path(
    session: &mut Session,
    id: &str,
    path: &mut Vec<String>,
    ran: &mut Option<std::collections::HashSet<String>>,
) -> Result<String> {
    // Within one scheduler tick (`job.runDue`), a job runs at most once: the tick passes a
    // shared set, so a chain cannot re-run a job that already ran, and neither can the due
    // loop. Manual `job.run` passes None and re-runs freely.
    if let Some(set) = ran.as_mut() {
        if !set.insert(id.to_string()) {
            return Ok("skipped: already ran this tick".into());
        }
    }
    run_job_tick(session, id, path, ran)
}

fn run_job_tick(
    session: &mut Session,
    id: &str,
    path: &mut Vec<String>,
    ran: &mut Option<std::collections::HashSet<String>>,
) -> Result<String> {
    let job = session
        .jobs
        .get(id)
        .cloned()
        .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
    // Runtime guard: create-time validation is the fast path, but a hand-edited jobs.json can
    // still hold a cycle — stop here instead of recursing until the stack overflows.
    if path.iter().any(|seen| seen == id) {
        return Err(Error::config(format!("job '{id}' would chain into itself")));
    }
    path.push(id.to_string());
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
            // run is recorded in its own job's log by the recursive call. A chained child's
            // failure propagates as this run's error; the child fires its own webhook.
            let then = job.then.clone();
            for next in &then {
                if let Err(e) = run_job_on_path(session, next, path, ran) {
                    path.pop();
                    return Err(e);
                }
            }
            path.pop();
            Ok(message)
        }
        Err(e) => {
            session.jobs.record(&job.id, false, e.to_string(), ms, at);
            if !job.webhook.is_empty() {
                notify_webhook(&job, &e, ms, at, session);
            }
            path.pop();
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

fn job_chain(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let id = p.str_arg("id")?;
    let then = match p.get("then") {
        Some(_) => p.list("then"),
        None => {
            let job = session
                .jobs
                .get(&id)
                .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
            return Ok(json!({ "id": id, "then": job.then }));
        }
    };
    let candidate = crate::jobs::Job {
        then,
        ..session
            .jobs
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?
    };
    let others: Vec<crate::jobs::Job> = session
        .jobs
        .ids()
        .into_iter()
        .filter(|other| other != &id)
        .filter_map(|other| session.jobs.get(&other).cloned())
        .collect();
    validate_chain(std::iter::once(&candidate).chain(others.iter()))?;
    session
        .jobs
        .get_mut(&id)
        .ok_or_else(|| Error::internal("the job vanished mid-command"))?
        .then = candidate.then.clone();
    session.jobs.save()?;
    session.log(Level::Success, format!("chain for '{id}' updated"));
    Ok(json!({ "id": id, "then": candidate.then }))
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
    let mut ran: Option<std::collections::HashSet<String>> = Some(Default::default());
    for id in &due {
        let outcome = run_job_on_path(session, id, &mut Vec::new(), &mut ran);
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

    fn live_session() -> (std::sync::MutexGuard<'static, ()>, Session) {
        let guard = crate::test_support::scratch_config();
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
        (guard, session)
    }

    #[test]
    fn a_successful_job_runs_its_chain_in_order_and_a_midchain_failure_stops_it() {
        let (_guard, mut session) = live_session();
        let mk = |session: &mut Session, id: &str, sql: &str, then: &[&str]| {
            let p = Params::from_json(json!({
                "id": id, "name": id, "kind": "query",
                "params": {"sql": sql}, "then": then
            }))
            .expect("params");
            job_create(session, &p).expect("create");
        };
        mk(&mut session, "third", "select 3", &[]);
        mk(&mut session, "second", "select 2", &["third"]);
        mk(&mut session, "first", "select 1", &["second"]);
        // Success path: first -> second -> third all run, in order, each recorded once.
        run_job(&mut session, "first").expect("chain runs");
        for id in ["first", "second", "third"] {
            assert_eq!(session.jobs.get(id).expect("job").runs, 1, "{id} ran");
            assert_eq!(
                session.jobs.get(id).expect("job").last_status,
                Some("ok".to_string()),
                "{id} succeeded"
            );
        }

        // Mid-chain failure: broken -> mid fails, and mid's own chain (tail) must never run.
        // A job whose SQL fails does not run its chain at all, so the failure has to sit in
        // the middle: broken succeeds, mid's SQL fails, tail stays at zero runs.
        mk(&mut session, "tail", "select 1", &[]);
        mk(&mut session, "mid", "select no_such_column", &["tail"]);
        mk(&mut session, "broken", "select 1", &["mid"]);
        assert!(run_job(&mut session, "broken").is_err());
        assert_eq!(session.jobs.get("broken").expect("job").runs, 1);
        assert_eq!(session.jobs.get("mid").expect("job").runs, 1);
        assert_eq!(
            session.jobs.get("tail").expect("job").runs,
            0,
            "tail must not run"
        );

        // A chain target that does not exist is rejected at create time — and, crucially,
        // the rejected job is not left in the store.
        let p = Params::from_json(json!({
            "id": "dangling", "name": "dangling", "kind": "query",
            "params": {"sql": "select 1"}, "then": ["no-such-job"]
        }))
        .expect("params");
        assert!(job_create(&mut session, &p).is_err());
        assert!(session.jobs.get("dangling").is_none());
    }

    #[test]
    fn a_hand_edited_cycle_is_stopped_at_run_time_not_by_overflow() {
        let (_guard, mut session) = live_session();
        let mk = |session: &mut Session, id: &str| {
            let p = Params::from_json(json!({
                "id": id, "name": id, "kind": "query",
                "params": {"sql": "select 1"}
            }))
            .expect("params");
            job_create(session, &p).expect("create");
        };
        mk(&mut session, "a");
        mk(&mut session, "b");
        // Write the cycle straight into the store, bypassing create-time validation — the
        // same shape a hand-edited jobs.json loads as. The runtime path guard must stop it.
        session.jobs.get_mut("a").expect("job a").then = vec!["b".into()];
        session.jobs.get_mut("b").expect("job b").then = vec!["a".into()];
        let err = run_job(&mut session, "a").expect_err("the cycle must error");
        assert!(
            err.to_string().contains("chain into itself"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn a_chain_diamond_is_legal_but_cycles_and_referenced_deletes_are_not() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        let mk = |session: &mut Session, id: &str, then: &[&str]| {
            let p = Params::from_json(json!({
                "id": id, "name": id, "kind": "query",
                "params": {"sql": "select 1"}, "then": then
            }))
            .expect("params");
            job_create(session, &p).expect("create");
        };
        mk(&mut session, "d", &[]);
        mk(&mut session, "b", &["d"]);
        mk(&mut session, "c", &["d"]);
        mk(&mut session, "a", &["b", "c"]); // diamond: d reached twice, still acyclic
        mk(&mut session, "a2", &["b"]);

        // A referenced job cannot be deleted while someone chains into it.
        let p = Params::from_json(json!({"id": "d"})).expect("params");
        assert!(job_delete(&mut session, &p).is_err());
        // Also by display name: lookups match id or name, so the guard must resolve the
        // caller's string to the stored id before comparing `then` entries. Give d an alias
        // by recreating it with a display name, then delete under that name.
        let p = Params::from_json(json!({"id": "d"})).expect("params");
        job_delete(&mut session, &p).expect_err("referenced");
        let holder = session.jobs.get_mut("b").expect("job b");
        holder.then = vec![];
        session.jobs.save().expect("save");
        let p = Params::from_json(json!({
            "id": "d2", "name": "Dee", "kind": "query",
            "params": {"sql": "select 1"}, "then": []
        }))
        .expect("params");
        job_create(&mut session, &p).expect("create named");
        let p = Params::from_json(json!({"id": "c", "then": ["d2"]})).expect("params");
        job_chain(&mut session, &p).expect("chain c to d2");
        let p = Params::from_json(json!({"id": "Dee"})).expect("params");
        assert!(
            job_delete(&mut session, &p).is_err(),
            "name delete must be guarded too"
        );
        let p = Params::from_json(json!({"id": "b", "then": []})).expect("params");
        let out = job_chain(&mut session, &p).expect("clear chain");
        assert_eq!(out["then"].as_array().map(Vec::len), Some(0));
        let p = Params::from_json(json!({"id": "c", "then": []})).expect("params");
        job_chain(&mut session, &p).expect("clear c too — it also chains into d");
        let p = Params::from_json(json!({"id": "d"})).expect("params");
        assert!(job_delete(&mut session, &p).is_ok());

        // job.chain lists, sets a valid chain, and rejects a cycle.
        let p = Params::from_json(json!({"id": "a2"})).expect("params");
        let out = job_chain(&mut session, &p).expect("list chain");
        assert_eq!(out["then"], json!(["b"]));
        let p = Params::from_json(json!({"id": "a2", "then": ["c"]})).expect("params");
        job_chain(&mut session, &p).expect("set chain");
        // c -> a2 while a2 -> c is a real two-node cycle, unlike a -> a2 which is just an edge.
        let p = Params::from_json(json!({"id": "c", "then": ["a2"]})).expect("params");
        assert!(job_chain(&mut session, &p).is_err());
    }

    #[test]
    fn a_due_chained_child_is_not_run_twice_in_one_tick() {
        let (_guard, mut session) = live_session();
        let mk = |session: &mut Session, id: &str, then: &[&str]| {
            let p = Params::from_json(json!({
                "id": id, "name": id, "kind": "query",
                "params": {"sql": "select 1"}, "then": then, "schedule": "every 1m"
            }))
            .expect("params");
            job_create(session, &p).expect("create");
        };
        mk(&mut session, "child", &[]);
        mk(&mut session, "parent", &["child"]);
        // Both are due; running parent runs child via the chain, so the tick must not run
        // child a second time as its own due entry.
        let out = job_run_due(&mut session, &Params::none()).expect("run due");
        assert_eq!(
            out["results"].as_array().map(Vec::len),
            Some(2),
            "both jobs stay in the report: {out}"
        );
        assert_eq!(
            session.jobs.get("child").expect("job").runs,
            1,
            "no double run"
        );
        assert_eq!(session.jobs.get("parent").expect("job").runs, 1);
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
