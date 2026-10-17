//! Tools: data generation, CSV/JSON import, SQL import, export, the visual query builder and reports.

use std::collections::BTreeMap;

use nook_core::{quote_ident, quote_qualified, sql_literal, Dialect, Error, Result, Value};
use nook_model::ddl;
use serde_json::json;

use crate::cmd::{dialect_name, run_sql};
use crate::datagen::{self, GenSpec, Generator};
use crate::report::{self, ChartKind};
use crate::{CommandSpec, Level, Params, Session, TableRef};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("tools.datagen", "Generate data", "Tools", datagen_cmd)
            .params(r#"{"table":"customers","rows":1000,"seed":1,"overrides":{"status":"pick:new|closed"},"apply":false,"out":"~/customers.sql"}"#)
            .help("Fake-but-consistent rows. apply:true inserts them; \"out\" writes an INSERT script."),
        CommandSpec::new("tools.generators", "Available generators", "Tools", generators),
        CommandSpec::new("tools.importCsv", "Import CSV", "Tools", import_csv)
            .params(r#"{"path":"~/data/users.csv","table":"users","create_table":true}"#),
        CommandSpec::new("tools.importExcel", "Import Excel", "Tools", import_excel)
            .params(r#"{"path":"~/data/users.xlsx","table":"users","create_table":true}"#)
            .help("First sheet of an .xlsx file; every cell arrives as text, like the CSV import."),
        CommandSpec::new("tools.importJson", "Import JSON", "Tools", import_json)
            .params(r#"{"path":"~/data/users.json","table":"users","create_table":true}"#),
        CommandSpec::new("tools.importSql", "Run a .sql file", "Tools", import_sql)
            .params(r#"{"path":"~/data/seed.sql"}"#),
        CommandSpec::new("tools.exportExcel", "Export a query result to Excel", "Tools", export_excel)
            .params(r#"{"sql":"select * from users","path":"~/exports/users.xlsx"}"#),
        CommandSpec::new("tools.exportQuery", "Export a query result", "Tools", export_query)
            .params(r#"{"sql":"select * from users","path":"~/exports/users.csv","format":"csv"}"#),
        CommandSpec::new("tools.queryBuilder", "Query builder", "Tools", query_builder)
            .params(r#"{"from":[{"table":"users","alias":"u","columns":["id","email"]}],"where":"u.email like '%@example.com'","order_by":"u.id","limit":100}"#)
            .help("Builds a SELECT from a table/column/join description. Returns the SQL."),
        CommandSpec::new("report.aggregate", "Aggregate report", "Report", aggregate)
            .params(r#"{"table":"orders","group":"status","agg":"sum","value":"amount","chart":"column"}"#),
        CommandSpec::new("report.chart", "Chart a query", "Report", chart)
            .params(r#"{"sql":"select status, count(*) from orders group by status","chart":"pie","label":"status"}"#),
        CommandSpec::new("report.pivot", "Pivot a result", "Report", pivot)
            .params(r#"{"sql":"select region, quarter, sum(total) from sales group by 1,2"}"#),
    ]
}

fn generators(_session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    Ok(json!({
        "generators": [
            "auto", "sequence", "int:1-100", "float:0-1000", "bool",
            "firstname", "lastname", "name", "email", "phone", "city", "country", "company",
            "word", "sentence", "pattern:INV-####", "pick:a|b|c",
            "date:365", "timestamp:365", "uuid", "null"
        ],
        "notes": "Overrides are keyed by column name, for example {\"email\": \"pattern:user-####@example.com\"}",
    }))
}

fn datagen_cmd(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let spec_name = p.str_arg("table")?;
    let reference = TableRef::parse(&spec_name);
    let table = session.table_of(index, reference.schema.as_deref(), &reference.table)?;
    let rows = p.int_opt("rows", 100).max(0) as usize;
    let seed = p.int_opt("seed", 1).max(0) as u64;
    let mut spec = GenSpec::from_table(&table, rows, seed);
    if let Some(serde_json::Value::Object(map)) = p.get("overrides") {
        let overrides: BTreeMap<String, String> = map
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        spec.apply_overrides(&overrides)?;
    }
    let generated = datagen::generate(&spec, &table)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let script = datagen::insert_script(&spec, &table, &generated, dialect)?;

    let mut out = json!({
        "table": table.key(),
        "rows": generated.len(),
        "seed": seed,
        "statements": script.lines().count(),
        "preview": generated.iter().take(5).map(|row| {
            spec.columns.iter().enumerate().map(|(i, c)| {
                (c.name.clone(), row.get(i).cloned().unwrap_or(Value::Null).to_json())
            }).collect::<serde_json::Map<String, serde_json::Value>>()
        }).collect::<Vec<_>>(),
    });

    if p.bool_opt("apply", false) {
        let limit = session.settings.max_rows;
        let statements: Vec<&str> = script.lines().take(limit).collect();
        session.driver_for(index)?.begin()?;
        for statement in &statements {
            if let Err(e) = run_sql(session, index, statement, Some(1)) {
                let _ = session.driver_for(index).and_then(|d| d.rollback());
                return Err(e);
            }
        }
        session.driver_for(index)?.commit()?;
        session.log(
            Level::Success,
            format!("inserted {} generated rows", statements.len()),
        );
        out["inserted"] = json!(statements.len());
    }

    if let Some(path) = p.str_opt("out") {
        let path = crate::cmd::grid::expand(&path, &session.home);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
        }
        std::fs::write(&path, &script)
            .map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
        out["path"] = json!(path.to_string_lossy());
    }
    Ok(out)
}

/// CSV per RFC 4180, including quoted fields with embedded commas, quotes and newlines.
pub fn parse_csv(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => in_quotes = true,
            c if c == delimiter => {
                row.push(std::mem::take(&mut field));
            }
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

/// Guess a value's type so an import does not turn every number into text.
pub fn infer_value(text: &str, target_type: Option<&str>) -> Value {
    let raw = text.trim();
    if let Some(ty) = target_type {
        if let Ok(v) = Value::coerce(raw, ty) {
            return v;
        }
    }
    if raw.is_empty() {
        return Value::Null;
    }
    if raw.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    if raw.eq_ignore_ascii_case("true") || raw.eq_ignore_ascii_case("false") {
        return Value::Bool(raw.eq_ignore_ascii_case("true"));
    }
    if raw.parse::<i64>().is_ok() {
        return Value::Int(raw.parse().unwrap_or(0));
    }
    if raw.parse::<f64>().is_ok() && raw.contains('.') {
        return Value::Float(raw.parse().unwrap_or(0.0));
    }
    Value::text(raw)
}

/// `["src=target", ...]` pairs from the `columnMapping` parameter; empty input means no mapping.
pub(crate) fn parse_mapping(entries: &[String]) -> Result<Option<Vec<(String, String)>>> {
    if entries.is_empty() {
        return Ok(None);
    }
    let mut map = Vec::with_capacity(entries.len());
    for entry in entries {
        let (source, target) = entry.split_once('=').ok_or_else(|| {
            Error::config(format!(
                "columnMapping entries are 'source=target', got '{entry}'"
            ))
        })?;
        let (source, target) = (source.trim(), target.trim());
        if source.is_empty() || target.is_empty() {
            return Err(Error::config(format!(
                "columnMapping entries are 'source=target', got '{entry}'"
            )));
        }
        map.push((source.to_string(), target.to_string()));
    }
    Ok(Some(map))
}

fn target_columns(
    session: &Session,
    index: usize,
    reference: &TableRef,
    fallback: &[String],
) -> Vec<String> {
    session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
        .map(|t| t.column_names())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| fallback.to_vec())
}

fn load_rows(
    session: &mut Session,
    p: &Params,
    reference: &TableRef,
    rows: Vec<Vec<String>>,
    header: bool,
    create_table: bool,
) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    if rows.is_empty() {
        return Err(Error::config("the file has no rows"));
    }
    let (columns, data): (Vec<String>, &[Vec<String>]) = if header {
        (rows[0].clone(), &rows[1..])
    } else {
        (
            (0..rows[0].len())
                .map(|i| format!("column_{}", i + 1))
                .collect(),
            &rows[..],
        )
    };
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let max = session
        .settings
        .max_rows
        .min(p.int_opt("max_rows", 500_000).max(1) as usize);
    let data: Vec<&Vec<String>> = data.iter().take(max).collect();

    let mapping = parse_mapping(&p.list("columnMapping"))?;
    // When a mapping is present it speaks for itself: every target must be a real table column,
    // every source a file column. The count-based alignment gate below no longer applies, and a
    // partial mapping must not fall back to the file's header names or create a table from them.
    let reordered: Vec<Vec<String>>;
    let (target_columns, data): (Vec<String>, Vec<&Vec<String>>) = if let Some(map) = &mapping {
        let table = session
            .conns
            .get(index)
            .and_then(|c| c.catalog.as_ref())
            .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
            .ok_or_else(|| {
                Error::config(format!(
                    "columnMapping needs an existing table; {} was not found in the catalog",
                    reference.key()
                ))
            })?;
        let mut targets = Vec::with_capacity(map.len());
        let mut order = Vec::with_capacity(map.len());
        for (source, target) in map {
            if table.column(target).is_none() {
                return Err(Error::config(format!(
                    "the table {} has no column '{target}'",
                    reference.key()
                )));
            }
            let position = columns
                .iter()
                .position(|c| c == source)
                .ok_or_else(|| Error::config(format!("the file has no column '{source}'")))?;
            targets.push(target.clone());
            order.push(position);
        }
        // Reorder every row into the mapped column order once, so the insert loop keeps its
        // positional walk.
        reordered = data
            .iter()
            .map(|row| {
                order
                    .iter()
                    .map(|i| row.get(*i).cloned().unwrap_or_default())
                    .collect::<Vec<String>>()
            })
            .collect();
        (targets, reordered.iter().collect())
    } else {
        let existing = target_columns(session, index, reference, &columns);
        let use_existing = existing.len() == columns.len()
            && !session
                .conns
                .get(index)
                .and_then(|c| c.catalog.as_ref())
                .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
                .is_none();
        let targets = if use_existing {
            existing
        } else {
            columns.clone()
        };
        (targets, data)
    };
    let types: Vec<Option<String>> = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
        .map(|t| {
            target_columns
                .iter()
                .map(|name| t.column(name).map(|c| c.type_name.clone()))
                .collect()
        })
        .unwrap_or_else(|| vec![None; target_columns.len()]);

    let mut created = false;
    if create_table
        && session
            .conns
            .get(index)
            .and_then(|c| c.catalog.as_ref())
            .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
            .is_none()
    {
        let mut table = nook_model::Table::new(reference.schema.clone(), reference.table.clone());
        for name in &target_columns {
            table
                .columns
                .push(nook_model::Column::new(name.clone(), "TEXT"));
        }
        run_sql(session, index, &ddl::create_table(&table, dialect), Some(1))?;
        created = true;
        let _ = crate::cmd::refresh_catalog(session, index);
    }

    let target = quote_qualified(reference.schema.as_deref(), &reference.table, dialect);
    let column_list = target_columns
        .iter()
        .map(|c| quote_ident(c, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let mut inserted = 0usize;
    session.driver_for(index)?.begin()?;
    for row in &data {
        let values = (0..target_columns.len())
            .map(|i| {
                let raw = row.get(i).map(String::as_str).unwrap_or("");
                let value = infer_value(raw, types.get(i).and_then(|t| t.as_deref()));
                sql_literal(&value, dialect)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("INSERT INTO {target} ({column_list}) VALUES ({values})");
        if let Err(e) = run_sql(session, index, &sql, Some(1)) {
            let _ = session.driver_for(index).and_then(|d| d.rollback());
            session.log(
                Level::Error,
                format!("import failed at row {inserted}; rolled back"),
            );
            return Err(e);
        }
        inserted += 1;
    }
    session.driver_for(index)?.commit()?;
    let _ = crate::cmd::refresh_catalog(session, index);
    session.log(
        Level::Success,
        format!("imported {inserted} rows into {}", reference.key()),
    );
    Ok(json!({
        "table": reference.key(),
        "columns": target_columns,
        "inserted": inserted,
        "skipped": rows.len().saturating_sub(data.len() + usize::from(header)),
        "created_table": created,
        "dialect": dialect_name(dialect),
    }))
}

fn import_csv(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let path = crate::cmd::grid::expand(&p.str_arg("path")?, &session.home);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
    let delimiter = p
        .str_opt("delimiter")
        .and_then(|d| d.chars().next())
        .unwrap_or(',');
    let rows = parse_csv(&text, delimiter);
    let reference = TableRef::parse(&p.str_arg("table")?);
    let header = p.bool_opt("header", true);
    let create_table = p.bool_opt("create_table", true);
    let mut out = load_rows(session, p, &reference, rows, header, create_table)?;
    out["path"] = json!(path.to_string_lossy());
    out["format"] = json!("csv");
    Ok(out)
}

fn import_excel(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let path = crate::cmd::grid::expand(&p.str_arg("path")?, &session.home);
    let rows = nook_drivers::xlsx::read_rows(&path)?;
    let reference = TableRef::parse(&p.str_arg("table")?);
    let header = p.bool_opt("header", true);
    let create_table = p.bool_opt("create_table", true);
    let mut out = load_rows(session, p, &reference, rows, header, create_table)?;
    out["path"] = json!(path.to_string_lossy());
    out["format"] = json!("xlsx");
    Ok(out)
}

fn export_excel(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let sql = p
        .str_opt("sql")
        .ok_or_else(|| Error::config("pass \"sql\""))?;
    let result = run_sql(session, index, &sql, None)?;
    let mut rows: Vec<Vec<String>> = vec![result.columns.clone()];
    for row in &result.rows {
        // as_text is the round-trip method: NULL must export as an empty cell, not "NULL".
        rows.push(row.iter().map(|v| v.as_text()).collect());
    }
    let path = crate::cmd::grid::expand(
        &p.str_opt("path")
            .unwrap_or_else(|| format!("exports/query-{}.xlsx", crate::jobs::now())),
        &session.home,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
    }
    nook_drivers::xlsx::write_rows(&path, &rows)?;
    session.log(Level::Success, format!("exported to {}", path.display()));
    Ok(json!({
        "path": path.to_string_lossy(),
        "rows": result.row_count(),
        "format": "xlsx",
    }))
}

fn import_json(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let path = crate::cmd::grid::expand(&p.str_arg("path")?, &session.home);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => {
            // Also accept {"rows": [...]} and a single object.
            match map.get("rows") {
                Some(serde_json::Value::Array(items)) => items.clone(),
                _ => vec![serde_json::Value::Object(map)],
            }
        }
        _ => return Err(Error::config("the JSON root must be an array or object")),
    };
    let mut columns: Vec<String> = Vec::new();
    for item in &items {
        if let serde_json::Value::Object(map) = item {
            for key in map.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    if columns.is_empty() {
        return Err(Error::config("the JSON has no object keys to import"));
    }
    let mut rows = vec![columns.clone()];
    for item in &items {
        let map = item.as_object().ok_or_else(|| {
            Error::config("every array entry must be an object with named fields")
        })?;
        rows.push(
            columns
                .iter()
                .map(|c| match map.get(c) {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                })
                .collect(),
        );
    }
    let reference = TableRef::parse(&p.str_arg("table")?);
    let create_table = p.bool_opt("create_table", true);
    let mut out = load_rows(session, p, &reference, rows, true, create_table)?;
    out["path"] = json!(path.to_string_lossy());
    out["format"] = json!("json");
    Ok(out)
}

fn import_sql(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let path = crate::cmd::grid::expand(&p.str_arg("path")?, &session.home);
    let script = std::fs::read_to_string(&path)
        .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
    let statements = nook_drivers::split_script(&script);
    if statements.is_empty() {
        return Err(Error::config("the file has no statements"));
    }
    run_sql(session, index, &script, None)?;
    let _ = crate::cmd::refresh_catalog(session, index);
    session.log(
        Level::Success,
        format!(
            "ran {} statements from {}",
            statements.len(),
            path.display()
        ),
    );
    Ok(json!({
        "path": path.to_string_lossy(),
        "statements": statements.len(),
    }))
}

fn export_query(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let sql = p
        .str_opt("sql")
        .ok_or_else(|| Error::config("pass \"sql\""))?;
    let format = p.str_opt("format").unwrap_or_else(|| "csv".into());
    let result = run_sql(session, index, &sql, None)?;
    let text = match format.as_str() {
        "csv" => result.to_csv(),
        "json" => serde_json::to_string_pretty(&result.to_json())
            .map_err(|e| Error::internal(format!("json: {e}")))?,
        "markdown" | "md" => result.to_markdown(),
        "sql" => {
            let reference =
                crate::TableRef::parse(&p.str_opt("table").unwrap_or_else(|| "export".into()));
            let dialect = session
                .conns
                .get(index)
                .and_then(|c| c.driver.as_ref())
                .map(|d| d.dialect())
                .unwrap_or(Dialect::Sqlite);
            result.to_sql_inserts(reference.schema.as_deref(), &reference.table, dialect)
        }
        other => {
            return Err(Error::config(format!(
                "unknown format '{other}'; try csv, json, sql or markdown"
            )))
        }
    };
    let path = crate::cmd::grid::expand(
        &p.str_opt("path").unwrap_or_else(|| {
            format!(
                "exports/query-{}.{}",
                crate::jobs::now(),
                if format == "json" { "json" } else { "csv" }
            )
        }),
        &session.home,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, &text)
        .map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
    session.log(Level::Success, format!("exported to {}", path.display()));
    Ok(json!({
        "path": path.to_string_lossy(),
        "rows": result.row_count(),
        "bytes": text.len(),
        "format": format,
    }))
}

/// Visual query builder: tables, joins, columns, filters, grouping and ordering into one SELECT.
fn query_builder(_session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let from = p
        .get("from")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::config("pass \"from\": [{table, columns, join}]"))?;
    if from.is_empty() {
        return Err(Error::config("at least one table is required"));
    }
    let dialect = p
        .str_opt("dialect")
        .as_deref()
        .and_then(crate::cmd::parse_dialect)
        .unwrap_or(Dialect::Postgres);

    // First pass: read every entry, qualify the tables and collect the columns.
    struct Entry {
        target: String,
        prefix: String,
        columns: Vec<String>,
        join: Option<(String, String)>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    for raw in from {
        let params = Params::from_json(raw.clone())?;
        let name = params
            .str_opt("table")
            .ok_or_else(|| Error::config("every \"from\" entry needs a table"))?;
        let reference = TableRef::parse(&name);
        let qualified = quote_qualified(reference.schema.as_deref(), &reference.table, dialect);
        let alias = params.str_opt("alias");
        let target = match &alias {
            Some(a) => format!("{qualified} AS {}", quote_ident(a, dialect)),
            None => qualified,
        };
        let prefix = alias.unwrap_or_else(|| reference.table.clone());
        let join = match params.str_opt("on") {
            Some(on) => {
                let mut kind = params
                    .str_opt("join")
                    .unwrap_or_else(|| "inner".into())
                    .to_ascii_uppercase();
                if !kind.ends_with("JOIN") {
                    kind.push_str(" JOIN");
                }
                Some((kind, on))
            }
            None => None,
        };
        entries.push(Entry {
            target,
            prefix,
            columns: params.list("columns"),
            join,
        });
    }
    if entries.first().map(|e| e.join.is_some()).unwrap_or(false) {
        return Err(Error::config(
            "the first table has no join condition; only later entries take \"on\"",
        ));
    }

    // Second pass: the select list needs every table's columns, so it is built after the loop.
    let mut selects: Vec<String> = Vec::new();
    for entry in &entries {
        if entry.columns.is_empty() {
            selects.push(format!("{}.*", quote_ident(&entry.prefix, dialect)));
        } else {
            for column in &entry.columns {
                selects.push(format!(
                    "{}.{}",
                    quote_ident(&entry.prefix, dialect),
                    quote_ident(column, dialect)
                ));
            }
        }
    }

    let mut sql = format!("SELECT {} FROM {}", selects.join(", "), entries[0].target);
    for entry in entries.iter().skip(1) {
        let (kind, on) = entry.join.clone().ok_or_else(|| {
            Error::config(format!("the join onto '{}' needs \"on\"", entry.prefix))
        })?;
        sql.push_str(&format!(" {kind} {} ON {on}", entry.target));
    }
    if let Some(clause) = p.str_opt("where") {
        if !clause.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", clause.trim()));
        }
    }
    if let Some(group) = p.str_opt("group_by") {
        if !group.trim().is_empty() {
            sql.push_str(&format!(" GROUP BY {}", group.trim()));
        }
    }
    if let Some(having) = p.str_opt("having") {
        if !having.trim().is_empty() {
            sql.push_str(&format!(" HAVING {}", having.trim()));
        }
    }
    if let Some(order) = p.str_opt("order_by") {
        if !order.trim().is_empty() {
            sql.push_str(&format!(" ORDER BY {}", order.trim()));
        }
    }
    let limit = p.int_opt("limit", 100).clamp(1, 1_000_000);
    sql.push_str(&format!(" LIMIT {limit};"));
    Ok(json!({ "sql": sql, "columns": selects.len() }))
}

fn chart(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let kind = p
        .str_opt("chart")
        .as_deref()
        .and_then(ChartKind::parse)
        .unwrap_or(ChartKind::Table);
    let title = p.str_opt("title").unwrap_or_default();
    let label = p.str_opt("label");
    let sql = p
        .str_opt("sql")
        .or_else(|| {
            let reference = p.str_opt("table")?;
            let table_ref = TableRef::parse(&reference);
            let table = session
                .table_of(index, table_ref.schema.as_deref(), &table_ref.table)
                .ok()?;
            let group = p.str_opt("group")?;
            let dialect = session.conns.get(index)?.driver.as_ref()?.dialect();
            report::aggregate_sql(
                &table,
                &group,
                &p.str_opt("agg").unwrap_or_else(|| "count".into()),
                p.str_opt("value").as_deref(),
                dialect,
                p.int_opt("limit", 50).clamp(1, 10_000) as usize,
            )
            .ok()
        })
        .ok_or_else(|| Error::config("pass \"sql\", or \"table\" with \"group\""))?;
    let result = run_sql(session, index, &sql, None)?;
    if kind == ChartKind::Table {
        return Ok(json!({
            "sql": sql,
            "chart": "table",
            "result": result.to_json(),
        }));
    }
    let labeled = report::chart_from_result(&result, kind, &title, label.as_deref())?;
    // An aggregate report is more readable with the label column named.
    let labeled = if labeled.labels.is_empty() && result.column_count() >= 2 {
        report::chart_from_result(
            &result,
            kind,
            &title,
            result.columns.first().map(String::as_str),
        )?
    } else {
        labeled
    };
    Ok(json!({
        "sql": sql,
        "chart": labeled.kind.label(),
        "title": labeled.title,
        "labels": labeled.labels,
        "series": labeled.series.iter().map(|s| json!({ "name": s.name, "values": s.values })).collect::<Vec<_>>(),
        "summary": labeled.summary(),
        "rows": result.row_count(),
    }))
}

fn aggregate(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let reference = TableRef::parse(&p.str_arg("table")?);
    let table = session.table_of(index, reference.schema.as_deref(), &reference.table)?;
    let group = p.str_arg("group")?;
    let agg = p.str_opt("agg").unwrap_or_else(|| "count".into());
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = report::aggregate_sql(
        &table,
        &group,
        &agg,
        p.str_opt("value").as_deref(),
        dialect,
        p.int_opt("limit", 50).clamp(1, 10_000) as usize,
    )?;
    let result = run_sql(session, index, &sql, None)?;
    let kind = p
        .str_opt("chart")
        .as_deref()
        .and_then(ChartKind::parse)
        .unwrap_or(ChartKind::Column);
    let chart =
        report::chart_from_result(&result, kind, format!("{agg} by {group}"), Some("label"))?;
    Ok(json!({
        "sql": sql,
        "rows": result.row_count(),
        "data": result.to_json()["rows"],
        "chart": chart.kind.label(),
        "labels": chart.labels,
        "series": chart.series.iter().map(|s| json!({ "name": s.name, "values": s.values })).collect::<Vec<_>>(),
    }))
}

fn pivot(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let sql = p.str_arg("sql")?;
    let result = run_sql(session, index, &sql, None)?;
    let (rows, columns, cells) = report::pivot(&result)?;
    Ok(json!({
        "rows": rows,
        "columns": columns,
        "cells": cells,
        "source_rows": result.row_count(),
    }))
}

/// Helper for the UI: what a generator spec is, for the override editor's placeholder text.
pub fn generator_hint(_generator: &Generator) -> &'static str {
    "auto|int:1-100|pick:a|b|pattern:INV-####|email|sentence|uuid|date:365|null"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parsing_handles_quotes_and_newlines() {
        let text = "id,name\n1,\"Smith, John\"\n2,\"line\nbreak\"\n";
        let rows = parse_csv(text, ',');
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1], "Smith, John");
        assert_eq!(rows[2][1], "line\nbreak");
        // Escaped quotes survive.
        let rows = parse_csv("a\n\"say \"\"hi\"\"\"\n", ',');
        assert_eq!(rows[1][0], "say \"hi\"");
        // A trailing blank line does not become a row.
        assert_eq!(parse_csv("a,b\n1,2\n\n", ',').len(), 2);
    }

    #[test]
    fn inference_follows_the_target_type_first() {
        assert_eq!(infer_value("42", Some("TEXT")), Value::text("42"));
        assert_eq!(infer_value("42", Some("INTEGER")), Value::Int(42));
        assert_eq!(infer_value("42", None), Value::Int(42));
        assert_eq!(infer_value("4.5", None), Value::Float(4.5));
        assert_eq!(infer_value("true", None), Value::Bool(true));
        assert_eq!(infer_value("", None), Value::Null);
        assert_eq!(infer_value("null", None), Value::Null);
        assert_eq!(infer_value("hello", None), Value::text("hello"));
    }

    #[test]
    fn query_builder_joins_and_filters() {
        let mut session = Session::new();
        let p = Params::from_json(json!({
            "from": [
                {"table": "public.users", "alias": "u", "columns": ["id", "email"]},
                {"table": "public.orders", "alias": "o", "columns": ["total"], "join": "left", "on": "o.user_id = u.id"}
            ],
            "where": "u.email like '%@example.com'",
            "order_by": "u.id desc",
            "limit": 25,
            "dialect": "postgres"
        }))
        .expect("params");
        let out = query_builder(&mut session, &p).expect("build");
        let sql = out["sql"].as_str().unwrap_or_default();
        assert!(sql.contains("SELECT \"u\".\"id\", \"u\".\"email\", \"o\".\"total\""));
        assert!(sql.contains("FROM \"public\".\"users\" AS \"u\""));
        assert!(sql.contains("LEFT JOIN \"public\".\"orders\" AS \"o\" ON o.user_id = u.id"));
        assert!(sql.contains("WHERE u.email like"));
        assert!(sql.ends_with("LIMIT 25;"));
    }

    #[test]
    fn query_builder_needs_tables_and_join_conditions() {
        let mut session = Session::new();
        assert!(query_builder(&mut session, &Params::none()).is_err());
        let p =
            Params::from_json(json!({"from": [{"table": "a"}, {"table": "b"}]})).expect("params");
        assert!(query_builder(&mut session, &p).is_err());
    }

    #[test]
    fn importing_a_missing_file_is_an_error() {
        let mut session = Session::new();
        let p =
            Params::from_json(json!({"path": "/tmp/nope-nook.csv", "table": "t"})).expect("params");
        assert!(import_csv(&mut session, &p).is_err());
        let p = Params::from_json(json!({"path": "/tmp/nope-nook.json", "table": "t"}))
            .expect("params");
        assert!(import_json(&mut session, &p).is_err());
    }

    #[test]
    fn generator_hint_is_documented() {
        assert!(generator_hint(&Generator::Auto).contains("pick:"));
        let mut session = Session::new();
        let out = generators(&mut session, &Params::none()).expect("list");
        assert!(out["generators"].as_array().map(Vec::len).unwrap_or(0) > 10);
    }
}
