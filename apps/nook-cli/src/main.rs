#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! `nook-cli`: the same commands the app runs, from a shell.
//!
//! Connections are named by saved profile id or given as a URI
//! (`sqlite:./app.db`, `postgres://user:pass@host:5432/db`, `mysql://…`).

use std::io::Write;

use nook_core::{ConnectionConfig, Error, Result, Value};
use nook_engine::{registry, Conn, Level, Session};
use serde_json::json;

type CliResult = std::result::Result<(), String>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("nook: {message}");
            1
        }
    };
    std::process::exit(code);
}

fn usage() -> String {
    r#"nook-cli — Nook from the shell

  commands [--group G] [--json]          every command id with its parameters
  parity [--markdown]                    feature coverage against the Navicat catalog
  profiles                               saved connections
  sql <conn> "<SQL>" [--format F]        run SQL (table, json, csv, markdown)
  script <conn> --file f.sql             run a .sql file
  tables <conn> [--schema S]             objects in a connection
  describe <conn> <table>                columns, indexes, foreign keys
  ddl <conn> <table>                     the CREATE statement
  dump <conn> --out f.sql [--tables a,b] [--schema-only]
  restore <conn> --file f.sql
  transfer --from A --to B [--tables a,b] [--create] [--truncate] [--drop]
  sync-data --from A --to B [--tables a,b] [--apply] [--batch N]
  diff-structure --from A --to B [--apply] [--allow-destructive]
  datagen <conn> <table> [--rows N] [--seed N] [--override col=spec ...] [--apply] [--out f.sql]
  import <conn> --file x.csv --table t [--no-create] [--delimiter C]
  export <conn> "<SQL>" --out f.csv [--format F]
  monitor <conn> [--processes] [--status] [--variables] [--filter S]
  users <conn> [--grants USER]
  diagram <conn> --out schema.svg [--schema S]
  chart <conn> "<SQL>" [--chart KIND] [--label COL]
  ai <conn> "<prompt>" [--tables a,b] [--json]   draft SQL with an AI provider
  ai providers [--json]                  providers, their endpoints and whether a key is set
  ai key <provider> [--remove]           save a key from stdin, or forget the stored one
  job list | run <id> | serve [--interval S]
  mcp                                    MCP server on stdio
  help

A connection is a saved profile name, or a URI such as sqlite:./app.db,
postgres://user:pass@127.0.0.1:5432/shop, mysql://root@127.0.0.1/shop.
"#
    .to_string()
}

fn run(args: &[String]) -> CliResult {
    let command = args.first().map(String::as_str).unwrap_or("help");
    match command {
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            Ok(())
        }
        "commands" => commands(args),
        "parity" => parity(args),
        "profiles" => profiles(),
        "sql" => sql(args),
        "script" => script(args),
        "tables" => tables(args),
        "describe" => describe(args),
        "ddl" => ddl(args),
        "dump" => dump(args),
        "restore" => restore(args),
        "transfer" => transfer(args),
        "sync-data" => sync_data(args),
        "diff-structure" => diff_structure(args),
        "datagen" => datagen(args),
        "import" => import(args),
        "export" => export(args),
        "monitor" => monitor(args),
        "users" => users(args),
        "diagram" => diagram(args),
        "chart" => chart(args),
        "ai" => ai(args),
        "job" => job(args),
        "mcp" => nook_mcp::serve_stdio(usage()).map_err(|e| e.to_string()),
        other => Err(format!("unknown command '{other}'\n\n{}", usage())),
    }
}

// ------------------------------------------------------------------ argument helpers

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            return it.next().cloned();
        }
        if let Some(rest) = arg.strip_prefix(&format!("{name}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn opt_all(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            if let Some(value) = it.next() {
                out.push(value.clone());
            }
        } else if let Some(rest) = arg.strip_prefix(&format!("{name}=")) {
            out.push(rest.to_string());
        }
    }
    out
}

/// Positional arguments, ignoring flags and their values.
fn positional(args: &[String], skip: usize) -> Vec<String> {
    let value_flags = [
        "--format",
        "--schema",
        "--out",
        "--file",
        "--tables",
        "--limit",
        "--rows",
        "--seed",
        "--override",
        "--table",
        "--delimiter",
        "--from",
        "--to",
        "--batch",
        "--label",
        "--chart",
        "--filter",
        "--grants",
        "--group",
        "--interval",
    ];
    let mut out = Vec::new();
    let mut it = args.iter().skip(skip);
    while let Some(arg) = it.next() {
        if arg.starts_with("--") {
            if value_flags.contains(&arg.as_str()) {
                let _ = it.next();
            }
            if arg.contains('=') {
                continue;
            }
            continue;
        }
        out.push(arg.clone());
    }
    out
}

// ------------------------------------------------------------------ session plumbing

