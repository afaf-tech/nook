//! Grid commands: paging, sorting, filtering, inline edits, export.

use std::path::PathBuf;

use nook_core::{Error, Result, Value};
use nook_model::{plan_edits, EditKind, RowEdit};
use serde_json::json;

use crate::cmd::{bind_grid_table, load_grid, pk_values, run_sql, sync_grid_table};
use crate::{CommandSpec, Level, Params, Session, TabKind, TableRef};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("grid.open", "Open table data", "Data", open)
            .params(r#"{"table":"public.users"}"#)
            .help("Open a data tab bound to a table, with the primary key known so edits work."),
        CommandSpec::new("grid.fetchPage", "Go to page", "Data", fetch_page)
            .params(r#"{"page":2}"#),
        CommandSpec::new("grid.nextPage", "Next page", "Data", next_page),
        CommandSpec::new("grid.prevPage", "Previous page", "Data", prev_page),
        CommandSpec::new("grid.firstPage", "First page", "Data", first_page),
        CommandSpec::new("grid.lastPage", "Last page", "Data", last_page),
        CommandSpec::new("grid.setPageSize", "Rows per page", "Data", set_page_size)
            .params(r#"{"page_size":500}"#),
        CommandSpec::new("grid.sort", "Sort by column", "Data", sort)
            .params(r#"{"column":"created_at","desc":true}"#),
        CommandSpec::new("grid.filter", "Filter rows", "Data", filter)
            .params(r#"{"where":"status = 'live'"}"#)
            .help("A raw WHERE clause, shown in the grid's filter bar."),
        CommandSpec::new("grid.clearFilter", "Clear the filter", "Data", clear_filter),
        CommandSpec::new("grid.countRows", "Count rows", "Data", count_rows),
        CommandSpec::new("grid.editCell", "Edit a cell", "Data", edit_cell)
            .params(r#"{"row":0,"column":"email","value":"new@example.com"}"#)
            .help("Records an edit; nothing is sent until grid.apply."),
        CommandSpec::new("grid.setNull", "Set a cell to NULL", "Data", set_null),
        CommandSpec::new("grid.insertRow", "Insert a row", "Data", insert_row)
            .params(r#"{"values":{"email":"new@example.com"}}"#),
        CommandSpec::new("grid.deleteRow", "Delete a row", "Data", delete_row)
            .params(r#"{"row":0}"#),
        CommandSpec::new(
            "grid.duplicateRow",
            "Duplicate a row",
            "Data",
            duplicate_row,
        )
        .params(r#"{"row":0}"#),
        CommandSpec::new("grid.pending", "Show pending edits", "Data", pending),
        CommandSpec::new("grid.revert", "Discard pending edits", "Data", revert),
        CommandSpec::new("grid.apply", "Apply changes", "Data", apply)
            .shortcut("cmd+s")
            .help("Runs the pending statements on the server, then reloads the page."),
        CommandSpec::new("grid.export", "Export the grid", "Data", export)
            .params(r#"{"format":"csv","path":"/tmp/rows.csv"}"#)
            .help("format: csv, json, sql, markdown. Without a path the text is returned."),
        CommandSpec::new(
            "grid.exportTable",
            "Export a whole table",
            "Data",
            export_table,
        )
        .params(r#"{"table":"users","format":"csv","path":"/tmp/users.csv"}"#),
        CommandSpec::new("grid.find", "Find in the grid", "Data", find).params(r#"{"text":"a@b"}"#),
        CommandSpec::new("grid.cellValue", "Read one cell", "Data", cell_value)
            .params(r#"{"row":0,"column":"id"}"#),
        CommandSpec::new("grid.formView", "Form view of one record", "Data", form_view)
            .params(r#"{"row":0}"#)
            .help("The record at `row` as a vertical form: one field per column with its type, value and whether it is a key. Read-only over the page the grid already holds."),
        CommandSpec::new("grid.copyAsSql", "Copy rows as INSERT", "Data", copy_as_sql),
    ]
}

fn active_grid_tab(session: &Session) -> Result<usize> {
    let index = session
        .active_tab
        .ok_or_else(|| Error::internal("no active tab"))?;
    let tab = session
        .tabs
        .get(index)
        .ok_or_else(|| Error::internal("active tab is gone"))?;
    if tab.kind != TabKind::Data && tab.grid.table.is_none() {
        return Err(Error::config(no_grid_reason()));
    }
    Ok(index)
}

/// Why the grid commands refuse the active tab. One definition, so a surface that cannot run one —
/// the results register's Export sits on a query tab, which has no data grid to hand over — can say
/// the engine's own words rather than its own.
pub fn no_grid_reason() -> &'static str {
    "the active tab has no data grid; open a table with grid.open"
}

fn grid_summary(session: &Session, index: usize) -> serde_json::Value {
    let tab = match session.tabs.get(index) {
        Some(t) => t,
        None => return json!({ "error": "tab is gone" }),
    };
    json!({
        "table": tab.grid.table.as_ref().map(TableRef::key),
        "page": tab.grid.page,
        "page_size": tab.grid.page_size,
        "rows": tab.grid.result.row_count(),
        "columns": tab.grid.result.columns,
        "types": tab.grid.result.column_types,
        "primary_key": tab.grid.primary_key,
        "order": tab.grid.order,
        "filter": tab.grid.filter,
        "total_rows": tab.grid.total_rows,
        "pending": tab.grid.pending.len(),
        "truncated": tab.grid.result.truncated,
        "elapsed_ms": tab.grid.elapsed_ms,
        "error": tab.grid.error,
    })
}

fn open(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let spec = p
        .str_opt("table")
        .ok_or_else(|| Error::config("pass \"table\""))?;
    let reference = TableRef::parse(&spec);
    let title = reference.key();
    let index = session.add_tab(TabKind::Data, title, Some(conn));
    bind_grid_table(session, index, reference)?;
    load_grid(session, index)?;
    if p.has("page_size") {
        let size = p.int_opt("page_size", 200).clamp(1, 100_000) as usize;
        if let Some(tab) = session.tabs.get_mut(index) {
            tab.grid.page_size = size;
        }
        load_grid(session, index)?;
    }
    Ok(grid_summary(session, index))
}

fn fetch_page(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let page = p.int_opt("page", 0).max(0) as usize;
    session
        .tabs
        .get_mut(index)
        .ok_or_else(|| Error::internal("tab is gone"))?
        .grid
        .page = page;
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn next_page(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    {
        let tab = session
            .tabs
            .get_mut(index)
            .ok_or_else(|| Error::internal("tab is gone"))?;
        if tab.grid.result.row_count() < tab.grid.page_size {
            return Ok(grid_summary(session, index));
        }
        tab.grid.page += 1;
    }
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn prev_page(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    {
        let tab = session
            .tabs
            .get_mut(index)
            .ok_or_else(|| Error::internal("tab is gone"))?;
        tab.grid.page = tab.grid.page.saturating_sub(1);
    }
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn first_page(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    session
        .tabs
        .get_mut(index)
        .ok_or_else(|| Error::internal("tab is gone"))?
        .grid
        .page = 0;
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn last_page(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let total = count_rows(session, &Params::none())?;
    let rows = total["rows"].as_i64().unwrap_or(0).max(0) as usize;
    let size = session.tabs[index].grid.page_size.max(1);
    session
        .tabs
        .get_mut(index)
        .ok_or_else(|| Error::internal("tab is gone"))?
        .grid
        .page = rows.saturating_sub(1) / size;
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn set_page_size(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let size = p.int_opt("page_size", 200).clamp(1, 100_000) as usize;
    session
        .tabs
        .get_mut(index)
        .ok_or_else(|| Error::internal("tab is gone"))?
        .grid
        .page_size = size;
    session.settings.page_size = size;
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn sort(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let column = p.str_arg("column")?;
    let desc = p.bool_opt("desc", false);
    {
        let tab = session
            .tabs
            .get_mut(index)
            .ok_or_else(|| Error::internal("tab is gone"))?;
        if !tab.grid.result.columns.iter().any(|c| c == &column) {
            return Err(Error::not_found(format!(
                "no column '{column}' in this grid"
            )));
        }
        let dialect = tab
            .payload
            .get("dialect")
            .and_then(|v| v.as_str())
            .and_then(super::parse_dialect)
            .unwrap_or(nook_core::Dialect::Sqlite);
        // Clicking the same column again flips the direction.
        let current = tab.grid.order.clone();
        let wanted = format!(
            "{} {}",
            nook_core::quote_ident(&column, dialect),
            if desc { "DESC" } else { "ASC" }
        );
        tab.grid.order = if current == wanted {
            format!(
                "{} {}",
                nook_core::quote_ident(&column, dialect),
                if desc { "ASC" } else { "DESC" }
            )
        } else {
            wanted
        };
        tab.grid.page = 0;
        // The count described the request in hand, and this is a new one; `grid.countRows` refills it.
        tab.grid.total_rows = None;
    }
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn filter(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let where_clause = p.str_arg("where")?;
    {
        let tab = session
            .tabs
            .get_mut(index)
            .ok_or_else(|| Error::internal("tab is gone"))?;
        tab.grid.filter = where_clause;
        tab.grid.page = 0;
        // The count belongs to the rows the grid was asking for; this asks for a different set.
        tab.grid.total_rows = None;
    }
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn clear_filter(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    {
        let tab = session
            .tabs
            .get_mut(index)
            .ok_or_else(|| Error::internal("tab is gone"))?;
        tab.grid.filter.clear();
        tab.grid.page = 0;
        tab.grid.total_rows = None;
    }
    load_grid(session, index)?;
    Ok(grid_summary(session, index))
}

fn count_rows(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let (conn, schema, table, filter) = {
        let tab = &session.tabs[index];
        let table = tab
            .grid
            .table
            .clone()
            .ok_or_else(|| Error::config("this data tab is not bound to a table"))?;
        (
            tab.conn
                .ok_or_else(|| Error::config("this tab has no connection"))?,
            table.schema.clone(),
            table.table.clone(),
            tab.grid.filter.clone(),
        )
    };
    let total = {
        let driver = session.driver_for(conn)?;
        if filter.trim().is_empty() {
            driver.count_rows(schema.as_deref(), &table)?
        } else {
            let dialect = driver.dialect();
            let sql = format!(
                "SELECT count(*) FROM {} WHERE {}",
                nook_core::quote_qualified(schema.as_deref(), &table, dialect),
                filter.trim()
            );
            let rs = driver.run(&sql, 1)?;
            rs.rows
                .first()
                .and_then(|r| r.first())
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0) as u64
        }
    };
    if let Some(tab) = session.tabs.get_mut(index) {
        tab.grid.total_rows = Some(total);
    }
    Ok(json!({ "rows": total, "table": table }))
}

fn record_edit(
    session: &mut Session,
    tab_index: usize,
    kind: EditKind,
    row: usize,
    column: Option<String>,
    value: Option<Value>,
    insert_values: Vec<(String, Value)>,
) -> Result<serde_json::Value> {
    let sql = plan_edit(
        session,
        tab_index,
        kind,
        row,
        column.as_deref(),
        value.as_ref(),
        &insert_values,
    )?;
    let tab = session
        .tabs
        .get_mut(tab_index)
        .ok_or_else(|| Error::internal("tab is gone"))?;
    tab.grid.pending.push(crate::PendingEdit {
        kind,
        row,
        column: column.unwrap_or_default(),
        value: value.unwrap_or(Value::Null),
        insert: insert_values,
        sql: sql.clone(),
    });
    Ok(json!({
        "pending": tab.grid.pending.len(),
        "sql": sql,
        "kind": format!("{kind:?}").to_lowercase(),
    }))
}

/// The statement one grid edit becomes, planned against the table and the page as they are now: the
/// primary key values come from the page, the names and the dialect from the binding.
fn plan_edit(
    session: &Session,
    tab_index: usize,
    kind: EditKind,
    row: usize,
    column: Option<&str>,
    value: Option<&Value>,
    insert_values: &[(String, Value)],
) -> Result<String> {
    let dialect = session.tabs[tab_index]
        .payload
        .get("dialect")
        .and_then(|v| v.as_str())
        .and_then(super::parse_dialect)
        .unwrap_or(nook_core::Dialect::Sqlite);
    let tab = session
        .tabs
        .get(tab_index)
        .ok_or_else(|| Error::internal("tab is gone"))?;
    let table_ref = tab
        .grid
        .table
        .clone()
        .ok_or_else(|| Error::config("this data tab is not bound to a table"))?;
    let pk = match kind {
        EditKind::InsertRow => Vec::new(),
        _ => pk_values(tab, row)?,
    };
    let changes = match kind {
        EditKind::InsertRow => insert_values.to_vec(),
        EditKind::SetCell => vec![(
            column
                .ok_or_else(|| Error::config("pass \"column\""))?
                .to_string(),
            value.cloned().unwrap_or(Value::Null),
        )],
        EditKind::DeleteRow => Vec::new(),
    };
    let edit = RowEdit {
        kind,
        row_index: row,
        schema: table_ref.schema.clone(),
        table: table_ref.table.clone(),
        pk,
        changes,
    };
    Ok(plan_edits(std::slice::from_ref(&edit), dialect)?
        .first()
        .cloned()
        .unwrap_or_default())
}

/// A value that satisfies a NOT NULL column the server would otherwise reject.
fn fallback(column: &nook_model::Column) -> Value {
    let ty = column.type_name.to_ascii_uppercase();
    if ty.contains("INT") || ty.contains("SERIAL") {
        Value::Int(0)
    } else if ty.contains("BOOL") {
        Value::Bool(false)
    } else if ty.contains("REAL")
        || ty.contains("FLOAT")
        || ty.contains("NUMERIC")
        || ty.contains("DECIMAL")
    {
        Value::Float(0.0)
    } else {
        Value::text("")
    }
}

fn edit_cell(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    // Before the column's type is read: a schema edit since the tab was bound would coerce the new
    // value with the old type, and a stale page would plan the edit behind it.
    sync_grid_table(session, index)?;
    let row = p.int_arg("row").unwrap_or(0).max(0) as usize;
    let column = p.str_arg("column")?;
    let raw = p.cell("value").unwrap_or(Value::Null);
    // Coerce to the declared column type, so '42' becomes 42 for an int column.
    let value = {
        let tab = &session.tabs[index];
        match tab.grid.columns.iter().find(|c| c.name == column) {
            Some(column_info) => {
                Value::coerce(&raw.as_text(), &column_info.type_name).map_err(Error::config)?
            }
            None => raw,
        }
    };
    record_edit(
        session,
        index,
        EditKind::SetCell,
        row,
        Some(column),
        Some(value),
        vec![],
    )
}

fn set_null(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    sync_grid_table(session, index)?;
    let row = p.int_arg("row").unwrap_or(0).max(0) as usize;
    let column = p.str_arg("column")?;
    record_edit(
        session,
        index,
        EditKind::SetCell,
        row,
        Some(column),
        Some(Value::Null),
        vec![],
    )
}

fn insert_row(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    // The NOT NULL columns the empty row is filled from come from the binding.
    sync_grid_table(session, index)?;
    let values = match p.get("values") {
        Some(serde_json::Value::Object(map)) => map
            .iter()
            .map(|(k, v)| (k.clone(), Value::from_json(v)))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    // An empty row still needs a statement: fill the columns the server would reject when missing.
    let values = if values.is_empty() {
        let tab = &session.tabs[index];
        tab.grid
            .columns
            .iter()
            .filter(|c| !c.auto_increment && !c.nullable && c.default.is_none())
            .map(|c| (c.name.clone(), fallback(c)))
            .collect()
    } else {
        values
    };
    record_edit(session, index, EditKind::InsertRow, 0, None, None, values)
}

fn delete_row(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    sync_grid_table(session, index)?;
    let row = p.int_arg("row").unwrap_or(0).max(0) as usize;
    {
        let tab = &session.tabs[index];
        if row >= tab.grid.result.row_count() {
            return Err(Error::not_found(format!("no row {row} on this page")));
        }
    }
    record_edit(session, index, EditKind::DeleteRow, row, None, None, vec![])
}

fn duplicate_row(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    sync_grid_table(session, index)?;
    let row = p.int_arg("row").unwrap_or(0).max(0) as usize;
    let values: Vec<(String, Value)> = {
        let tab = &session.tabs[index];
        if row >= tab.grid.result.row_count() {
            return Err(Error::not_found(format!("no row {row} on this page")));
        }
        tab.grid
            .result
            .columns
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                // Generated keys are left out so the copy gets its own.
                !tab.grid
                    .columns
                    .iter()
                    .find(|c| &c.name == *name)
                    .map(|c| c.auto_increment)
                    .unwrap_or(false)
            })
            .map(|(i, name)| {
                (
                    name.clone(),
                    tab.grid.result.rows[row]
                        .get(i)
                        .cloned()
                        .unwrap_or(Value::Null),
                )
            })
            .collect()
    };
    record_edit(session, index, EditKind::InsertRow, 0, None, None, values)
}

fn pending(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let tab = &session.tabs[index];
    Ok(json!({
        "pending": tab.grid.pending.iter().map(|e| json!({
            "kind": format!("{:?}", e.kind).to_lowercase(),
            "row": e.row, "column": e.column, "sql": e.sql,
        })).collect::<Vec<_>>(),
        "dirty": tab.grid.dirty(),
    }))
}

fn revert(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let discarded = {
        let tab = session
            .tabs
            .get_mut(index)
            .ok_or_else(|| Error::internal("tab is gone"))?;
        let n = tab.grid.pending.len();
        tab.grid.pending.clear();
        n
    };
    Ok(json!({ "discarded": discarded }))
}

fn apply(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    // Plan every pending edit again, against the table and the page as they are now. A statement
    // planned before the table changed cannot be trusted: the columns it names may be gone, and a
    // stale `WHERE` matches no row while reporting the edit applied — the SQL that runs here is the
    // SQL the grid shows afterwards, and an edit that cannot be planned fails before anything runs.
    sync_grid_table(session, index)?;
    let pending = session.tabs[index].grid.pending.clone();
    let mut statements: Vec<String> = Vec::with_capacity(pending.len());
    for edit in &pending {
        let column = (!edit.column.is_empty()).then_some(edit.column.as_str());
        let sql = plan_edit(
            session,
            index,
            edit.kind,
            edit.row,
            column,
            Some(&edit.value),
            &edit.insert,
        )?;
        if let Some(slot) = session.tabs[index].grid.pending.get_mut(statements.len()) {
            slot.sql = sql.clone();
        }
        statements.push(sql);
    }
    let conn = session.tabs[index]
        .conn
        .ok_or_else(|| Error::config("this tab has no connection"))?;
    if statements.is_empty() {
        return Ok(json!({ "applied": 0, "message": "nothing to apply" }));
    }
    let in_transaction = !session.settings.auto_commit_edits;
    if in_transaction {
        session.driver_for(conn)?.begin()?;
    }
    let mut applied = 0usize;
    for (edit, sql) in pending.iter().zip(&statements) {
        // An `UPDATE` or a `DELETE` keyed by a primary key changes exactly one row, so a driver that
        // reports none changed nothing, and saying "applied" would be a lie the data disagrees with.
        // Without this, a key that moved under the tab makes the edit a silent no-op: SQLite reads a
        // double-quoted name that matches no column as a string literal, and the statement then
        // matches no row at all. A row that is gone (deleted elsewhere) lands here too, and the user
        // should hear about it rather than be told their edit landed.
        let one_row = matches!(edit.kind, EditKind::SetCell | EditKind::DeleteRow);
        let failure = match run_sql(session, conn, sql, Some(1)) {
            Ok(result) if one_row && result.affected == 0 => Some(Error::config(format!(
                "the edit changed no row, so nothing was applied: {sql}\n\
                 The row or the table may have moved under this tab — reload it and edit again."
            ))),
            Ok(_) => None,
            Err(e) => Some(e),
        };
        if let Some(e) = failure {
            if in_transaction {
                let _ = session.driver_for(conn).and_then(|d| d.rollback());
            }
            session.log(
                Level::Error,
                format!("edit {applied} of {} failed; rolled back", statements.len()),
            );
            return Err(e);
        }
        applied += 1;
    }
    if in_transaction {
        session.driver_for(conn)?.commit()?;
    }
    if let Some(tab) = session.tabs.get_mut(index) {
        tab.grid.pending.clear();
        // Rows the grid had counted have just been inserted, updated or deleted; the count described
        // the request before them, so it goes the way `filter`, `clear_filter` and `sort` send theirs.
        tab.grid.total_rows = None;
    }
    load_grid(session, index)?;
    session.log(Level::Success, format!("{applied} changes applied"));
    Ok(json!({ "applied": applied, "grid": grid_summary(session, index) }))
}

fn export(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let format = p.str_opt("format").unwrap_or_else(|| "csv".into());
    let text = render(&session.tabs[index].grid.result, &format)?;
    write_or_return(session, p, &format, &text)
}

fn export_table(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let conn = session.resolve_conn(p)?;
    let spec = p
        .str_opt("table")
        .ok_or_else(|| Error::config("pass \"table\""))?;
    let reference = TableRef::parse(&spec);
    let format = p.str_opt("format").unwrap_or_else(|| "csv".into());
    let limit = p
        .int_opt("limit", session.settings.max_rows as i64)
        .clamp(1, 5_000_000) as usize;
    let (dialect, result) = {
        let driver = session.driver_for(conn)?;
        let dialect = driver.dialect();
        let rs = driver.fetch_table(
            reference.schema.as_deref(),
            &reference.table,
            limit,
            0,
            None,
            None,
        )?;
        (dialect, rs)
    };
    let text = match format.as_str() {
        "sql" => result.to_sql_inserts(reference.schema.as_deref(), &reference.table, dialect),
        other => render(&result, other)?,
    };
    write_or_return(session, p, &format, &text)
}

fn render(result: &nook_model::ResultSet, format: &str) -> Result<String> {
    match format {
        "csv" => Ok(result.to_csv()),
        "json" => serde_json::to_string_pretty(&result.to_json())
            .map_err(|e| Error::internal(format!("json: {e}"))),
        "sql" => Err(Error::config(
            "exporting SQL needs a table; use grid.exportTable or dump_script",
        )),
        "markdown" | "md" => Ok(result.to_markdown()),
        other => Err(Error::config(format!(
            "unknown format '{other}'; try csv, json, sql or markdown"
        ))),
    }
}

fn write_or_return(
    session: &mut Session,
    p: &Params,
    format: &str,
    text: &str,
) -> Result<serde_json::Value> {
    match p.str_opt("path") {
        Some(path) => {
            let path = expand(&path, &session.home);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
            }
            std::fs::write(&path, text)
                .map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
            session.log(Level::Success, format!("exported to {}", path.display()));
            Ok(json!({
                "path": path.to_string_lossy(),
                "bytes": text.len(),
                "lines": text.lines().count(),
                "format": format,
            }))
        }
        None => Ok(json!({
            "format": format,
            "bytes": text.len(),
            "lines": text.lines().count(),
            "text": text,
        })),
    }
}

pub(crate) fn expand(path: &str, home: &std::path::Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(base) = std::env::var_os("HOME") {
            return PathBuf::from(base).join(rest);
        }
    }
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        home.join(candidate)
    }
}

fn find(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let needle = p.str_arg("text")?.to_ascii_lowercase();
    let column = p.str_opt("column");
    let tab = &session.tabs[index];
    let mut hits = Vec::new();
    for (row, values) in tab.grid.result.rows.iter().enumerate() {
        for (col, value) in values.iter().enumerate() {
            let name = tab.grid.result.columns.get(col);
            if let Some(want) = &column {
                if name.map(|n| n != want).unwrap_or(true) {
                    continue;
                }
            }
            if value.as_text().to_ascii_lowercase().contains(&needle) {
                hits.push(json!({
                    "row": row,
                    "column": name,
                    "value": value.display(),
                }));
            }
        }
    }
    Ok(json!({ "text": needle, "matches": hits }))
}

fn cell_value(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let row = p.int_arg("row").unwrap_or(0).max(0) as usize;
    let column = p.str_arg("column")?;
    let tab = &session.tabs[index];
    let value = tab
        .grid
        .result
        .value_at(row, &column)
        .cloned()
        .ok_or_else(|| Error::not_found(format!("no cell at row {row}, column {column}")))?;
    Ok(json!({ "value": value.to_json(), "display": value.display(), "type": value.type_name() }))
}
/// One record as a vertical form: every column with its declared type, its value and whether it is
/// a key. Read-only over the page the grid already holds, so it asks the server nothing — the same
/// row the grid painted, arranged for a table too wide to read across.
fn form_view(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    // The form offers a field per column, so it reads the same binding an edit plans against: without
    // this it lists the names and types the tab was opened with, and an edit made from a field the
    // table no longer has dies on the server instead of being refused here.
    sync_grid_table(session, index)?;
    let tab = &session.tabs[index];
    let grid = &tab.grid;
    // `row` is explicit. The engine's cursor is not kept in step with the grid's selection, so
    // guessing here would show a different record than the one the user clicked — the UI passes
    // the selected row instead.
    let row = p.int_opt("row", 0).max(0) as usize;
    let row_count = grid.result.rows.len();
    if row >= row_count {
        return Err(Error::not_found(format!(
            "no row {row} on this page (it holds {row_count} rows)"
        )));
    }
    // The form shows what `grid.apply` will send, not the page as it was fetched: a pending edit
    // to this row overlays the cell the form paints. Pending edits apply in order, so the last
    // one to a cell wins — the same rule the apply loop follows.
    let overlaid: std::collections::BTreeMap<&str, &Value> = grid
        .pending
        .iter()
        .filter(|e| e.row == row)
        .filter_map(|e| match e.kind {
            nook_model::EditKind::SetCell => Some((e.column.as_str(), &e.value)),
            _ => None,
        })
        .collect();
    let fields: Vec<serde_json::Value> = grid
        .columns
        .iter()
        .map(|column| {
            let value = overlaid
                .get(column.name.as_str())
                .copied()
                .or_else(|| grid.result.value_at(row, &column.name));
            json!({
                "name": column.name,
                "type": column.type_name,
                "value": value.map(|v| v.to_json()),
                "display": value.map(|v| v.display()).unwrap_or_default(),
                "primary_key": grid.primary_key.iter().any(|key| *key == column.name),
            })
        })
        .collect();
    Ok(json!({
        "row": row,
        "rows": row_count,
        // The grid's own gate, so the form never offers a field the engine would refuse.
        "editable": grid.editable(),
        "fields": fields,
    }))
}

fn copy_as_sql(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let index = active_grid_tab(session)?;
    let tab = &session.tabs[index];
    let table = tab
        .grid
        .table
        .clone()
        .ok_or_else(|| Error::config("this data tab is not bound to a table"))?;
    let dialect = tab
        .payload
        .get("dialect")
        .and_then(|v| v.as_str())
        .and_then(super::parse_dialect)
        .unwrap_or(nook_core::Dialect::Sqlite);
    let sql = tab
        .grid
        .result
        .to_sql_inserts(table.schema.as_deref(), &table.table, dialect);
    Ok(json!({
        "statements": sql.lines().count(),
        "sql": sql,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nook_core::Value;
    use nook_model::{Column, ResultSet};

    fn grid_tab() -> (Session, usize) {
        let mut session = Session::new();
        let conn = nook_core::ConnectionConfig::new("mem", nook_core::DriverKind::Sqlite);
        let mut conn = crate::Conn::new(conn);
        // The grid's columns and key are read from the catalog, so the connection carries the same
        // table the tab claims to show: a tab bound to nothing would edit nothing.
        let mut id = Column::new("id", "INTEGER");
        id.primary_key = true;
        let mut users = nook_model::Table::new(None, "users");
        users.columns = vec![id, Column::new("email", "TEXT")];
        conn.catalog = Some(nook_model::Catalog {
            driver: "SQLite".into(),
            schemas: vec![nook_model::Schema {
                name: "main".into(),
                tables: vec![users],
            }],
            ..nook_model::Catalog::default()
        });
        session.conns.push(conn);
        let index = session.add_tab(TabKind::Data, "users", Some(0));
        let tab = &mut session.tabs[index];
        tab.grid.result = ResultSet {
            columns: vec!["id".into(), "email".into()],
            column_types: vec!["INTEGER".into(), "TEXT".into()],
            rows: vec![
                vec![Value::Int(1), Value::text("a@b.c")],
                vec![Value::Int(2), Value::text("d@e.f")],
            ],
            ..ResultSet::default()
        };
        tab.grid.table = Some(TableRef {
            schema: None,
            table: "users".into(),
        });
        tab.grid.primary_key = vec!["id".into()];
        tab.grid.columns = vec![Column::new("id", "INTEGER"), Column::new("email", "TEXT")];
        tab.payload.insert("dialect".into(), json!("sqlite"));
        let index = session.tabs.len() - 1;
        (session, index)
    }

    /// One page of a table is one statement, and it is written in one place: the grid's `SELECT`
    /// and the inspector's preview must never drift apart.
    #[test]
    fn a_page_select_quotes_the_table_and_pages_it() {
        let table = TableRef {
            schema: Some("main".into()),
            table: "customers".into(),
        };
        // SQLite has no schemas of its own, so the schema drops out there and stays in Postgres.
        assert_eq!(
            crate::cmd::page_select(
                &table,
                nook_core::Dialect::Sqlite,
                "  status = 'trial'  ",
                " id DESC ",
                50,
                100,
            ),
            "SELECT * FROM \"customers\" WHERE status = 'trial' ORDER BY id DESC LIMIT 50 OFFSET 100"
        );
        assert_eq!(
            crate::cmd::page_select(&table, nook_core::Dialect::Postgres, "", "", 50, 100,),
            "SELECT * FROM \"main\".\"customers\" LIMIT 50 OFFSET 100"
        );
        // No filter, no order, no schema: the plain first page.
        let bare = TableRef {
            schema: None,
            table: "users".into(),
        };
        assert_eq!(
            crate::cmd::page_select(&bare, nook_core::Dialect::Sqlite, "", "", 0, 0),
            "SELECT * FROM \"users\" LIMIT 1 OFFSET 0"
        );
    }

    #[test]
    fn editing_a_cell_records_sql_and_coerces_types() {
        let (mut session, _) = grid_tab();
        let p =
            Params::from_json(json!({"row": 0, "column": "id", "value": "42"})).expect("params");
        let out = edit_cell(&mut session, &p).expect("edit");
        assert_eq!(out["pending"], 1);
        assert!(out["sql"]
            .as_str()
            .unwrap_or("")
            .contains("SET \"id\" = 42"));
        assert!(out["sql"]
            .as_str()
            .unwrap_or("")
            .contains("WHERE \"id\" = 1"));
    }

    #[test]
    fn editing_without_a_primary_key_is_refused() {
        let (mut session, tab) = grid_tab();
        // The key comes from the catalog, so a keyless table is one the catalog has no key for —
        // and the set of columns the tab would plan against has to match it, or the page is reloaded.
        let table = session.conns[0]
            .catalog
            .as_mut()
            .and_then(|c| c.schemas.get_mut(0))
            .and_then(|s| s.tables.first_mut())
            .expect("the fixture holds a table");
        table.columns[0].primary_key = false;
        session.tabs[tab].grid.primary_key.clear();
        let p =
            Params::from_json(json!({"row": 0, "column": "email", "value": "x"})).expect("params");
        let err = edit_cell(&mut session, &p).expect_err("a keyless table is refused");
        assert!(
            err.to_string().contains("primary key"),
            "the refusal has to name the reason: {err}"
        );
    }

    #[test]
    fn revert_and_pending_report_the_buffer() {
        let (mut session, tab) = grid_tab();
        let p =
            Params::from_json(json!({"row": 1, "column": "email", "value": "z"})).expect("params");
        edit_cell(&mut session, &p).expect("edit");
        let out = pending(&mut session, &Params::none()).expect("pending");
        assert_eq!(out["dirty"], true);
        let out = revert(&mut session, &Params::none()).expect("revert");
        assert_eq!(out["discarded"], 1);
        assert!(!session.tabs[tab].grid.dirty());
    }

    #[test]
    fn exporting_the_grid_writes_a_file() {
        let (mut session, _) = grid_tab();
        let dir = std::env::temp_dir().join("nook_grid_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("rows.csv").to_string_lossy().into_owned();
        let p = Params::from_json(json!({"format": "csv", "path": path})).expect("params");
        let out = export(&mut session, &p).expect("export");
        assert_eq!(out["lines"], 3);
        assert!(std::fs::read_to_string(&path)
            .expect("read")
            .contains("a@b.c"));
        let _ = std::fs::remove_file(&path);
    }

    /// The sentence a surface that cannot run a grid command quotes is the one the command answers
    /// with: the results register's Export is painted disabled on a query tab, and the reason it
    /// hovers is this string, read from here rather than copied.
    #[test]
    fn the_no_grid_refusal_is_the_sentence_export_answers_with() {
        let mut session = Session::new();
        let index = session.add_tab(TabKind::Query, "Query", None);
        session.active_tab = Some(index);
        let p = Params::from_json(json!({"format": "csv"})).expect("params");
        let e = export(&mut session, &p).expect_err("a query tab has no data grid to export");
        assert_eq!(e.message(), no_grid_reason());
    }
    /// The form must show what `grid.apply` will send, not the page as it was fetched: an edit
    /// recorded through `grid.editCell` overlays the cell the form paints, or the edit looks lost
    /// until the apply and the refetch.
    #[test]
    fn the_form_view_shows_pending_edits_over_the_page() {
        let (mut session, _) = grid_tab();
        session
            .run(
                "grid.editCell",
                json!({"row": 1, "column": "email", "value": "new@e.f"}),
            )
            .expect("edit");
        let out = session
            .run("grid.formView", json!({"row": 1}))
            .expect("form view");
        assert_eq!(out["fields"][1]["value"], "new@e.f", "{:#?}", out);
        assert_eq!(out["fields"][1]["display"], "new@e.f");
        // A row the edit does not touch still shows the page value.
        let other = session
            .run("grid.formView", json!({"row": 0}))
            .expect("form view");
        assert_eq!(other["fields"][1]["value"], "a@b.c");
    }

    /// The last edit to a cell wins, matching what `grid.apply` sends: the pending list is applied
    /// in order, so a second edit to the same cell must not be hidden by the first.
    #[test]
    fn the_form_view_prefers_the_last_pending_edit_to_a_cell() {
        let (mut session, _) = grid_tab();
        for email in ["one@e.f", "two@e.f"] {
            session
                .run(
                    "grid.editCell",
                    json!({"row": 0, "column": "email", "value": email}),
                )
                .expect("edit");
        }
        let out = session
            .run("grid.formView", json!({"row": 0}))
            .expect("form view");
        assert_eq!(out["fields"][1]["value"], "two@e.f");
    }

    #[test]
    fn find_scans_visible_cells() {
        let (mut session, _) = grid_tab();
        let p = Params::from_json(json!({"text": "e.f"})).expect("params");
        let out = find(&mut session, &p).expect("find");
        assert_eq!(out["matches"].as_array().map(Vec::len), Some(1));
        let p = Params::from_json(json!({"text": "nothing"})).expect("params");
        assert_eq!(
            find(&mut session, &p).expect("find")["matches"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
    }

    /// `grid.countRows` counts the rows the grid is currently asking for, so the moment the filter,
    /// the order or the rows themselves change the old count stops describing the page: it must not
    /// be kept and printed as the new match total. `set_page_size` and the pager reload the same
    /// slice, so they leave it standing deliberately — only the commands that change *which* rows
    /// are asked for clear it.
    #[test]
    fn changing_the_filter_or_the_order_drops_a_stale_row_count() {
        let (mut session, tab) = grid_tab();
        session.tabs[tab].grid.total_rows = Some(12_481);
        let p = Params::from_json(json!({"column": "id"})).expect("params");
        sort(&mut session, &p).expect("sort");
        assert!(session.tabs[tab].grid.total_rows.is_none());

        session.tabs[tab].grid.total_rows = Some(12_481);
        let p = Params::from_json(json!({"where": "status = 'live'"})).expect("params");
        let out = filter(&mut session, &p).expect("filter");
        assert!(session.tabs[tab].grid.total_rows.is_none());
        assert!(
            out["total_rows"].is_null(),
            "the summary printed the pre-filter count as the current match total"
        );

        session.tabs[tab].grid.total_rows = Some(12_481);
        clear_filter(&mut session, &Params::none()).expect("clear");
        assert!(session.tabs[tab].grid.total_rows.is_none());
    }
    /// The form view is one record as a vertical form: every column with its type, its value and
    /// whether it is a key. It reads the page the grid already holds, so it asks the server nothing.
    #[test]
    fn the_form_view_reports_one_record_as_fields() {
        let (mut session, _) = grid_tab();
        let out = session
            .run("grid.formView", json!({"row": 1}))
            .expect("form view");
        assert_eq!(out["row"], 1);
        assert_eq!(out["editable"], true);
        let fields = out["fields"].as_array().expect("a field per column");
        assert_eq!(fields.len(), 2, "{:#?}", fields);
        assert_eq!(fields[0]["name"], "id");
        assert_eq!(fields[0]["type"], "INTEGER");
        assert_eq!(fields[0]["value"], 2);
        assert_eq!(fields[0]["primary_key"], true);
        assert_eq!(fields[1]["name"], "email");
        assert_eq!(fields[1]["value"], "d@e.f");
        assert_eq!(fields[1]["primary_key"], false);
    }

    /// A row the page does not hold is an error, never a panic and never a silent empty form: the
    /// grid would then edit a record that is not there.
    #[test]
    fn the_form_view_rejects_a_row_the_page_does_not_hold() {
        let (mut session, _) = grid_tab();
        assert!(session.run("grid.formView", json!({"row": 99})).is_err());
    }

    #[test]
    fn counting_rows_without_a_connection_is_an_error() {
        let (mut session, _) = grid_tab();
        let e = count_rows(&mut session, &Params::none());
        assert!(e.is_err());
    }
}
