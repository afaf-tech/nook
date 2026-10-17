//! Data movement: transfer between any two connected databases, row-level sync, and dump scripts.
//! Everything here goes through the `Driver` trait, so SQLite → Postgres → MySQL combinations all work.

use std::time::Instant;

use nook_core::{quote_qualified, sql_literal, Dialect, DriverKind, Error, Result, Value};
use nook_model::{ddl, Catalog, ResultSet, Table};

use crate::Driver;

/// How many rows one page of a transfer holds, and one transaction commits.
pub const PAGE_ROWS: usize = 2_000;

#[derive(Debug, Clone, PartialEq)]
pub struct TransferOptions {
    /// Create target tables that do not exist, from the source catalog.
    pub create_tables: bool,
    /// Drop the target table first when it exists.
    pub drop_target: bool,
    /// Delete the target's rows before copying.
    pub truncate_target: bool,
    pub include_data: bool,
    pub include_indexes: bool,
    /// Optional `WHERE` for the source read.
    pub filter: String,
    /// Stop at the first failing table instead of carrying on.
    pub fail_fast: bool,
    /// Upper bound per table, so one huge table cannot run forever.
    pub max_rows: usize,
    /// `src=target` column pairs applied when copying; empty means positional copy.
    pub column_mapping: Vec<(String, String)>,
}