/// Open a connection: a saved profile id, or a URI.
fn open_connection(session: &mut Session, spec: &str) -> Result<usize> {
    let cfg = match session.profiles.get(spec) {
        Some(saved) => saved.clone(),
        None => ConnectionConfig::parse(spec).map_err(Error::config)?,
    };
    let id = cfg.id.clone();
    if session.conn_index(&id).is_err() {
        session.conns.push(Conn::new(cfg));
    }
    let index = session.conn_index(&id)?;
    nook_engine::cmd::open_now(session, index)?;
    Ok(index)
}

fn run_command(
    session: &mut Session,
    id: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    session.run(id, params)
}

// ------------------------------------------------------------------ presentation

fn print_result(rows: &nook_model::ResultSet, format: &str) -> CliResult {
    match format {
        "json" => {
            let text =
                serde_json::to_string_pretty(&rows.to_json()).map_err(|e| format!("json: {e}"))?;
            println!("{text}");
        }
        "csv" => print!("{}", rows.to_csv()),
        "markdown" | "md" => print!("{}", rows.to_markdown()),
        _ => print_table(rows),
    }
    Ok(())
}

fn print_table(rows: &nook_model::ResultSet) {
    if !rows.is_tabular() {
        println!(
            "{} row(s) affected in {:.1} ms",
            rows.affected, rows.elapsed_ms
        );
        return;
    }
    let mut widths: Vec<usize> = rows.columns.iter().map(|c| c.chars().count()).collect();
    let cell = |v: &Value| {
        let text = v.display();
        if text.chars().count() > 60 {
            format!("{}…", text.chars().take(59).collect::<String>())
        } else {
            text
        }
    };
    let rendered: Vec<Vec<String>> = rows
        .rows
        .iter()
        .map(|row| {
            (0..rows.columns.len())
                .map(|i| cell(row.get(i).unwrap_or(&Value::Null)))
                .collect()
        })
        .collect();
    for row in &rendered {
        for (i, text) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(i) {
                *width = (*width).max(text.chars().count());
            }
        }
    }
    let line = |row: &[String]| {
        let mut out = String::new();
        for (i, text) in row.iter().enumerate() {
            let width = widths.get(i).copied().unwrap_or(0);
            out.push_str(&format!("{text:<width$}  ", width = width));
        }
        println!("{}", out.trim_end());
    };
    line(&rows.columns.clone());
    let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
    line(&rule);
    for row in &rendered {
        line(row);
    }
    println!(
        "{} row(s) in {:.1} ms{}",
        rows.row_count(),
        rows.elapsed_ms,
        if rows.truncated { " (truncated)" } else { "" }
    );
}

fn print_json(value: &serde_json::Value) -> CliResult {
    let text = serde_json::to_string_pretty(value).map_err(|e| format!("json: {e}"))?;
    println!("{text}");
    Ok(())
}

// ------------------------------------------------------------------ commands

fn commands(args: &[String]) -> CliResult {
    let group = opt(args, "--group");
    let json_out = flag(args, "--json");
    let list: Vec<&nook_engine::CommandSpec> = registry()
        .iter()
        .filter(|c| group.as_deref().map(|g| c.group == g).unwrap_or(true))
        .collect();
    if json_out {
        let value = serde_json::json!({
            "count": list.len(),
            "commands": list.iter().map(|c| serde_json::json!({
                "id": c.id, "label": c.label, "group": c.group,
                "shortcut": c.shortcut, "params": c.params,
            })).collect::<Vec<_>>()
        });
        return print_json(&value);
    }
    let mut current = "";
    for spec in list {
        if spec.group != current {
            current = spec.group;
            println!("\n{current}");
        }
        let shortcut = if spec.shortcut.is_empty() {
            String::new()
        } else {
            format!("  [{}]", spec.shortcut)
        };
        println!("  {:<26} {}{}", spec.id, spec.label, shortcut);
        if !spec.params.is_empty() {
            println!("    {}", spec.params);
        }
    }
    println!("\n{} commands", registry().len());
    Ok(())
}

fn parity(args: &[String]) -> CliResult {
    let report = nook_engine::parity::measure();
    if flag(args, "--markdown") {
        print!("{}", report.markdown());
        return Ok(());
    }
    println!("{}", report.summary());
    println!("\n{:<16} {:>7} {:>7}   coverage", "area", "have", "total");
    for area in &report.by_area {
        println!(
            "{:<16} {:>7} {:>7}   {:.0}%",
            area.area,
            area.covered,
            area.total,
            area.percent()
        );
    }
    println!("\nMissing ({}):", report.missing.len());
    for (area, ids) in report.missing_by_area() {
        println!("  {area}: {}", ids.join(", "));
    }
    Ok(())
}

fn profiles() -> CliResult {
    let store = nook_engine::ProfileStore::load().map_err(|e| e.to_string())?;
    println!("config: {}", nook_engine::profiles::config_dir().display());
    for cfg in store.list() {
        println!(
            "  {:<20} {:<10} {:<10} {}{}",
            cfg.id,
            cfg.kind.label(),
            cfg.name,
            cfg.display_uri(),
            if cfg.ssh.enabled { "  (ssh)" } else { "" }
        );
    }
    println!("{} profile(s)", store.list().len());
    Ok(())
}

fn sql(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals
        .first()
        .ok_or_else(|| format!("pass a connection\n\n{}", usage()))?;
    let statement = positionals
        .get(1)
        .ok_or_else(|| "pass SQL as the second argument".to_string())?;
    let format = opt(args, "--format").unwrap_or_else(|| "table".into());
    let limit = opt(args, "--limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(0);
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("sql".into(), json!(statement));
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    if limit > 0 {
        params.insert("limit".into(), json!(limit));
    }
    let out = run_command(&mut session, "sql.run", serde_json::Value::Object(params))
        .map_err(|e| e.to_string())?;
    let rows = nook_model::ResultSet {
        columns: string_list(&out["columns"]),
        column_types: string_list(&out["types"]),
        rows: json_rows(&out["data"], string_list(&out["columns"]).len()),
        affected: out["affected"].as_u64().unwrap_or(0),
        elapsed_ms: out["elapsed_ms"].as_f64().unwrap_or(0.0),
        truncated: out["truncated"].as_bool().unwrap_or(false),
        ..nook_model::ResultSet::default()
    };
    print_result(&rows, &format)?;
    for message in &session.messages {
        if message.level == Level::Warn {
            eprintln!("note: {}", message.text);
        }
    }
    Ok(())
}

fn string_list(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn json_rows(value: &serde_json::Value, width: usize) -> Vec<Vec<Value>> {
    value
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    (0..width)
                        .map(|i| match row.get(i) {
                            Some(v) => Value::from_json(v),
                            None => Value::Null,
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn script(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let file = opt(args, "--file").ok_or("pass --file script.sql")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("read {file}: {e}"))?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let statements = nook_drivers::split_script(&text);
    let conn_id = session.conns[index].cfg.id.clone();
    let out = run_command(
        &mut session,
        "sql.run",
        json!({ "sql": text, "conn": conn_id }),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{} statement(s), {} row(s), {} affected in {:.1} ms",
        statements.len(),
        out["rows"].as_u64().unwrap_or(0),
        out["affected"].as_u64().unwrap_or(0),
        out["elapsed_ms"].as_f64().unwrap_or(0.0)
    );
    Ok(())
}

fn tables(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    if let Some(schema) = opt(args, "--schema") {
        params.insert("schema".into(), json!(schema));
    }
    let out = run_command(
        &mut session,
        "schema.tables",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    if flag(args, "--json") {
        return print_json(&out);
    }
    println!(
        "{:<40} {:<10} {:>7} {:>8}",
        "table", "kind", "columns", "rows"
    );
    for table in out["tables"].as_array().cloned().unwrap_or_default() {
        println!(
            "{:<40} {:<10} {:>7} {:>8}",
            table["key"].as_str().unwrap_or(""),
            table["kind"].as_str().unwrap_or(""),
            table["columns"].as_u64().unwrap_or(0),
            table["rows"]
                .as_i64()
                .map(|r| r.to_string())
                .unwrap_or_else(|| "-".into()),
        );
    }
    Ok(())
}

fn describe(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let table = positionals.get(1).ok_or("pass a table")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let conn_id = session.conns[index].cfg.id.clone();
    let columns = run_command(
        &mut session,
        "schema.columns",
        json!({ "conn": conn_id, "table": table }),
    )
    .map_err(|e| e.to_string())?;
    let indexes = run_command(
        &mut session,
        "schema.indexes",
        json!({ "conn": conn_id, "table": table }),
    )
    .map_err(|e| e.to_string())?;
    let fks = run_command(
        &mut session,
        "schema.foreignKeys",
        json!({ "conn": conn_id, "table": table }),
    )
    .map_err(|e| e.to_string())?;
    println!("{}", columns["table"].as_str().unwrap_or(table));
    println!(
        "primary key: {}",
        columns["primary_key"]
            .as_array()
            .map(|k| k
                .iter()
                .map(|v| v.as_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    );
    println!(
        "\n{:<24} {:<20} {:>8} {:>6}  default",
        "column", "type", "nullable", "pk"
    );
    for column in columns["columns"].as_array().cloned().unwrap_or_default() {
        println!(
            "{:<24} {:<20} {:>8} {:>6}  {}",
            column["name"].as_str().unwrap_or(""),
            column["type"].as_str().unwrap_or(""),
            if column["nullable"].as_bool().unwrap_or(true) {
                "yes"
            } else {
                "no"
            },
            if column["primary_key"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                ""
            },
            column["default"].as_str().unwrap_or(""),
        );
    }
    for index_info in indexes["indexes"].as_array().cloned().unwrap_or_default() {
        println!(
            "index {} ({}){}{}",
            index_info["name"].as_str().unwrap_or(""),
            index_info["columns"]
                .as_array()
                .map(|c| c
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default(),
            if index_info["unique"].as_bool().unwrap_or(false) {
                " unique"
            } else {
                ""
            },
            if index_info["primary"].as_bool().unwrap_or(false) {
                " primary"
            } else {
                ""
            },
        );
    }
    for fk in fks["foreign_keys"].as_array().cloned().unwrap_or_default() {
        let joined = |v: &serde_json::Value| {
            v.as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|i| i.as_str().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        };
        println!(
            "foreign key {}: ({}) → {}({}){}",
            fk["name"].as_str().unwrap_or(""),
            joined(&fk["columns"]),
            fk["ref_table"].as_str().unwrap_or(""),
            joined(&fk["ref_columns"]),
            match fk["on_delete"].as_str() {
                Some(rule) if !rule.is_empty() && !rule.eq_ignore_ascii_case("no action") =>
                    format!(" on delete {}", rule.to_uppercase()),
                _ => String::new(),
            }
        );
    }
    Ok(())
}

fn ddl(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let table = positionals.get(1).ok_or("pass a table")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let conn_id = session.conns[index].cfg.id.clone();
    let out = run_command(
        &mut session,
        "schema.ddl",
        json!({ "conn": conn_id, "table": table }),
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out["ddl"].as_str().unwrap_or(""));
    Ok(())
}

fn dump(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let out_path = opt(args, "--out").ok_or("pass --out file.sql")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("path".into(), json!(out_path));
    if let Some(tables) = opt(args, "--tables") {
        params.insert(
            "tables".into(),
            json!(tables.split(',').map(|s| s.trim()).collect::<Vec<_>>()),
        );
    }
    if flag(args, "--schema-only") {
        params.insert("schema_only".into(), json!(true));
    }
    let out = run_command(
        &mut session,
        "backup.dump",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} bytes) via {}",
        out["path"].as_str().unwrap_or(""),
        out["bytes"].as_u64().unwrap_or(0),
        out["engine"].as_str().unwrap_or("")
    );
    if let Some(command) = out["command"].as_str() {
        println!("command: {command}");
    }
    Ok(())
}

fn restore(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let file = opt(args, "--file").ok_or("pass --file backup.sql")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let conn_id = session.conns[index].cfg.id.clone();
    let out = run_command(
        &mut session,
        "backup.restore",
        json!({ "conn": conn_id, "path": file }),
    )
    .map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    Ok(())
}

fn transfer(args: &[String]) -> CliResult {
    let from = opt(args, "--from").ok_or("pass --from SOURCE")?;
    let to = opt(args, "--to").ok_or("pass --to TARGET")?;
    let mut session = Session::new();
    let from_index = open_connection(&mut session, &from).map_err(|e| e.to_string())?;
    let to_index = open_connection(&mut session, &to).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("from".into(), json!(session.conns[from_index].cfg.id));
    params.insert("to".into(), json!(session.conns[to_index].cfg.id));
    if let Some(tables) = opt(args, "--tables") {
        params.insert(
            "tables".into(),
            json!(tables.split(',').map(|s| s.trim()).collect::<Vec<_>>()),
        );
    }
    params.insert("create_tables".into(), json!(!flag(args, "--no-create")));
    let _: () = ();
    params.insert("truncate_target".into(), json!(flag(args, "--truncate")));
    params.insert("drop_target".into(), json!(flag(args, "--drop")));
    if flag(args, "--preview") {
        let out = run_command(
            &mut session,
            "transfer.preview",
            serde_json::Value::Object(params),
        )
        .map_err(|e| e.to_string())?;
        return print_json(&out);
    }
    let out = run_command(
        &mut session,
        "transfer.data",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out["summary"].as_str().unwrap_or(""));
    for table in out["tables"].as_array().cloned().unwrap_or_default() {
        println!(
            "  {:<32} {:>8} rows {:>6} statements{}",
            table["table"].as_str().unwrap_or(""),
            table["rows"].as_u64().unwrap_or(0),
            table["statements"].as_u64().unwrap_or(0),
            match table["error"].as_str() {
                Some(e) => format!("  failed: {e}"),
                None => String::new(),
            }
        );
    }
    Ok(())
}

fn sync_data(args: &[String]) -> CliResult {
    let from = opt(args, "--from").ok_or("pass --from SOURCE")?;
    let to = opt(args, "--to").ok_or("pass --to TARGET")?;
    let mut session = Session::new();
    let from_index = open_connection(&mut session, &from).map_err(|e| e.to_string())?;
    let to_index = open_connection(&mut session, &to).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("from".into(), json!(session.conns[from_index].cfg.id));
    params.insert("to".into(), json!(session.conns[to_index].cfg.id));
    params.insert("dry_run".into(), json!(!flag(args, "--apply")));
    if let Some(tables) = opt(args, "--tables") {
        params.insert(
            "tables".into(),
            json!(tables.split(',').map(|s| s.trim()).collect::<Vec<_>>()),
        );
    }
    if let Some(batch) = opt(args, "--batch").and_then(|b| b.parse::<i64>().ok()) {
        params.insert("batch".into(), json!(batch));
    }
    let out = run_command(&mut session, "sync.data", serde_json::Value::Object(params))
        .map_err(|e| e.to_string())?;
    println!("{}", out["summary"].as_str().unwrap_or(""));
    for line in out["log"].as_array().cloned().unwrap_or_default() {
        println!("  {}", line.as_str().unwrap_or(""));
    }
    Ok(())
}

fn diff_structure(args: &[String]) -> CliResult {
    let from = opt(args, "--from").ok_or("pass --from SOURCE")?;
    let to = opt(args, "--to").ok_or("pass --to TARGET")?;
    let mut session = Session::new();
    let from_index = open_connection(&mut session, &from).map_err(|e| e.to_string())?;
    let to_index = open_connection(&mut session, &to).map_err(|e| e.to_string())?;
    let command = if flag(args, "--apply") {
        "sync.structure"
    } else {
        "diff.structure"
    };
    let from_id = session.conns[from_index].cfg.id.clone();
    let to_id = session.conns[to_index].cfg.id.clone();
    let out = run_command(
        &mut session,
        command,
        json!({
            "from": from_id,
            "to": to_id,
            "apply": flag(args, "--apply"),
            "allow_destructive": flag(args, "--allow-destructive"),
        }),
    )
    .map_err(|e| e.to_string())?;
    if let Some(applied) = out["applied"].as_array() {
        for change in applied {
            println!(
                "applied {}: {}",
                change["object"].as_str().unwrap_or(""),
                change["sql"].as_str().unwrap_or("")
            );
        }
        for change in out["skipped"].as_array().cloned().unwrap_or_default() {
            println!(
                "skipped {}: {}",
                change["object"].as_str().unwrap_or(""),
                change["reason"].as_str().unwrap_or("")
            );
        }
        return Ok(());
    }
    println!("{} change(s)", out["count"].as_u64().unwrap_or(0));
    for change in out["changes"].as_array().cloned().unwrap_or_default() {
        println!(
            "  [{}] {}{}{}",
            change["kind"].as_str().unwrap_or(""),
            change["object"].as_str().unwrap_or(""),
            if change["destructive"].as_bool().unwrap_or(false) {
                "  destructive"
            } else {
                ""
            },
            if change["manual"].as_bool().unwrap_or(false) {
                "  manual"
            } else {
                ""
            },
        );
        if let Some(sql) = change["sql"].as_str() {
            if !sql.is_empty() {
                println!("      {sql}");
            }
        }
        if let Some(note) = change["note"].as_str() {
            if !note.is_empty() {
                println!("      note: {note}");
            }
        }
    }
    Ok(())
}

fn datagen(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let table = positionals.get(1).ok_or("pass a table")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("table".into(), json!(table));
    params.insert(
        "rows".into(),
        json!(opt(args, "--rows")
            .and_then(|r| r.parse::<i64>().ok())
            .unwrap_or(100)),
    );
    if let Some(seed) = opt(args, "--seed").and_then(|s| s.parse::<i64>().ok()) {
        params.insert("seed".into(), json!(seed));
    }
    if flag(args, "--apply") {
        params.insert("apply".into(), json!(true));
    }
    if let Some(out) = opt(args, "--out") {
        params.insert("out".into(), json!(out));
    }
    let overrides = opt_all(args, "--override");
    if !overrides.is_empty() {
        let mut map = serde_json::Map::new();
        for entry in overrides {
            match entry.split_once('=') {
                Some((column, spec)) => {
                    map.insert(column.to_string(), json!(spec));
                }
                None => return Err(format!("--override needs column=spec, got '{entry}'")),
            }
        }
        params.insert("overrides".into(), serde_json::Value::Object(map));
    }
    let out = run_command(
        &mut session,
        "tools.datagen",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{} rows for {} (seed {}){}",
        out["rows"].as_u64().unwrap_or(0),
        out["table"].as_str().unwrap_or(""),
        out["seed"].as_u64().unwrap_or(0),
        match out["path"].as_str() {
            Some(p) => format!(" → {p}"),
            None => String::new(),
        }
    );
    if let Some(inserted) = out["inserted"].as_u64() {
        println!("inserted {inserted} rows");
    }
    print_json(&out["preview"])
}

fn import(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let file = opt(args, "--file").ok_or("pass --file data.csv")?;
    let table = opt(args, "--table").ok_or("pass --table target")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("path".into(), json!(file));
    params.insert("table".into(), json!(table));
    params.insert("create_table".into(), json!(!flag(args, "--no-create")));
    if let Some(delimiter) = opt(args, "--delimiter") {
        params.insert("delimiter".into(), json!(delimiter));
    }
    let command = if file.to_ascii_lowercase().ends_with(".json") {
        "tools.importJson"
    } else {
        "tools.importCsv"
    };
    let out = run_command(&mut session, command, serde_json::Value::Object(params))
        .map_err(|e| e.to_string())?;
    println!(
        "imported {} rows into {} ({}{})",
        out["inserted"].as_u64().unwrap_or(0),
        out["table"].as_str().unwrap_or(""),
        out["columns"].as_array().map(|c| c.len()).unwrap_or(0),
        if out["created_table"].as_bool().unwrap_or(false) {
            " columns, table created"
        } else {
            " columns"
        }
    );
    Ok(())
}

fn export(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let statement = positionals.get(1).ok_or("pass SQL")?;
    let out_path = opt(args, "--out").ok_or("pass --out file.csv")?;
    let format = opt(args, "--format").unwrap_or_else(|| {
        if out_path.ends_with(".json") {
            "json".into()
        } else if out_path.ends_with(".md") {
            "markdown".into()
        } else if out_path.ends_with(".sql") {
            "sql".into()
        } else {
            "csv".into()
        }
    });
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("sql".into(), json!(statement));
    params.insert("path".into(), json!(out_path));
    params.insert("format".into(), json!(format));
    if let Some(table) = opt(args, "--table") {
        params.insert("table".into(), json!(table));
    }
    let out = run_command(
        &mut session,
        "tools.exportQuery",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} rows, {} bytes)",
        out["path"].as_str().unwrap_or(""),
        out["rows"].as_u64().unwrap_or(0),
        out["bytes"].as_u64().unwrap_or(0)
    );
    Ok(())
}

fn monitor(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let conn_id = session.conns[index].cfg.id.clone();
    let command = if flag(args, "--processes") {
        "monitor.processes"
    } else if flag(args, "--status") {
        "monitor.status"
    } else if flag(args, "--variables") {
        "monitor.variables"
    } else {
        "monitor.dashboard"
    };
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(conn_id));
    if let Some(filter) = opt(args, "--filter") {
        params.insert("filter".into(), json!(filter));
    }
    let out = run_command(&mut session, command, serde_json::Value::Object(params))
        .map_err(|e| e.to_string())?;
    print_json(&out)
}

fn users(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let conn_id = session.conns[index].cfg.id.clone();
    if let Some(user) = opt(args, "--grants") {
        let out = run_command(
            &mut session,
            "priv.grants",
            json!({ "conn": conn_id, "user": user }),
        )
        .map_err(|e| e.to_string())?;
        for grant in out["grants"].as_array().cloned().unwrap_or_default() {
            println!("  {}", grant.as_str().unwrap_or(""));
        }
        return Ok(());
    }
    let out = run_command(&mut session, "priv.users", json!({ "conn": conn_id }))
        .map_err(|e| e.to_string())?;
    for user in out["users"].as_array().cloned().unwrap_or_default() {
        println!(
            "  {:<28} {}{}{}",
            user["display"].as_str().unwrap_or(""),
            if user["superuser"].as_bool().unwrap_or(false) {
                "superuser "
            } else {
                ""
            },
            if user["can_login"].as_bool().unwrap_or(false) {
                "login "
            } else {
                "no-login "
            },
            user["expires"].as_str().unwrap_or(""),
        );
    }
    Ok(())
}

fn diagram(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let out_path = opt(args, "--out").ok_or("pass --out schema.svg")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("path".into(), json!(out_path));
    if let Some(schema) = opt(args, "--schema") {
        params.insert("schema".into(), json!(schema));
    }
    let out = run_command(
        &mut session,
        "diagram.exportSvg",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} tables, {} relationships, {} bytes)",
        out["path"].as_str().unwrap_or(""),
        out["tables"].as_u64().unwrap_or(0),
        out["relationships"].as_u64().unwrap_or(0),
        out["bytes"].as_u64().unwrap_or(0)
    );
    Ok(())
}

fn chart(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals.first().ok_or("pass a connection")?;
    let statement = positionals.get(1).ok_or("pass SQL")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("sql".into(), json!(statement));
    params.insert(
        "chart".into(),
        json!(opt(args, "--chart").unwrap_or_else(|| "column".into())),
    );
    if let Some(label) = opt(args, "--label") {
        params.insert("label".into(), json!(label));
    }
    let out = run_command(
        &mut session,
        "report.chart",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    if out["chart"] == "table" {
        return print_json(&out["result"]);
    }
    println!("{}", out["summary"].as_str().unwrap_or(""));
    let labels: Vec<String> = out["labels"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    for series in out["series"].as_array().cloned().unwrap_or_default() {
        println!("{}:", series["name"].as_str().unwrap_or(""));
        let values: Vec<f64> = series["values"]
            .as_array()
            .map(|v| v.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default();
        let max = values.iter().cloned().fold(0.0f64, f64::max).max(1.0);
        for (i, value) in values.iter().enumerate() {
            let width = ((value / max) * 40.0).round().max(0.0) as usize;
            println!(
                "  {:<16} {:>12.2} {}",
                labels.get(i).cloned().unwrap_or_default(),
                value,
                "█".repeat(width)
            );
        }
    }
    Ok(())
}

/// `ai`: draft SQL with a provider, list the providers, or manage an API key.
///
/// A key is read from stdin rather than taken as an argument so it never lands in the process table.
fn ai(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    match positionals.first().map(String::as_str).unwrap_or("") {
        "providers" => ai_providers(args),
        "key" => ai_key(args),
        _ => ai_generate(args),
    }
}

fn ai_generate(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let conn = positionals
        .first()
        .ok_or("pass a connection: nook-cli ai <conn> \"<prompt>\"")?;
    let prompt = positionals
        .get(1)
        .ok_or("pass a prompt: nook-cli ai <conn> \"<prompt>\"")?;
    let mut session = Session::new();
    let index = open_connection(&mut session, conn).map_err(|e| e.to_string())?;
    let mut params = serde_json::Map::new();
    params.insert("conn".into(), json!(session.conns[index].cfg.id));
    params.insert("prompt".into(), json!(prompt));
    if let Some(list) = opt(args, "--tables") {
        let names: Vec<String> = list
            .split(',')
            .map(|table| table.trim().to_string())
            .filter(|table| !table.is_empty())
            .collect();
        if !names.is_empty() {
            params.insert("tables".into(), json!(names));
        }
    }
    // The app reads the session log itself; a shell has nothing else to read it from, and a mention
    // that matched no table is only ever reported here.
    let logged = session.messages.len();
    let out = run_command(
        &mut session,
        "ai.generate",
        serde_json::Value::Object(params),
    )
    .map_err(|e| e.to_string())?;
    warn_from_log(&session, logged);
    if flag(args, "--json") {
        return print_json(&out);
    }
    // The SQL alone goes to stdout so it can be piped onward; where it came from goes to stderr.
    println!("{}", out["sql"].as_str().unwrap_or(""));
    let tables: Vec<&str> = out["tables"]
        .as_array()
        .map(|list| list.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    eprintln!(
        "-- {} via {} · {}",
        out["model"].as_str().unwrap_or(""),
        out["endpoint"].as_str().unwrap_or(""),
        if tables.is_empty() {
            "no table structure".to_string()
        } else {
            format!("tables: {}", tables.join(", "))
        }
    );
    Ok(())
}

fn ai_providers(args: &[String]) -> CliResult {
    let mut session = Session::new();
    let out = run_command(&mut session, "ai.providers", json!({})).map_err(|e| e.to_string())?;
    if flag(args, "--json") {
        return print_json(&out);
    }
    println!(
        "active {} · model {} · {}{}",
        out["active"].as_str().unwrap_or(""),
        out["model"].as_str().unwrap_or(""),
        out["endpoint"].as_str().unwrap_or(""),
        if out["keychain"].as_bool().unwrap_or(false) {
            ""
        } else {
            " · no keychain on this platform: set NOOK_AI_KEY"
        }
    );
    for provider in out["providers"].as_array().cloned().unwrap_or_default() {
        let key = match (
            provider["key_set"].as_bool().unwrap_or(false),
            provider["needs_key"].as_bool().unwrap_or(true),
        ) {
            (true, _) => "key set",
            (_, true) => "no key",
            _ => "no key needed",
        };
        println!(
            "  {:<10} {:<22} {:<15} {}",
            provider["id"].as_str().unwrap_or(""),
            provider["label"].as_str().unwrap_or(""),
            key,
            if provider["active"].as_bool().unwrap_or(false) {
                "<- active"
            } else {
                ""
            }
        );
    }
    Ok(())
}

fn ai_key(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    let provider = positionals
        .get(1)
        .ok_or("pass a provider: nook-cli ai key <provider> [--remove]")?;
    let mut session = Session::new();
    if flag(args, "--remove") {
        let out = run_command(&mut session, "ai.clearKey", json!({"provider": provider}))
            .map_err(|e| e.to_string())?;
        println!(
            "{}: key removed",
            out["provider"].as_str().unwrap_or(provider)
        );
        return Ok(());
    }
    eprint!("key for {provider} (read from stdin, never from an argument): ");
    let _ = std::io::stderr().flush();
    let key = read_secret()?;
    let out = run_command(
        &mut session,
        "ai.setKey",
        json!({"provider": provider, "key": key}),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{}: key saved",
        out["provider"].as_str().unwrap_or(provider)
    );
    Ok(())
}

/// The warnings and errors a command left in the session log, on stderr. Errors here are not fatal
/// — the command already returned its result — but they are the only trace of a partly met request.
fn warn_from_log(session: &Session, since: usize) {
    for message in session.messages.iter().skip(since) {
        let level = match message.level {
            Level::Warn => "warn",
            Level::Error => "error",
            _ => continue,
        };
        eprintln!("nook: {level}: {}", message.text);
    }
}

/// One line from stdin, for a secret that must not show up in `ps`.
fn read_secret() -> std::result::Result<String, String> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("stdin: {e}"))?;
    let key = line.trim().to_string();
    if key.is_empty() {
        return Err("no key on stdin".into());
    }
    Ok(key)
}

fn job(args: &[String]) -> CliResult {
    let positionals = positional(args, 1);
    match positionals.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            let mut session = Session::new();
            let out =
                run_command(&mut session, "job.list", json!({})).map_err(|e| e.to_string())?;
            for job in out["jobs"].as_array().cloned().unwrap_or_default() {
                println!(
                    "  {:<18} {:<20} {:<16} {:<10} runs {} failures {}{}",
                    job["id"].as_str().unwrap_or(""),
                    job["name"].as_str().unwrap_or(""),
                    job["kind"].as_str().unwrap_or(""),
                    job["schedule"].as_str().unwrap_or(""),
                    job["runs"].as_u64().unwrap_or(0),
                    job["failures"].as_u64().unwrap_or(0),
                    if job["enabled"].as_bool().unwrap_or(false) {
                        ""
                    } else {
                        "  disabled"
                    },
                );
                if let Some(status) = job["last_status"].as_str() {
                    println!("      last: {status}");
                }
            }
            Ok(())
        }
        "run" => {
            let id = positionals.get(1).ok_or("pass a job id")?;
            let mut session = Session::new();
            // Jobs reference saved profiles, so open every profile they name.
            let profile_ids: Vec<String> = session
                .jobs
                .jobs
                .iter()
                .flat_map(|j| vec![j.conn.clone(), j.target.clone()])
                .filter(|c| !c.is_empty())
                .collect();
            for profile in profile_ids {
                let _ = open_connection(&mut session, &profile);
            }
            let out = run_command(&mut session, "job.run", json!({ "id": id }))
                .map_err(|e| e.to_string())?;
            println!(
                "{} ok in {} ms: {}",
                out["job"].as_str().unwrap_or(""),
                out["elapsed_ms"].as_u64().unwrap_or(0),
                out["summary"].as_str().unwrap_or("")
            );
            Ok(())
        }
        "serve" => {
            let interval = opt(args, "--interval")
                .and_then(|i| i.parse::<u64>().ok())
                .unwrap_or(30)
                .clamp(5, 3600);
            println!("nook scheduler: every {interval}s (ctrl-c to stop)");
            loop {
                let mut session = Session::new();
                let profile_ids: Vec<String> = session
                    .jobs
                    .jobs
                    .iter()
                    .flat_map(|j| vec![j.conn.clone(), j.target.clone()])
                    .filter(|c| !c.is_empty())
                    .collect();
                for profile in profile_ids {
                    let _ = open_connection(&mut session, &profile);
                }
                match run_command(&mut session, "job.runDue", json!({})) {
                    Ok(out) => {
                        let due = out["due"].as_u64().unwrap_or(0);
                        if due > 0 {
                            println!("{} job(s) due", due);
                            for result in out["results"].as_array().cloned().unwrap_or_default() {
                                println!(
                                    "  {} {} {}",
                                    result["job"].as_str().unwrap_or(""),
                                    if result["ok"].as_bool().unwrap_or(false) {
                                        "ok"
                                    } else {
                                        "failed"
                                    },
                                    result["summary"]
                                        .as_str()
                                        .or_else(|| result["error"].as_str())
                                        .unwrap_or("")
                                );
                            }
                        }
                        let _ = std::io::stdout().flush();
                    }
                    Err(e) => eprintln!("scheduler tick failed: {e}"),
                }
                std::thread::sleep(std::time::Duration::from_secs(interval));
            }
        }
        other => Err(format!(
            "unknown job subcommand '{other}'; try list, run or serve"
        )),
    }
}