impl Default for TransferOptions {
    fn default() -> Self {
        TransferOptions {
            create_tables: true,
            drop_target: false,
            truncate_target: false,
            include_data: true,
            include_indexes: true,
            filter: String::new(),
            fail_fast: false,
            max_rows: 1_000_000,
            column_mapping: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableReport {
    pub table: String,
    pub rows: u64,
    pub statements: u64,
    pub skipped: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransferReport {
    pub tables: Vec<TableReport>,
    pub elapsed_ms: u128,
}

impl TransferReport {
    pub fn rows(&self) -> u64 {
        self.tables.iter().map(|t| t.rows).sum()
    }
    pub fn errors(&self) -> Vec<&TableReport> {
        self.tables.iter().filter(|t| t.error.is_some()).collect()
    }
    pub fn summary(&self) -> String {
        format!(
            "{} tables, {} rows, {} statements, {} failed, {} ms",
            self.tables.len(),
            self.rows(),
            self.tables.iter().map(|t| t.statements).sum::<u64>(),
            self.errors().len(),
            self.elapsed_ms
        )
    }
}

/// Rebuild a page in the mapped column order: `src=target` pairs name the page's source columns
/// and the target table columns they become. The result's columns are the targets, so
/// `to_sql_inserts` writes `INSERT INTO <table> (targets…)` against the target's own layout.
fn mapped_page(
    page: &nook_model::ResultSet,
    mapping: &[(String, String)],
    table: &Table,
) -> Result<nook_model::ResultSet> {
    let mut order = Vec::with_capacity(mapping.len());
    let mut columns = Vec::with_capacity(mapping.len());
    for (source, target) in mapping {
        let position = page.index_of(source).ok_or_else(|| {
            nook_core::Error::config(format!("the source table has no column '{source}'"))
        })?;
        if !table.columns.iter().any(|c| &c.name == target) {
            return Err(nook_core::Error::config(format!(
                "the table {} has no column '{target}'",
                table.key()
            )));
        }
        order.push(position);
        columns.push(target.clone());
    }
    let rows = page
        .rows
        .iter()
        .map(|row| {
            order
                .iter()
                .map(|i| row.get(*i).cloned().unwrap_or(nook_core::Value::Null))
                .collect()
        })
        .collect();
    Ok(nook_model::ResultSet {
        columns,
        rows,
        ..nook_model::ResultSet::default()
    })
}

/// Read one table from the source into pages, then write them to the target.
pub fn transfer_table(
    src: &mut dyn Driver,
    dst: &mut dyn Driver,
    table: &Table,
    dst_schema: Option<&str>,
    opts: &TransferOptions,
    progress: &mut dyn FnMut(&str),
) -> Result<TableReport> {
    let mut report = TableReport {
        table: table.key(),
        ..TableReport::default()
    };
    let dst_dialect = dst.dialect();

    if opts.drop_target {
        dst.run(
            &ddl::drop_table(dst_schema, &table.name, dst_dialect, true),
            0,
        )?;
        report.statements += 1;
    }

    if opts.create_tables && dst.table_ddl(dst_schema, &table.name).is_err() {
        dst.run(&ddl::create_table(table, dst_dialect), 0)?;
        report.statements += 1;
        if opts.include_indexes {
            for idx in ddl::create_indexes(table, dst_dialect) {
                if let Err(e) = dst.run(&idx, 0) {
                    if opts.fail_fast {
                        return Err(e);
                    }
                    progress(&format!("  index skipped: {e}"));
                } else {
                    report.statements += 1;
                }
            }
        }
    } else if opts.truncate_target {
        dst.run(
            &ddl::truncate_table(dst_schema, &table.name, dst_dialect),
            0,
        )?;
        report.statements += 1;
    }

    if !opts.include_data {
        return Ok(report);
    }

    // Validate mapped targets against the DESTINATION's catalog once, above the row loop: the
    // target table may share the source's name with a different column layout, and a per-page
    // `dst.catalog()` would repeat a live query for every page. Runs after the create/truncate
    // DDL above, so the live catalog already sees the created table. A mapping therefore also
    // refuses a not-yet-existing target: the create DDL comes from the source schema and would
    // not carry the mapped column names.
    let dst_table = if opts.column_mapping.is_empty() {
        None
    } else {
        let catalog = dst.catalog()?;
        Some(
            catalog
                .find_table(dst_schema, &table.name)
                .ok_or_else(|| {
                    nook_core::Error::config(format!(
                        "the target table {} was not found in the destination catalog",
                        table.key()
                    ))
                })?
                .clone(),
        )
    };

    let mut offset = 0usize;
    let mut done = false;
    while !done {
        let page = src.fetch_table(
            table.schema.as_deref(),
            &table.name,
            PAGE_ROWS,
            offset,
            None,
            if opts.filter.trim().is_empty() {
                None
            } else {
                Some(opts.filter.trim())
            },
        )?;
        if page.rows.is_empty() {
            break;
        }
        // A mapping renames and reorders: the page's own column names would otherwise become the
        // INSERT column list. Rebuild the page in target order first; the mapped page MUST be
        // bound back to `page`, otherwise the mapping is silently discarded.
        let page = if let Some(dst_table) = &dst_table {
            match mapped_page(&page, &opts.column_mapping, dst_table) {
                Ok(p) => p,
                Err(e) => {
                    if opts.fail_fast {
                        return Err(e);
                    }
                    report.error = Some(e.to_string());
                    progress(&format!("  mapping failed: {e}"));
                    return Ok(report);
                }
            }
        } else {
            page
        };
        let fetched = page.rows.len();
        let script = page.to_sql_inserts(
            dst_schema.or(table.schema.as_deref()),
            &table.name,
            dst_dialect,
        );
        dst.begin()?;
        for stmt in script.lines().filter(|l| !l.trim().is_empty()) {
            if let Err(e) = dst.run(stmt.trim_end_matches(';'), 0) {
                let _ = dst.rollback();
                if opts.fail_fast {
                    return Err(e);
                }
                report.error = Some(e.to_string());
                progress(&format!("  page at row {offset} failed: {e}"));
                return Ok(report);
            }
            report.statements += 1;
        }
        dst.commit()?;
        report.rows += fetched as u64;
        offset += fetched;
        done = fetched < PAGE_ROWS || offset >= opts.max_rows;
        progress(&format!(
            "  {}: {}/{} rows",
            table.key(),
            report.rows,
            if done { report.rows } else { 0 }
        ));
    }
    Ok(report)
}

/// Transfer every named table (`*` or empty means every table in the source catalog).
pub fn transfer(
    src: &mut dyn Driver,
    dst: &mut dyn Driver,
    src_catalog: &Catalog,
    tables: &[String],
    dst_schema: Option<&str>,
    opts: &TransferOptions,
    progress: &mut dyn FnMut(&str),
) -> Result<TransferReport> {
    let started = Instant::now();
    let selected: Vec<Table> = src_catalog
        .schemas
        .iter()
        .flat_map(|s| s.tables.iter())
        .filter(|t| t.kind == nook_model::TableKind::Table)
        .filter(|t| {
            tables.is_empty()
                || tables.iter().any(|w| w == "*")
                || tables.iter().any(|w| w == &t.name || w == &t.key())
        })
        .cloned()
        .collect();
    if selected.is_empty() {
        return Err(Error::not_found(
            "no tables matched; pass table names or '*' for all",
        ));
    }
    let mut report = TransferReport::default();
    for table in &selected {
        progress(&format!("transferring {}", table.key()));
        let result = transfer_table(src, dst, table, dst_schema, opts, progress);
        match result {
            Ok(r) => report.tables.push(r),
            Err(e) => {
                report.tables.push(TableReport {
                    table: table.key(),
                    error: Some(e.to_string()),
                    ..TableReport::default()
                });
                if opts.fail_fast {
                    report.elapsed_ms = started.elapsed().as_millis();
                    return Ok(report);
                }
            }
        }
    }
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}

// ---------------------------------------------------------------- row-level sync

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowChangeKind {
    Insert,
    Update,
    Delete,
}

impl RowChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            RowChangeKind::Insert => "insert",
            RowChangeKind::Update => "update",
            RowChangeKind::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowChange {
    pub kind: RowChangeKind,
    pub key: Vec<(String, Value)>,
    /// Columns to write: the full row for an insert, only what differs for an update.
    pub values: Vec<(String, Value)>,
    /// Previous values of the changed columns, for the report.
    pub was: Vec<(String, Value)>,
}

/// Pure row comparison: what a data sync would change. No database involved, so it is easy to test.
pub fn compare_rows(
    source: &ResultSet,
    target: &ResultSet,
    pk_columns: &[String],
) -> Result<Vec<RowChange>> {
    if pk_columns.is_empty() {
        return Err(Error::unsupported(
            "no primary key: a data sync cannot match rows without one",
        ));
    }
    let src_pk_idx: Vec<usize> = pk_columns
        .iter()
        .map(|c| {
            source
                .index_of(c)
                .ok_or_else(|| Error::internal(format!("source is missing key column {c}")))
        })
        .collect::<Result<_>>()?;
    let dst_pk_idx: Vec<usize> = pk_columns
        .iter()
        .map(|c| {
            target
                .index_of(c)
                .ok_or_else(|| Error::internal(format!("target is missing key column {c}")))
        })
        .collect::<Result<_>>()?;

    let key_of = |rs: &ResultSet, idx: &[usize], row: &[Value]| -> String {
        let _ = rs;
        idx.iter()
            .map(|i| row.get(*i).cloned().unwrap_or(Value::Null).display())
            .collect::<Vec<_>>()
            .join("\u{1f}")
    };

    let mut inserts = Vec::new();
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::with_capacity(target.rows.len());
    for row in &target.rows {
        seen.push(key_of(target, &dst_pk_idx, row));
    }

    for row in &source.rows {
        let key = key_of(source, &src_pk_idx, row);
        let key_values: Vec<(String, Value)> = pk_columns
            .iter()
            .enumerate()
            .map(|(n, c)| {
                (
                    c.clone(),
                    row.get(src_pk_idx[n]).cloned().unwrap_or(Value::Null),
                )
            })
            .collect();
        match seen.iter().position(|k| *k == key) {
            None => {
                let values = source
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (c.clone(), row.get(i).cloned().unwrap_or(Value::Null)))
                    .collect();
                inserts.push(RowChange {
                    kind: RowChangeKind::Insert,
                    key: key_values,
                    values,
                    was: vec![],
                });
            }
            Some(pos) => {
                let target_row = &target.rows[pos];
                let mut changed = Vec::new();
                let mut was = Vec::new();
                for (i, col) in source.columns.iter().enumerate() {
                    let Some(ti) = target.index_of(col) else {
                        continue;
                    };
                    let want = row.get(i).cloned().unwrap_or(Value::Null);
                    let have = target_row.get(ti).cloned().unwrap_or(Value::Null);
                    if !values_equal(&want, &have) {
                        changed.push((col.clone(), want));
                        was.push((col.clone(), have));
                    }
                }
                if !changed.is_empty() {
                    updates.push(RowChange {
                        kind: RowChangeKind::Update,
                        key: key_values,
                        values: changed,
                        was,
                    });
                }
            }
        }
    }

    for (pos, row) in target.rows.iter().enumerate() {
        let _ = pos;
        let key = key_of(target, &dst_pk_idx, row);
        let in_source = source
            .rows
            .iter()
            .any(|r| key_of(source, &src_pk_idx, r) == key);
        if !in_source {
            let key_values: Vec<(String, Value)> = pk_columns
                .iter()
                .enumerate()
                .map(|(n, c)| {
                    (
                        c.clone(),
                        row.get(dst_pk_idx[n]).cloned().unwrap_or(Value::Null),
                    )
                })
                .collect();
            let values = target
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| (c.clone(), row.get(i).cloned().unwrap_or(Value::Null)))
                .collect();
            deletes.push(RowChange {
                kind: RowChangeKind::Delete,
                key: key_values,
                values,
                was: vec![],
            });
        }
    }
    // Grouped so the report reads as inserts, then updates, then deletes.
    out.extend(inserts);
    out.extend(updates);
    out.extend(deletes);
    Ok(out)
}

/// Numbers from different sources (`Int(1)` vs `Float(1.0)`) still count as equal.
fn values_equal(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    if a.is_null() || b.is_null() {
        return a.is_null() && b.is_null();
    }
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < f64::EPSILON,
        _ => a.as_text() == b.as_text(),
    }
}

/// Statements that turn the target into the source for the changed rows only.
pub fn sync_statements(
    changes: &[RowChange],
    schema: Option<&str>,
    table: &str,
    dialect: Dialect,
) -> Vec<String> {
    let mut out = Vec::new();
    for change in changes {
        let sql = match change.kind {
            RowChangeKind::Insert => {
                let cols = change
                    .values
                    .iter()
                    .map(|(c, _)| nook_core::quote_ident(c, dialect))
                    .collect::<Vec<_>>()
                    .join(", ");
                let vals = change
                    .values
                    .iter()
                    .map(|(_, v)| sql_literal(v, dialect))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "INSERT INTO {} ({cols}) VALUES ({vals});",
                    quote_qualified(schema, table, dialect)
                )
            }
            RowChangeKind::Update => {
                let sets = change
                    .values
                    .iter()
                    .map(|(c, v)| {
                        format!(
                            "{} = {}",
                            nook_core::quote_ident(c, dialect),
                            sql_literal(v, dialect)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let where_clause = change
                    .key
                    .iter()
                    .map(|(c, v)| {
                        if v.is_null() {
                            format!("{} IS NULL", nook_core::quote_ident(c, dialect))
                        } else {
                            format!(
                                "{} = {}",
                                nook_core::quote_ident(c, dialect),
                                sql_literal(v, dialect)
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ");
                format!(
                    "UPDATE {} SET {sets} WHERE {where_clause};",
                    quote_qualified(schema, table, dialect)
                )
            }
            RowChangeKind::Delete => {
                let where_clause = change
                    .key
                    .iter()
                    .map(|(c, v)| {
                        format!(
                            "{} = {}",
                            nook_core::quote_ident(c, dialect),
                            sql_literal(v, dialect)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ");
                format!(
                    "DELETE FROM {} WHERE {where_clause};",
                    quote_qualified(schema, table, dialect)
                )
            }
        };
        out.push(sql);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SyncReport {
    pub changes: Vec<(String, RowChangeKind)>,
    pub applied: u64,
    pub elapsed_ms: u128,
    /// Whether this run was a dry run. The label must come from what the caller asked for, not
    /// from `applied == 0`: an applied sync over tables that were already in sync also has zero
    /// applied statements and must not be mislabeled.
    pub dry_run: bool,
}

impl SyncReport {
    pub fn inserts(&self) -> usize {
        self.changes
            .iter()
            .filter(|(_, k)| *k == RowChangeKind::Insert)
            .count()
    }
    pub fn updates(&self) -> usize {
        self.changes
            .iter()
            .filter(|(_, k)| *k == RowChangeKind::Update)
            .count()
    }
    pub fn deletes(&self) -> usize {
        self.changes
            .iter()
            .filter(|(_, k)| *k == RowChangeKind::Delete)
            .count()
    }
    pub fn summary(&self) -> String {
        format!(
            "{} inserts, {} updates, {} deletes{}",
            self.inserts(),
            self.updates(),
            self.deletes(),
            if self.dry_run {
                " (dry run)".to_string()
            } else {
                format!(", {} applied", self.applied)
            }
        )
    }
}

/// Compare and (unless `dry_run`) apply the differences for the named tables.
/// The parameter list is long on purpose: it is the whole description of a sync, and the CLI, the
/// UI wizard and the job runner all build it from one place.
#[allow(clippy::too_many_arguments)]
pub fn sync_data(
    src: &mut dyn Driver,
    dst: &mut dyn Driver,
    src_catalog: &Catalog,
    tables: &[String],
    dst_schema: Option<&str>,
    batch: usize,
    dry_run: bool,
    progress: &mut dyn FnMut(&str),
) -> Result<SyncReport> {
    let started = Instant::now();
    let mut report = SyncReport {
        dry_run,
        ..SyncReport::default()
    };
    let selected: Vec<Table> = src_catalog
        .schemas
        .iter()
        .flat_map(|s| s.tables.iter())
        .filter(|t| t.kind == nook_model::TableKind::Table)
        .filter(|t| {
            tables.is_empty()
                || tables.iter().any(|w| w == "*")
                || tables.iter().any(|w| w == &t.name || w == &t.key())
        })
        .cloned()
        .collect();
    let dst_catalog = dst.catalog()?;
    let dialect = dst.dialect();

    for table in &selected {
        let pk = table.primary_key();
        if pk.is_empty() {
            progress(&format!("{}: skipped, no primary key", table.key()));
            continue;
        }
        let Some(dst_table) = dst_catalog.find_table(dst_schema, &table.name) else {
            progress(&format!("{}: missing in the target, skipped", table.key()));
            continue;
        };
        let dst_pk = dst_table.primary_key();
        if dst_pk != pk {
            progress(&format!(
                "{}: primary key differs ({} vs {}), skipped",
                table.key(),
                pk.join(", "),
                dst_pk.join(", ")
            ));
            continue;
        }
        let src_rows = src.fetch_table(
            table.schema.as_deref(),
            &table.name,
            crate::MAX_ROWS,
            0,
            None,
            None,
        )?;
        let dst_rows = dst.fetch_table(dst_schema, &table.name, crate::MAX_ROWS, 0, None, None)?;
        let changes = compare_rows(&src_rows, &dst_rows, &pk)?;
        if changes.is_empty() {
            progress(&format!("{}: in sync", table.key()));
            continue;
        }
        for c in &changes {
            report.changes.push((table.key(), c.kind));
        }
        progress(&format!(
            "{}: {} inserts, {} updates, {} deletes",
            table.key(),
            changes
                .iter()
                .filter(|c| c.kind == RowChangeKind::Insert)
                .count(),
            changes
                .iter()
                .filter(|c| c.kind == RowChangeKind::Update)
                .count(),
            changes
                .iter()
                .filter(|c| c.kind == RowChangeKind::Delete)
                .count()
        ));
        if dry_run {
            continue;
        }
        let statements = sync_statements(&changes, dst_schema, &table.name, dialect);
        for chunk in statements.chunks(batch.max(1)) {
            dst.begin()?;
            for stmt in chunk {
                dst.run(stmt, 0)?;
                report.applied += 1;
            }
            dst.commit()?;
        }
    }
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}

/// A portable `.sql` script for the named tables: schema, indexes, then rows.
/// Works for any driver, which is what makes SQLite backups and cross-engine exports the same code.
pub fn dump_script(
    driver: &mut dyn Driver,
    catalog: &Catalog,
    tables: &[String],
    schema_only: bool,
    max_rows: usize,
    progress: &mut dyn FnMut(&str),
) -> Result<String> {
    let dialect = driver.dialect();
    let kind = driver.kind();
    let mut out = String::new();
    out.push_str(&format!(
        "-- Nook dump of {} ({})\n-- {}\n\n",
        catalog
            .database
            .clone()
            .unwrap_or_else(|| "(no database)".into()),
        kind.label(),
        timestamp()
    ));
    if kind == DriverKind::MySql {
        out.push_str("SET FOREIGN_KEY_CHECKS=0;\n\n");
    }
    let selected: Vec<Table> = catalog
        .schemas
        .iter()
        .flat_map(|s| s.tables.iter())
        .filter(|t| t.kind == nook_model::TableKind::Table)
        .filter(|t| {
            tables.is_empty()
                || tables.iter().any(|w| w == "*")
                || tables.iter().any(|w| w == &t.name || w == &t.key())
        })
        .cloned()
        .collect();
    for table in &selected {
        progress(&format!("dumping {}", table.key()));
        out.push_str(&format!("\n-- {} ({})\n", table.key(), table.kind.label()));
        out.push_str(&ddl::create_table(table, dialect));
        out.push('\n');
        for idx in ddl::create_indexes(table, dialect) {
            out.push_str(&idx);
            out.push('\n');
        }
        for fk in ddl::create_foreign_keys(table, dialect) {
            out.push_str(&fk);
            out.push('\n');
        }
        if schema_only {
            continue;
        }
        let mut offset = 0usize;
        let mut total = 0usize;
        loop {
            let page = driver.fetch_table(
                table.schema.as_deref(),
                &table.name,
                PAGE_ROWS,
                offset,
                None,
                None,
            )?;
            if page.rows.is_empty() {
                break;
            }
            let fetched = page.rows.len();
            out.push_str(&page.to_sql_inserts(table.schema.as_deref(), &table.name, dialect));
            total += fetched;
            offset += fetched;
            if fetched < PAGE_ROWS || total >= max_rows {
                break;
            }
        }
        out.push('\n');
    }
    out.push_str(&format!("\n-- {} tables\n", selected.len()));
    Ok(out)
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix {secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::Sqlite;

    fn rs(cols: &[&str], rows: Vec<Vec<Value>>) -> ResultSet {
        ResultSet {
            columns: cols.iter().map(|c| c.to_string()).collect(),
            column_types: cols.iter().map(|_| "text".to_string()).collect(),
            rows,
            ..ResultSet::default()
        }
    }

    #[test]
    fn compare_detects_insert_update_delete() {
        let source = rs(
            &["id", "name"],
            vec![
                vec![Value::Int(1), Value::text("same")],
                vec![Value::Int(2), Value::text("changed in source")],
                vec![Value::Int(3), Value::text("new")],
            ],
        );
        let target = rs(
            &["id", "name"],
            vec![
                vec![Value::Int(1), Value::text("same")],
                vec![Value::Int(2), Value::text("old")],
                vec![Value::Int(4), Value::text("gone")],
            ],
        );
        let changes = compare_rows(&source, &target, &["id".to_string()]).expect("compare");
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].kind, RowChangeKind::Insert);
        assert_eq!(changes[1].kind, RowChangeKind::Update);
        assert_eq!(
            changes[1].values,
            vec![("name".into(), Value::text("changed in source"))]
        );
        assert_eq!(changes[1].was, vec![("name".into(), Value::text("old"))]);
        assert_eq!(changes[2].kind, RowChangeKind::Delete);
    }

    #[test]
    fn compare_needs_a_key_and_tolerates_number_shapes() {
        let a = rs(&["id"], vec![vec![Value::Int(1)]]);
        assert!(compare_rows(&a, &a, &[]).is_err());
        let src = rs(&["id", "n"], vec![vec![Value::Int(1), Value::Int(5)]]);
        let dst = rs(&["id", "n"], vec![vec![Value::Int(1), Value::Float(5.0)]]);
        assert!(compare_rows(&src, &dst, &["id".to_string()])
            .expect("compare")
            .is_empty());
    }

    #[test]
    fn sync_statements_are_escaped_and_qualified() {
        let change = RowChange {
            kind: RowChangeKind::Update,
            key: vec![("id".into(), Value::Int(7))],
            values: vec![("note".into(), Value::text("o'brien"))],
            was: vec![("note".into(), Value::text("old"))],
        };
        let sql = sync_statements(&[change], Some("public"), "users", Dialect::Postgres);
        assert_eq!(
            sql[0],
            "UPDATE \"public\".\"users\" SET \"note\" = 'o''brien' WHERE \"id\" = 7;"
        );
    }

    #[test]
    fn transfer_between_two_sqlite_databases() {
        let mut src = Sqlite::memory().expect("src");
        src.run_script(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);\
             INSERT INTO users (id, email) VALUES (1,'a@b.c'),(2,'d@e.f');",
            10,
        )
        .expect("setup");
        let mut dst = Sqlite::memory().expect("dst");
        let catalog = src.catalog().expect("catalog");
        let opts = TransferOptions::default();
        let report = transfer(
            &mut src,
            &mut dst,
            &catalog,
            &["users".to_string()],
            None,
            &opts,
            &mut |_| {},
        )
        .expect("transfer");
        assert_eq!(report.rows(), 2);
        assert!(report.errors().is_empty());
        let check = dst.run("SELECT count(*) FROM users", 1).expect("count");
        assert_eq!(check.rows[0][0], Value::Int(2));
    }

    #[test]
    fn sync_between_two_sqlite_databases_applies_the_difference() {
        let mut src = Sqlite::memory().expect("src");
        src.run_script(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);\
             INSERT INTO t VALUES (1,'a'),(2,'b');",
            10,
        )
        .expect("setup src");
        let mut dst = Sqlite::memory().expect("dst");
        dst.run_script(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);\
             INSERT INTO t VALUES (1,'a'),(3,'c');",
            10,
        )
        .expect("setup dst");
        let catalog = src.catalog().expect("catalog");
        let dry = sync_data(
            &mut src,
            &mut dst,
            &catalog,
            &["t".to_string()],
            None,
            100,
            true,
            &mut |_| {},
        )
        .expect("dry run");
        assert_eq!(dry.applied, 0);
        assert_eq!(dry.inserts(), 1);
        assert_eq!(dry.deletes(), 1);
        // The label comes from what the caller asked for, not from `applied == 0`.
        assert!(dry.summary().contains("(dry run)"));
        let applied = sync_data(
            &mut src,
            &mut dst,
            &catalog,
            &["t".to_string()],
            None,
            100,
            false,
            &mut |_| {},
        )
        .expect("apply");
        assert_eq!(applied.applied, 2);
        assert!(!applied.summary().contains("(dry run)"));
        // An apply over already-in-sync tables has zero applied statements and must still
        // not be mislabeled — the exact case the live battery caught.
        let in_sync = sync_data(
            &mut src,
            &mut dst,
            &catalog,
            &["t".to_string()],
            None,
            100,
            false,
            &mut |_| {},
        )
        .expect("apply over in-sync tables");
        assert_eq!(in_sync.applied, 0);
        assert!(!in_sync.summary().contains("(dry run)"));
        let rows = dst
            .run("SELECT id FROM t ORDER BY id", 10)
            .expect("read back");
        assert_eq!(rows.rows.len(), 2);
        assert_eq!(rows.rows[0][0], Value::Int(1));
        assert_eq!(rows.rows[1][0], Value::Int(2));
    }

    #[test]
    fn dump_script_restores_into_a_fresh_database() {
        let mut db = Sqlite::memory().expect("db");
        db.run_script(
            "CREATE TABLE a (id INTEGER PRIMARY KEY, v TEXT);\
             INSERT INTO a VALUES (1,'x');\
             CREATE VIEW va AS SELECT * FROM a;",
            10,
        )
        .expect("setup");
        let catalog = db.catalog().expect("catalog");
        let script = dump_script(&mut db, &catalog, &[], false, 1000, &mut |_| {}).expect("dump");
        assert!(script.contains("CREATE TABLE \"a\""));
        assert!(script.contains("INSERT INTO \"a\""));
        let mut fresh = Sqlite::memory().expect("fresh");
        for stmt in crate::split_script(&script) {
            let _ = fresh.run(&stmt, 10).expect("replay");
        }
        assert_eq!(fresh.count_rows(None, "a").expect("count"), 1);
    }

    #[test]
    fn schema_only_dump_has_no_rows() {
        let mut db = Sqlite::memory().expect("db");
        db.run_script(
            "CREATE TABLE a (id INTEGER PRIMARY KEY, v TEXT); INSERT INTO a VALUES (1,'x');",
            10,
        )
        .expect("setup");
        let catalog = db.catalog().expect("catalog");
        let script = dump_script(&mut db, &catalog, &[], true, 10, &mut |_| {}).expect("dump");
        assert!(script.contains("CREATE TABLE"));
        assert!(!script.contains("INSERT INTO"));
    }

    #[test]
    fn unknown_table_selection_is_an_error() {
        let mut src = Sqlite::memory().expect("src");
        src.run("CREATE TABLE t (id int)", 1).expect("setup");
        let mut dst = Sqlite::memory().expect("dst");
        let catalog = src.catalog().expect("catalog");
        let e = transfer(
            &mut src,
            &mut dst,
            &catalog,
            &["nope".to_string()],
            None,
            &TransferOptions::default(),
            &mut |_| {},
        );
        assert!(e.is_err());
    }
}
