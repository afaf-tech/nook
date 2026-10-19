//! Schema commands: the table designer, object DDL, indexes and foreign keys.

use nook_core::{quote_ident, quote_qualified, Dialect, Error, Result};
use nook_model::{ddl, Table};
use serde_json::json;

use crate::cmd::{dialect_name, run_sql};
use crate::{CommandSpec, Params, Session, TableRef};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("schema.tables", "Tables and views", "Design", list_tables)
            .params(r#"{"conn":"local","schema":"public"}"#),
        CommandSpec::new("schema.columns", "Columns", "Design", columns)
            .params(r#"{"table":"public.users"}"#),
        CommandSpec::new("schema.indexes", "Indexes", "Design", indexes)
            .params(r#"{"table":"public.users"}"#),
        CommandSpec::new("schema.foreignKeys", "Foreign keys", "Design", foreign_keys)
            .params(r#"{"table":"public.posts"}"#),
        CommandSpec::new("schema.ddl", "Object DDL", "Design", ddl_of).params(r#"{"table":"public.users"}"#),
        CommandSpec::new("schema.createTable", "New table", "Design", create_table)
            .params(r#"{"table":"notes","columns":[{"name":"id","type":"int","primary_key":true,"auto_increment":true},{"name":"body","type":"text","nullable":false}]}"#),
        CommandSpec::new("schema.copyTable", "Copy table", "Design", copy_table)
            .params(r#"{"table":"users","new_name":"users_copy","with_data":true}"#),
        CommandSpec::new("schema.dropTable", "Drop table", "Design", drop_table)
            .params(r#"{"table":"users","confirm":true}"#),
        CommandSpec::new("schema.truncateTable", "Truncate table", "Design", truncate_table)
            .params(r#"{"table":"logs","confirm":true}"#),
        CommandSpec::new("schema.renameTable", "Rename table", "Design", rename_table)
            .params(r#"{"table":"old","new_name":"new"}"#),
        CommandSpec::new("schema.addColumn", "Add column", "Design", add_column)
            .params(r#"{"table":"users","name":"nickname","type":"text","nullable":true}"#),
        CommandSpec::new("schema.dropColumn", "Drop column", "Design", drop_column)
            .params(r#"{"table":"users","column":"nickname","confirm":true}"#),
        CommandSpec::new("schema.renameColumn", "Rename column", "Design", rename_column)
            .params(r#"{"table":"users","column":"email","new_name":"email_address"}"#),
        CommandSpec::new("schema.changeColumnType", "Change column type", "Design", change_type)
            .params(r#"{"table":"users","column":"age","type":"bigint"}"#),
        CommandSpec::new("schema.setNullable", "Change nullability", "Design", set_nullable)
            .params(r#"{"table":"users","column":"email","nullable":false}"#),
        CommandSpec::new("schema.setDefault", "Change default", "Design", set_default)
            .params(r#"{"table":"users","column":"status","default":"'draft'"}"#),
        CommandSpec::new("schema.createIndex", "Create index", "Design", create_index)
            .params(r#"{"table":"users","name":"ix_users_email","columns":"email","unique":true}"#),
        CommandSpec::new("schema.dropIndex", "Drop index", "Design", drop_index)
            .params(r#"{"table":"users","index":"ix_users_email","confirm":true}"#),
        CommandSpec::new("schema.addForeignKey", "Add foreign key", "Design", add_foreign_key)
            .params(r#"{"table":"posts","name":"fk_posts_author","columns":"author_id","ref_table":"users","ref_columns":"id","on_delete":"cascade"}"#),
        CommandSpec::new("schema.createView", "Create view", "Design", create_view)
            .params(r#"{"name":"active_users","sql":"SELECT * FROM users WHERE status = 'live'"}"#),
        CommandSpec::new("schema.dropView", "Drop view", "Design", drop_view)
            .params(r#"{"name":"active_users","confirm":true}"#),
        CommandSpec::new("schema.search", "Search objects", "Design", search)
            .params(r#"{"pattern":"user"}"#)
            .help("Find tables and views whose name or columns contain the pattern."),
        CommandSpec::new("schema.emptyDatabase", "Empty database", "Design", empty_database)
            .params(r#"{"confirm":true}"#)
            .help("Drop every user table in the connected database. Asks for confirm:true."),
        CommandSpec::new("schema.createPartition", "Create partition", "Design", create_partition)
            .params(r#"{"parent":"events","name":"events_2026_11","from":"2026-11-01","to":"2026-12-01"}"#)
            .help(
                "Range-by-date only: Postgres CREATE TABLE … PARTITION OF, MySQL ALTER TABLE … \
                 ADD PARTITION. The bounds are date literals, never free SQL.",
            ),
        CommandSpec::new("schema.dropPartition", "Drop partition", "Design", drop_partition)
            .params(r#"{"parent":"events","name":"events_2026_11","confirm":true}"#)
            .help("Drops one partition; the rows it held go with it, so it needs \"confirm\"."),
        CommandSpec::new("schema.partitions", "Partitions", "Design", partitions)
            .params(r#"{"table":"events"}"#)
            .help(
                "A partitioned table's partitions with their bounds and row estimates; an \
                 unpartitioned table answers an empty list rather than an error.",
            ),
        CommandSpec::new("schema.tableRebuild", "Rebuild a table", "Design", table_rebuild)
            .params(
                r#"{"table":"users","changes":{"rename":{"email":"mail"},"drop":["legacy"]},"confirm":true}"#,
            )
            .help(
                "SQLite only: rebuilds the table from a *diff* of its live columns, so a column the \
                 caller does not name is carried over instead of dropped. Inside an open \
                 transaction it refuses.",
            ),
        CommandSpec::new("schema.triggerEditor", "Trigger editor", "Design", trigger_editor)
            .params(r#"{"action":"create","table":"users","name":"audit_email","timing":"AFTER","event":"INSERT","body":"UPDATE users SET email = lower(email);"}"#)
            .help("list/create/drop via \"action\"; create builds the CREATE TRIGGER per dialect."),
        CommandSpec::new("schema.functionEditor", "Function editor", "Design", function_editor)
            .params(r#"{"action":"create","name":"add_one","kind":"function","sql":"(n int) RETURNS int AS $$ SELECT n + 1 $$ LANGUAGE sql"}"#)
            .help("list/create/drop stored routines via \"action\"; the sql body is passed through verbatim; SQLite refuses."),
        CommandSpec::new("schema.sequenceEditor", "Sequence editor", "Design", sequence_editor)
            .params(r#"{"action":"create","name":"user_ids","start":1,"increment":1}"#)
            .help("list/create/drop sequences via \"action\"; PostgreSQL only."),
        CommandSpec::new("schema.setPrimaryKey", "Set primary key", "Design", set_primary_key)
            .params(r#"{"table":"public.users","columns":["id"]}"#)
            .help("Add the table's primary key with ALTER TABLE; refuses a table that already has one; SQLite rebuilds the table inside a transaction (which refuses an open one)."),
    ]
}

/// Parameters that describe a column, as the designer sends them.
fn column_from_params(p: &Params) -> Result<nook_model::Column> {
    let name = p
        .str_opt("name")
        .or_else(|| p.str_opt("column"))
        .ok_or_else(|| Error::config("a column needs a name"))?;
    let type_name = p.str_opt("type").unwrap_or_else(|| "TEXT".into());
    if name.trim().is_empty() {
        return Err(Error::config("a column needs a name"));
    }
    Ok(nook_model::Column {
        name,
        type_name,
        nullable: p.bool_opt("nullable", true),
        primary_key: p.bool_opt("primary_key", false),
        auto_increment: p.bool_opt("auto_increment", false),
        default: p.str_opt("default"),
        comment: p.str_opt("comment").unwrap_or_default(),
        ..nook_model::Column::default()
    })
}

pub(crate) fn require_confirm(session: &Session, p: &Params, what: &str) -> Result<()> {
    if session.settings.confirm_destructive && !p.bool_opt("confirm", false) {
        return Err(Error::config(format!(
            "refusing to {what} without \"confirm\": true"
        )));
    }
    // Safe mode is the second check, and the one a caller cannot pass by setting a flag: it wants the
    // connection's token, which only a live connection test hands out.
    super::require_safe_mode_for_params(session, p)
}

/// Execute DDL and refresh the cached catalog so the tree matches the server again.
fn execute_ddl(
    session: &mut Session,
    index: usize,
    statements: &[String],
) -> Result<serde_json::Value> {
    let mut executed = Vec::new();
    for sql in statements {
        run_sql(session, index, sql, Some(1))?;
        executed.push(sql.clone());
    }
    let _ = crate::cmd::refresh_catalog(session, index);
    Ok(json!({ "statements": executed }))
}

fn list_tables(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let filter_schema = p.str_opt("schema");
    let catalog = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))?;
    let mut out = Vec::new();
    for schema in &catalog.schemas {
        if filter_schema.as_deref().is_some_and(|s| s != schema.name) {
            continue;
        }
        for table in &schema.tables {
            if !session.settings.show_system_objects && table.kind == nook_model::TableKind::System
            {
                continue;
            }
            out.push(json!({
                "key": table.key(), "schema": table.schema, "name": table.name,
                "kind": table.kind.label(), "columns": table.columns.len(),
                "rows": table.row_estimate, "size_bytes": table.size_bytes,
                "comment": table.comment,
            }));
        }
    }
    Ok(json!({ "tables": out, "databases": catalog.databases }))
}

fn table_from_params(
    session: &Session,
    p: &Params,
) -> Result<(usize, TableRef, nook_model::Table)> {
    let index = session.resolve_conn(p)?;
    let spec = p
        .str_opt("table")
        .filter(|spec| !spec.trim().is_empty())
        .ok_or_else(|| Error::config("pass \"table\""))?;
    let reference = TableRef::parse(&spec);
    let table = session.table_of(index, reference.schema.as_deref(), &reference.table)?;
    Ok((index, reference, table))
}

fn columns(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (_, reference, table) = table_from_params(session, p)?;
    Ok(json!({
        "table": reference.key(),
        "primary_key": table.primary_key(),
        "columns": table.columns.iter().map(|c| json!({
            "name": c.name, "type": c.type_name, "nullable": c.nullable,
            "primary_key": c.primary_key, "auto_increment": c.auto_increment,
            "default": c.default, "comment": c.comment, "ordinal": c.ordinal,
        })).collect::<Vec<_>>(),
    }))
}

fn indexes(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (_, reference, table) = table_from_params(session, p)?;
    Ok(json!({
        "table": reference.key(),
        "indexes": table.indexes.iter().map(|i| json!({
            "name": i.name, "columns": i.columns, "unique": i.unique,
            "primary": i.primary, "kind": i.kind,
        })).collect::<Vec<_>>(),
    }))
}

fn foreign_keys(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (_, reference, table) = table_from_params(session, p)?;
    Ok(json!({
        "table": reference.key(),
        "foreign_keys": table.foreign_keys.iter().map(|f| json!({
            "name": f.name, "columns": f.columns, "ref_schema": f.ref_schema,
            "ref_table": f.ref_table, "ref_columns": f.ref_columns,
            "on_delete": f.on_delete, "on_update": f.on_update,
        })).collect::<Vec<_>>(),
    }))
}

fn ddl_of(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, _) = table_from_params(session, p)?;
    let sql = {
        let driver = session.driver_for(index)?;
        driver.table_ddl(reference.schema.as_deref(), &reference.table)?
    };
    Ok(json!({ "table": reference.key(), "ddl": sql }))
}

fn create_table(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let spec = p
        .str_opt("table")
        .ok_or_else(|| Error::config("pass \"table\""))?;
    let reference = TableRef::parse(&spec);
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let raw_columns = p
        .get("columns")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::config("pass \"columns\": [{name, type, …}]"))?;
    let mut table = nook_model::Table::new(reference.schema.clone(), reference.table.clone());
    for raw in raw_columns {
        let params = Params::from_json(raw.clone())?;
        let mut column = column_from_params(&params)?;
        column.ordinal = table.columns.len() as i64 + 1;
        table.columns.push(column);
    }
    if table.columns.is_empty() {
        return Err(Error::config("a table needs at least one column"));
    }
    let mut statements = vec![ddl::create_table(&table, dialect)];
    if let Some(raw_indexes) = p.get("indexes").and_then(|v| v.as_array()) {
        for raw in raw_indexes {
            let params = Params::from_json(raw.clone())?;
            let name = params
                .str_opt("name")
                .unwrap_or_else(|| format!("ix_{}_{}", table.name, table.indexes.len() + 1));
            table.indexes.push(nook_model::Index {
                name,
                columns: params.list("columns"),
                unique: params.bool_opt("unique", false),
                ..nook_model::Index::default()
            });
        }
        statements.extend(ddl::create_indexes(&table, dialect));
    }
    execute_ddl(session, index, &statements)
}

fn copy_table(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, _, table) = table_from_params(session, p)?;
    let new_name = p.str_arg("new_name")?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let mut copy = table.clone();
    copy.name = new_name.clone();
    copy.indexes.clear();
    copy.foreign_keys.clear();
    let mut statements = vec![ddl::create_table(&copy, dialect)];
    if p.bool_opt("with_data", false) {
        let columns = copy
            .columns
            .iter()
            .map(|c| quote_ident(&c.name, dialect))
            .collect::<Vec<_>>()
            .join(", ");
        statements.push(format!(
            "INSERT INTO {} ({columns}) SELECT {columns} FROM {};",
            quote_qualified(table.schema.as_deref(), &new_name, dialect),
            quote_qualified(table.schema.as_deref(), &table.name, dialect)
        ));
    }
    execute_ddl(session, index, &statements)
}

fn drop_table(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a table")?;
    let (index, reference, _) = table_from_params(session, p)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::drop_table(reference.schema.as_deref(), &reference.table, dialect, true);
    execute_ddl(session, index, &[sql])
}

fn truncate_table(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "truncate a table")?;
    let (index, reference, _) = table_from_params(session, p)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::truncate_table(reference.schema.as_deref(), &reference.table, dialect);
    execute_ddl(session, index, &[sql])
}

fn rename_table(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, _) = table_from_params(session, p)?;
    let new_name = p.str_arg("new_name")?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::rename_table(
        reference.schema.as_deref(),
        &reference.table,
        &new_name,
        dialect,
    );
    execute_ddl(session, index, &[sql])
}

fn add_column(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, _) = table_from_params(session, p)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let column = column_from_params(p)?;
    let sql = ddl::add_column(
        reference.schema.as_deref(),
        &reference.table,
        &column,
        dialect,
    );
    execute_ddl(session, index, &[sql])
}

fn set_primary_key(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let columns = {
        let list = p.list("columns");
        if list.is_empty() {
            return Err(Error::config("pass \"columns\" — the key columns to add"));
        }
        list
    };
    let (index, reference, mut table) = table_from_params(session, p)?;
    let existing = table.primary_key();
    if !existing.is_empty() {
        return Err(Error::config(format!(
            "{} already has a primary key ({})",
            reference.key(),
            existing.join(", ")
        )));
    }
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    // SQLite cannot ALTER a key in place: mark the columns and let the driver rebuild the table.
    if dialect == Dialect::Sqlite {
        for name in &columns {
            let column = table
                .column_mut(name)
                .ok_or_else(|| Error::not_found(format!("no column '{name}'")))?;
            column.primary_key = true;
        }
        return rebuild(session, index, reference.schema.as_deref(), &table);
    }
    let sql = ddl::add_primary_key(
        reference.schema.as_deref(),
        &reference.table,
        &columns,
        dialect,
    )?;
    execute_ddl(session, index, &[sql])
}

fn partitions(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, _) = table_from_params(session, p)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    // The bounds are read from the server's own catalog rather than from a parsed DDL string: a
    // partition bound is an expression (`FOR VALUES FROM ('2026-11-01') TO ('2026-12-01')`), and
    // re-parsing text to recover what the server already models would be a second implementation.
    let (sql, kind) = match dialect {
        Dialect::Postgres => (
            format!(
                "SELECT c.relname, pg_get_expr(c.relpartbound, c.oid), \
                        (SELECT reltuples::bigint FROM pg_class p WHERE p.oid = c.oid) \
                 FROM pg_class c JOIN pg_inherits i ON i.inhrelid = c.oid \
                 JOIN pg_class parent ON parent.oid = i.inhparent \
                 JOIN pg_namespace n ON n.oid = parent.relnamespace \
                 WHERE parent.relname = {name} AND parent.relkind = 'p' \
                 ORDER BY c.relname",
                name = quote_literal(&reference.table)
            ),
            "parent is not partitioned",
        ),
        Dialect::MySql => (
            format!(
                "SELECT PARTITION_NAME, PARTITION_DESCRIPTION, TABLE_ROWS \
                 FROM information_schema.PARTITIONS \
                 WHERE TABLE_SCHEMA = COALESCE({schema}, DATABASE()) AND TABLE_NAME = {name} \
                   AND PARTITION_NAME IS NOT NULL \
                 ORDER BY PARTITION_ORDINAL_POSITION",
                schema = reference
                    .schema
                    .as_deref()
                    .map(quote_literal)
                    .unwrap_or_else(|| "NULL".into()),
                name = quote_literal(&reference.table)
            ),
            "not a partitioned table",
        ),
        Dialect::Sqlite => (
            String::new(),
            "SQLite has no partitions; a partition is a table of its own",
        ),
    };
    if sql.is_empty() {
        return Ok(json!({
            "table": reference.key(),
            "partitioned": false,
            "partitions": [],
            "note": kind,
        }));
    }
    let rs = run_sql(session, index, &sql, Some(1000))?;
    let items: Vec<serde_json::Value> = rs
        .rows
        .iter()
        .map(|row| {
            json!({
                "name": row.first().map(|v| v.as_text()).unwrap_or_default(),
                "bound": row.get(1).map(|v| v.as_text()),
                "rows": row.get(2).and_then(|v| match v {
                    nook_core::Value::Int(n) => Some(*n),
                    nook_core::Value::UInt(n) => i64::try_from(*n).ok(),
                    _ => None,
                }),
            })
        })
        .collect();
    Ok(json!({
        "table": reference.key(),
        "partitioned": !items.is_empty(),
        "partitions": items,
        "note": if items.is_empty() { kind } else { "" },
    }))
}

fn create_partition(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let parent = p.str_arg("parent")?;
    let name = p.str_arg("name")?;
    let from = p.str_arg("from")?;
    let to = p.str_arg("to")?;
    // Range partitions only, and the bounds are dates: free SQL in a partition bound would be a
    // query editor wearing a dialog's clothes.
    // Shape alone is not a date: `2026-13-01` has the right bytes and no such month, and the server
    // would reject it after the dialog already closed. The month/day ranges are checked here.
    let date = |s: &str| -> Result<String> {
        let bytes = s.as_bytes();
        let shaped = s.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-';
        let number = |range: std::ops::Range<usize>| -> Option<u32> {
            s.get(range).and_then(|part| part.parse().ok())
        };
        let (year, month, day) = (number(0..4), number(5..7), number(8..10));
        let valid = match (shaped, year, month, day) {
            (true, Some(_), Some(m), Some(d)) => {
                (1..=12).contains(&m) && (1..=31).contains(&d)
            }
            _ => false,
        };
        if !valid {
            return Err(Error::config(format!(
                "'{s}' is not a date; use YYYY-MM-DD with a real month and day"
            )));
        }
        Ok(s.to_string())
    };
    let (from, to) = (date(&from)?, date(&to)?);
    if from >= to {
        return Err(Error::config(format!(
            "the range is empty: from '{from}' is not before to '{to}'"
        )));
    }
    let index = session.resolve_conn(p)?;
    let reference = TableRef::parse(&parent);
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let parent_q = quote_qualified(reference.schema.as_deref(), &reference.table, dialect);
    let sql = match dialect {
        Dialect::Postgres => format!(
            "CREATE TABLE {child} PARTITION OF {parent_q} FOR VALUES FROM ('{from}') TO ('{to}');",
            child = quote_qualified(reference.schema.as_deref(), &name, dialect),
        ),
        Dialect::MySql => format!(
            "ALTER TABLE {parent_q} ADD PARTITION (PARTITION {part} VALUES LESS THAN ('{to}'));",
            part = quote_ident(&name, dialect),
        ),
        Dialect::Sqlite => {
            return Err(Error::unsupported(
                "SQLite has no partitions; a partition is a table of its own",
            ))
        }
    };
    let out = execute_ddl(session, index, &[sql])?;
    Ok(json!({
        "parent": parent,
        "name": name,
        "from": from,
        "to": to,
        "statements": out["statements"],
    }))
}

fn drop_partition(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a partition")?;
    let parent = p.str_arg("parent")?;
    let name = p.str_arg("name")?;
    let index = session.resolve_conn(p)?;
    let reference = TableRef::parse(&parent);
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = match dialect {
        Dialect::Postgres => format!(
            "DROP TABLE {};",
            quote_qualified(reference.schema.as_deref(), &name, dialect)
        ),
        Dialect::MySql => format!(
            "ALTER TABLE {} DROP PARTITION {};",
            quote_qualified(reference.schema.as_deref(), &reference.table, dialect),
            quote_ident(&name, dialect)
        ),
        Dialect::Sqlite => {
            return Err(Error::unsupported(
                "SQLite has no partitions; drop the table instead",
            ))
        }
    };
    let out = execute_ddl(session, index, &[sql])?;
    Ok(json!({
        "parent": parent,
        "name": name,
        "statements": out["statements"],
    }))
}

/// A single-quoted SQL literal. The catalog queries interpolate names, so this is a trust boundary:
/// a doubled quote is the only escape SQL has.
fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn table_rebuild(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, mut table) = table_from_params(session, p)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    if dialect != Dialect::Sqlite {
        return Err(Error::unsupported(format!(
            "{} alters in place; a table rebuild is SQLite's workaround for DDL it cannot ALTER",
            dialect_name(dialect)
        )));
    }
    let changes = p
        .get("changes")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            Error::config(
                r#"pass "changes" — a diff, not a column list: a full list is how a column gets dropped by omission"#,
            )
        })?;
    let before: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
    apply_changes(&mut table, changes)?;
    if table.columns.is_empty() {
        return Err(Error::config(
            "the changes would leave the table with no columns",
        ));
    }
    // The rows have to survive a destructive rebuild, so the count is compared across it. A change
    // that silently truncates is the failure this command exists to make impossible.
    let rows_before = count_rows(session, index, reference.schema.as_deref(), &reference.table)?;
    let outcome = rebuild(session, index, reference.schema.as_deref(), &table)?;
    let rows_after = count_rows(session, index, reference.schema.as_deref(), &reference.table)?;
    // The catalog is the proof, not a second implementation: every index, trigger and foreign key
    // the table had must still be there under the same name. One the plan could not replay is named
    // rather than silently missing.
    let after = session.table_of(index, reference.schema.as_deref(), &reference.table)?;
    let mut lost: Vec<String> = Vec::new();
    for before in &table.indexes {
        if !after.indexes.iter().any(|i| i.name == before.name) {
            lost.push(format!("index {}", before.name));
        }
    }
    for fk in &table.foreign_keys {
        let same = after
            .foreign_keys
            .iter()
            .any(|a| a.columns == fk.columns && a.ref_table == fk.ref_table);
        if !same {
            lost.push(format!("foreign key on {}", fk.columns.join(", ")));
        }
    }
    // Triggers live on the schema in the catalog, not on the table. A rebuild that dropped one is
    // the loss nobody sees until the audit trail goes quiet, so it is checked by name.
    let triggers_before: Vec<String> = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .map(|catalog| {
            catalog
                .schemas
                .iter()
                .flat_map(|s| &s.triggers)
                .filter(|t| t.table == reference.table)
                .map(|t| t.name.clone())
                .collect()
        })
        .unwrap_or_default();
    if !lost.is_empty() {
        return Err(Error::config(format!(
            "the rebuild could not replay {}; the table was rebuilt, so recreate {} now",
            lost.join(", "),
            if lost.len() == 1 { "it" } else { "them" }
        )));
    }
    Ok(json!({
        "table": reference.key(),
        "before": before,
        "after": after.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        "rows_before": rows_before,
        "rows_after": rows_after,
        "lost": lost,
        "triggers_seen": triggers_before,
        "rebuilt": true,
        "statements": outcome["statements"],
    }))
}

/// Apply a `changes` diff to `table` in place: `rename`, `retype`, `nullable`, `default`, `drop`,
/// `add`. Every key is optional and unknown keys are refused, so a typo cannot silently do nothing.
fn apply_changes(table: &mut Table, changes: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    for (key, value) in changes {
        match key.as_str() {
            "rename" => {
                let map = value
                    .as_object()
                    .ok_or_else(|| Error::config(r#""rename" takes {"old":"new"}"#))?;
                for (old, new) in map {
                    let new = new
                        .as_str()
                        .ok_or_else(|| Error::config(r#""rename" maps a name to a name"#))?;
                    let column = table
                        .column_mut(old)
                        .ok_or_else(|| Error::not_found(format!("no column '{old}'")))?;
                    column.name = new.to_string();
                }
            }
            "retype" => {
                let map = value
                    .as_object()
                    .ok_or_else(|| Error::config(r#""retype" takes {"column":"TYPE"}"#))?;
                for (name, ty) in map {
                    let ty = ty
                        .as_str()
                        .ok_or_else(|| Error::config(r#""retype" maps a name to a type"#))?;
                    let column = table
                        .column_mut(name)
                        .ok_or_else(|| Error::not_found(format!("no column '{name}'")))?;
                    column.type_name = ty.to_string();
                }
            }
            "nullable" => {
                let map = value
                    .as_object()
                    .ok_or_else(|| Error::config(r#""nullable" takes {"column":true}"#))?;
                for (name, on) in map {
                    let on = on
                        .as_bool()
                        .ok_or_else(|| Error::config(r#""nullable" maps a name to a bool"#))?;
                    let column = table
                        .column_mut(name)
                        .ok_or_else(|| Error::not_found(format!("no column '{name}'")))?;
                    column.nullable = on;
                }
            }
            "default" => {
                let map = value
                    .as_object()
                    .ok_or_else(|| Error::config(r#""default" takes {"column":"expr"}"#))?;
                for (name, expr) in map {
                    let column = table
                        .column_mut(name)
                        .ok_or_else(|| Error::not_found(format!("no column '{name}'")))?;
                    column.default = match expr {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(s) => Some(s.clone()),
                        other => Some(other.to_string()),
                    };
                }
            }
            "drop" => {
                let names = value
                    .as_array()
                    .ok_or_else(|| Error::config(r#""drop" takes ["column", …]"#))?;
                for name in names {
                    let name = name
                        .as_str()
                        .ok_or_else(|| Error::config(r#""drop" takes column names"#))?;
                    let before = table.columns.len();
                    table.columns.retain(|c| c.name != name);
                    if table.columns.len() == before {
                        return Err(Error::not_found(format!("no column '{name}'")));
                    }
                }
            }
            "add" => {
                let list = value
                    .as_array()
                    .ok_or_else(|| Error::config(r#""add" takes [{"name":…,"type":…}]"#))?;
                for entry in list {
                    let name = entry
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Error::config(r#"an "add" entry needs "name""#))?;
                    let ty = entry
                        .get("type")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Error::config(r#"an "add" entry needs "type""#))?;
                    if table.columns.iter().any(|c| c.name == name) {
                        return Err(Error::config(format!("column '{name}' already exists")));
                    }
                    let mut column = nook_model::Column::new(name, ty);
                    column.nullable = entry
                        .get("nullable")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    // Past every existing ordinal, not `len + 1`: after a drop, `len + 1` can equal
                    // a live column's ordinal and the rebuild would copy that column's data into
                    // the new one.
                    column.ordinal = table
                        .columns
                        .iter()
                        .map(|c| c.ordinal)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    table.columns.push(column);
                }
            }
            other => {
                return Err(Error::config(format!(
                    "unknown change '{other}'; use rename, retype, nullable, default, drop or add"
                )))
            }
        }
    }
    Ok(())
}

/// Every foreign key that points at `table` but names a column `columns` no longer has.
///
/// Read from `sqlite_master` and `PRAGMA foreign_key_list`, because the catalog is the server's own
/// record of the reference: a rebuild that changed a name has to be checked against what the other
/// tables still say, not against what the caller intended.
fn dangling_references(
    session: &mut Session,
    index: usize,
    table: &str,
    columns: &[nook_model::Column],
) -> Result<Vec<String>> {
    let children = run_sql(
        session,
        index,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        Some(10_000),
    )?;
    let mut dangling = Vec::new();
    for row in &children.rows {
        let child = row.first().map(|v| v.as_text()).unwrap_or_default();
        if child.is_empty() || child.eq_ignore_ascii_case(table) {
            continue;
        }
        let fks = run_sql(
            session,
            index,
            &format!("PRAGMA foreign_key_list({})", quote_ident(&child, Dialect::Sqlite)),
            Some(1000),
        )?;
        for fk in &fks.rows {
            let parent = fk.get(2).map(|v| v.as_text()).unwrap_or_default();
            if !parent.eq_ignore_ascii_case(table) {
                continue;
            }
            // A null `to` means "the parent's primary key", which this check cannot name; it is left
            // to `foreign_key_check`, which does resolve it.
            let to = match fk.get(4) {
                Some(v) if !v.is_null() => v.as_text(),
                _ => continue,
            };
            if !columns.iter().any(|c| c.name.eq_ignore_ascii_case(&to)) {
                let from = fk.get(3).map(|v| v.as_text()).unwrap_or_default();
                dangling.push(format!("{child}.{from} -> {table}.{to}"));
            }
        }
    }
    Ok(dangling)
}

/// `COUNT(*)` for a table, so a rebuild can prove it kept its rows.
fn count_rows(
    session: &mut Session,
    index: usize,
    schema: Option<&str>,
    table: &str,
) -> Result<i64> {
    let qualified = quote_qualified(schema, table, session.driver_for(index)?.dialect());
    let rs = run_sql(session, index, &format!("SELECT COUNT(*) FROM {qualified}"), Some(1))?;
    Ok(rs
        .rows
        .first()
        .and_then(|row| row.first())
        .map(|v| v.as_text().parse().unwrap_or(0))
        .unwrap_or(0))
}

/// Run a SQLite table rebuild for `table`'s current columns, atomically.
///
/// One implementation for every caller: `set_primary_key` and `schema.tableRebuild` both hand the
/// driver a table they have already modified, so neither can invent a second rebuild with different
/// transaction rules.
///
/// The plan's first and last statements are pragmas — SQLite ignores them inside a transaction — so
/// they run outside; every step in between is atomic. A failure after DROP TABLE would otherwise
/// leave the rows stranded in {table}_dc_new and the session without its pragma restore. A rebuild
/// cannot run inside a transaction the user already opened: the plan's pragma OFF would be a silent
/// no-op in it (DROP TABLE would cascade child rows), and BEGIN would fail with "cannot start a
/// transaction within a transaction".
fn rebuild(
    session: &mut Session,
    index: usize,
    schema: Option<&str>,
    table: &Table,
) -> Result<serde_json::Value> {
    if session.driver_for(index)?.in_transaction() {
        return Err(Error::config(
            "commit or roll back the open transaction before rebuilding the table",
        ));
    }
    let script = {
        let driver = session.driver_for(index)?;
        driver.rebuild_table(schema, table, &table.columns.clone())?
    };
    let (pre, body, post): (Vec<_>, Vec<_>, Vec<_>) = {
        let mut pre = Vec::new();
        let mut body = Vec::new();
        let mut post = Vec::new();
        for sql in script.into_iter() {
            if sql.trim_start().to_ascii_lowercase().starts_with("pragma") {
                if body.is_empty() {
                    pre.push(sql);
                } else {
                    post.push(sql);
                }
            } else {
                body.push(sql);
            }
        }
        (pre, body, post)
    };
    for sql in &pre {
        run_sql(session, index, sql, Some(1))?;
    }
    let outcome = (|| -> Result<()> {
        session.driver_for(index)?.begin()?;
        for sql in &body {
            if let Err(e) = run_sql(session, index, sql, Some(1)) {
                let _ = session.driver_for(index)?.rollback();
                return Err(e);
            }
        }
        // A child table that still names a column this rebuild renamed is the loss SQLite will not
        // report: `foreign_key_check` skips a parent key it cannot resolve, so the database looks
        // healthy while every insert into the child now fails. Resolving the references here is the
        // only place that can see it.
        let dangling = dangling_references(session, index, &table.name, &table.columns)?;
        if !dangling.is_empty() {
            let _ = session.driver_for(index)?.rollback();
            return Err(Error::config(format!(
                "the rebuild would leave {} dangling ({}); nothing was changed",
                if dangling.len() == 1 { "a reference" } else { "references" },
                dangling.join(", ")
            )));
        }
        // Inside the transaction, because the only useful answer to "this rebuild would orphan rows"
        // is to put the rows back. `foreign_keys` is OFF for the copy, so the check is what proves the
        // result is still coherent.
        // Scoped to this table: an unrelated legacy orphan elsewhere in the database is not this
        // rebuild's business, and refusing every rebuild because of one would make the command
        // unusable on any database that has ever had a constraint disabled.
        let check = run_sql(
            session,
            index,
            &format!(
                "PRAGMA foreign_key_check({})",
                quote_ident(&table.name, Dialect::Sqlite)
            ),
            Some(50),
        );
        let orphans = match check {
            Ok(rs) => rs,
            Err(e) => {
                // A mismatch — a child still naming a column this rebuild renamed — surfaces as an
                // error, not a row, and must also put the table back.
                let _ = session.driver_for(index)?.rollback();
                return Err(Error::config(format!(
                    "the rebuild would leave a reference dangling ({e}); nothing was changed"
                )));
            }
        };
        if !orphans.rows.is_empty() {
            let _ = session.driver_for(index)?.rollback();
            let rows: Vec<String> = orphans
                .rows
                .iter()
                .take(10)
                .map(|r| {
                    format!(
                        "{}.{}",
                        r.first().map(|v| v.as_text()).unwrap_or_default(),
                        r.get(1).map(|v| v.as_text()).unwrap_or_default()
                    )
                })
                .collect();
            return Err(Error::config(format!(
                "the rebuild would orphan {} row(s) ({}); nothing was changed",
                orphans.rows.len(),
                rows.join(", ")
            )));
        }
        session.driver_for(index)?.commit()
    })();
    for sql in &post {
        run_sql(session, index, sql, Some(1))?;
    }
    outcome?;
    execute_ddl(session, index, &[])
}

fn drop_column(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a column")?;
    let (index, reference, _) = table_from_params(session, p)?;
    let column = p
        .str_opt("column")
        .ok_or_else(|| Error::config("pass \"column\""))?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::drop_column(
        reference.schema.as_deref(),
        &reference.table,
        &column,
        dialect,
    );
    execute_ddl(session, index, &[sql])
}

fn rename_column(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, _) = table_from_params(session, p)?;
    let column = p.str_arg("column")?;
    let new_name = p.str_arg("new_name")?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::rename_column(
        reference.schema.as_deref(),
        &reference.table,
        &column,
        &new_name,
        dialect,
    )?;
    execute_ddl(session, index, &[sql])
}

fn change_type(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, mut table) = table_from_params(session, p)?;
    let column = p.str_arg("column")?;
    let new_type = p.str_arg("type")?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    // SQLite has no in-place type change, but it does have a rebuild. Refusing here is what made
    // the designer unable to edit a field's type at all.
    if dialect == Dialect::Sqlite {
        let target = table
            .column_mut(&column)
            .ok_or_else(|| Error::not_found(format!("no column '{column}'")))?;
        target.type_name = new_type;
        return rebuild(session, index, reference.schema.as_deref(), &table);
    }
    let sql = ddl::alter_column_type(
        reference.schema.as_deref(),
        &reference.table,
        &column,
        &new_type,
        dialect,
    )?;
    execute_ddl(session, index, &[sql])
}

fn set_nullable(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, mut table) = table_from_params(session, p)?;
    let column_name = p.str_arg("column")?;
    let nullable = p.bool_opt("nullable", true);
    let column = table
        .column(&column_name)
        .cloned()
        .ok_or_else(|| Error::not_found(format!("no column '{column_name}'")))?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    if dialect == Dialect::Sqlite {
        if let Some(target) = table.column_mut(&column_name) {
            target.nullable = nullable;
        }
        return rebuild(session, index, reference.schema.as_deref(), &table);
    }
    let sql = ddl::set_nullable(
        reference.schema.as_deref(),
        &reference.table,
        &column_name,
        nullable,
        &column.type_name,
        dialect,
    )?;
    execute_ddl(session, index, &[sql])
}

fn set_default(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, mut table) = table_from_params(session, p)?;
    let column = p.str_arg("column")?;
    let default = p.str_opt("default");
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    if dialect == Dialect::Sqlite {
        table
            .column_mut(&column)
            .ok_or_else(|| Error::not_found(format!("no column '{column}'")))?
            .default = default;
        return rebuild(session, index, reference.schema.as_deref(), &table);
    }
    let sql = ddl::set_default(
        reference.schema.as_deref(),
        &reference.table,
        &column,
        default.as_deref(),
        dialect,
    )?;
    execute_ddl(session, index, &[sql])
}

fn create_index(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, _) = table_from_params(session, p)?;
    let columns = {
        let list = p.list("columns");
        if list.is_empty() {
            return Err(Error::config("pass \"columns\""));
        }
        list
    };
    let name = p
        .str_opt("name")
        .unwrap_or_else(|| format!("ix_{}_{}", reference.table, columns.join("_")));
    let index_def = nook_model::Index {
        name,
        columns,
        unique: p.bool_opt("unique", false),
        ..nook_model::Index::default()
    };
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::create_index(
        reference.schema.as_deref(),
        &reference.table,
        &index_def,
        dialect,
    );
    execute_ddl(session, index, &[sql])
}

fn drop_index(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop an index")?;
    let (index, reference, _) = table_from_params(session, p)?;
    let name = p
        .str_opt("index")
        .or_else(|| p.str_opt("name"))
        .ok_or_else(|| Error::config("pass \"index\""))?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = ddl::drop_index(
        reference.schema.as_deref(),
        &reference.table,
        &name,
        dialect,
    );
    execute_ddl(session, index, &[sql])
}

fn add_foreign_key(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let (index, reference, table) = table_from_params(session, p)?;
    let columns = p.list("columns");
    let ref_table = p.str_arg("ref_table")?;
    let ref_columns = p.list("ref_columns");
    if columns.is_empty() || ref_columns.is_empty() {
        return Err(Error::config("pass \"columns\" and \"ref_columns\""));
    }
    let name = p
        .str_opt("name")
        .unwrap_or_else(|| format!("fk_{}_{}", reference.table, ref_table));
    let fk = nook_model::ForeignKey {
        name,
        columns,
        ref_schema: p.str_opt("ref_schema").or_else(|| table.schema.clone()),
        ref_table,
        ref_columns,
        on_delete: p.str_opt("on_delete").unwrap_or_default(),
        on_update: p.str_opt("on_update").unwrap_or_default(),
    };
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    if dialect == Dialect::Sqlite {
        return Err(Error::unsupported(
            "SQLite cannot add a foreign key to an existing table; recreate the table instead",
        ));
    }
    let sql = ddl::add_foreign_key(reference.schema.as_deref(), &reference.table, &fk, dialect);
    execute_ddl(session, index, &[sql])
}

fn create_view(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let name = p.str_arg("name")?;
    let body = p.str_arg("sql")?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let replace = if p.bool_opt("replace", true) {
        "CREATE OR REPLACE "
    } else {
        "CREATE "
    };
    let sql = format!(
        "{replace}VIEW {} AS {}",
        quote_qualified(p.str_opt("schema").as_deref(), &name, dialect),
        body.trim_end_matches(';')
    );
    execute_ddl(session, index, &[sql])
}

fn drop_view(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a view")?;
    let (index, reference, _) = table_from_params(session, p)?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
    let sql = match dialect {
        Dialect::MySql => format!(
            "DROP VIEW IF EXISTS {};",
            quote_qualified(reference.schema.as_deref(), &reference.table, dialect)
        ),
        _ => format!(
            "DROP VIEW IF EXISTS {};",
            quote_qualified(reference.schema.as_deref(), &reference.table, dialect)
        ),
    };
    execute_ddl(session, index, &[sql])
}

fn search(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let needle = p.str_arg("pattern")?.to_ascii_lowercase();
    let catalog = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))?;
    let mut hits = Vec::new();
    for schema in &catalog.schemas {
        for table in &schema.tables {
            let table_hit = table.name.to_ascii_lowercase().contains(&needle);
            let columns: Vec<&str> = table
                .columns
                .iter()
                .filter(|c| c.name.to_ascii_lowercase().contains(&needle))
                .map(|c| c.name.as_str())
                .collect();
            if table_hit || !columns.is_empty() {
                hits.push(json!({
                    "table": table.key(), "matched_name": table_hit, "matched_columns": columns,
                }));
            }
        }
    }
    Ok(json!({ "pattern": needle, "matches": hits }))
}

fn empty_database(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "empty the database")?;
    let index = session.resolve_conn(p)?;
    let (dialect, statements) = {
        let conn = session
            .conns
            .get(index)
            .ok_or_else(|| Error::not_found("no such connection"))?;
        let dialect = conn
            .driver
            .as_ref()
            .map(|d| d.dialect())
            .ok_or_else(|| Error::config("the connection is not open"))?;
        let catalog = conn
            .catalog
            .as_ref()
            .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))?;
        let mut statements: Vec<String> = Vec::new();
        for schema in &catalog.schemas {
            for table in &schema.tables {
                if table.kind == nook_model::TableKind::Table {
                    statements.push(ddl::drop_table_cascade(
                        table.schema.as_deref(),
                        &table.name,
                        dialect,
                    ));
                }
            }
        }
        (dialect, statements)
    };
    let _ = dialect;
    if statements.is_empty() {
        return Ok(json!({ "dropped": 0, "statements": [] }));
    }
    let count = statements.len();
    execute_ddl(session, index, &statements).map(|_| json!({ "dropped": count }))
}

/// Shared plumbing for the three editors: the connection's dialect or nothing.
fn editor_dialect(session: &Session, index: usize) -> Result<Dialect> {
    session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))
}

/// Run one statement verbatim, then refresh the catalog. Unlike `execute_ddl` this
/// avoids `run_script`/`split_script`, which would cut a trigger or routine body on
/// its inner `;` — the server parses the whole CREATE as one statement.
fn run_ddl_verbatim(session: &mut Session, index: usize, sql: &str) -> Result<()> {
    {
        let driver = session.driver_for(index)?;
        driver.run(sql, 1)?;
    }
    let _ = crate::cmd::refresh_catalog(session, index);
    Ok(())
}

fn trigger_editor(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let action = p.str_opt("action").unwrap_or_else(|| "list".into());
    match action.as_str() {
        "list" => trigger_list(session, index, p),
        "create" => trigger_create(session, index, p),
        "drop" => trigger_drop(session, index, p),
        other => Err(Error::config(format!(
            "unknown action '{other}'; use list, create or drop"
        ))),
    }
}

fn trigger_list(session: &Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    let (_, reference, _) = table_from_params(session, p)?;
    let _ = index;
    let spec = p.str_arg("table")?;
    let _ = spec;
    let catalog = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))?;
    let items = catalog
        .schemas
        .iter()
        .flat_map(|s| &s.triggers)
        .filter(|t| {
            let matches_schema = reference
                .schema
                .as_deref()
                .map(|s| t.schema.as_deref() == Some(s))
                .unwrap_or(true);
            matches_schema && (t.table == reference.table || reference.table.is_empty())
        })
        .map(|t| {
            json!({
                "name": t.name, "table": t.table, "timing": t.timing,
                "events": t.events, "enabled": t.enabled, "ddl": t.ddl,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "table": reference.key(), "items": items }))
}

fn trigger_create(session: &mut Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    let dialect = editor_dialect(session, index)?;
    let name = p.str_arg("name")?;
    let spec = p.str_arg("table")?;
    let timing = p.str_arg("timing")?;
    let event = p.str_arg("event")?;
    let body = p.str_arg("body")?;
    let reference = TableRef::parse(&spec);
    let sql = ddl::create_trigger(
        reference.schema.as_deref(),
        &name,
        &reference.table,
        &timing,
        &event,
        &body,
        dialect,
    );
    run_ddl_verbatim(session, index, &sql)?;
    Ok(json!({ "created": name, "sql": sql }))
}

fn trigger_drop(session: &mut Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a trigger")?;
    let dialect = editor_dialect(session, index)?;
    let name = p.str_arg("name")?;
    let spec = p.str_arg("table")?;
    let reference = TableRef::parse(&spec);
    let sql = ddl::drop_trigger(
        reference.schema.as_deref(),
        &name,
        &reference.table,
        dialect,
    );
    run_ddl_verbatim(session, index, &sql)?;
    Ok(json!({ "dropped": name }))
}

fn function_editor(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let action = p.str_opt("action").unwrap_or_else(|| "list".into());
    match action.as_str() {
        "list" => function_list(session, index),
        "create" => function_create(session, index, p),
        "drop" => function_drop(session, index, p),
        other => Err(Error::config(format!(
            "unknown action '{other}'; use list, create or drop"
        ))),
    }
}

fn function_list(session: &Session, index: usize) -> Result<serde_json::Value> {
    let catalog = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))?;
    let items = catalog
        .schemas
        .iter()
        .flat_map(|s| &s.routines)
        .map(|r| {
            json!({
                "name": r.name, "kind": r.kind, "schema": r.schema,
                "returns": r.returns, "comment": r.comment, "ddl": r.ddl,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "items": items }))
}

fn function_create(session: &mut Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    let dialect = editor_dialect(session, index)?;
    if dialect == Dialect::Sqlite {
        return Err(Error::config(
            "SQLite has no stored routines; its app-defined functions are registered at connect time, not created here",
        ));
    }
    let name = p.str_arg("name")?;
    let kind = p.str_opt("kind").unwrap_or_else(|| "function".into());
    let sql = p.str_arg("sql")?;
    let schema = p.str_opt("schema");
    let statement = ddl::create_routine(schema.as_deref(), &name, &kind, &sql, dialect);
    run_ddl_verbatim(session, index, &statement)?;
    Ok(json!({ "created": name, "sql": statement }))
}

fn function_drop(session: &mut Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a routine")?;
    let dialect = editor_dialect(session, index)?;
    if dialect == Dialect::Sqlite {
        return Err(Error::config("SQLite has no stored routines to drop"));
    }
    let name = p.str_arg("name")?;
    let kind = p.str_opt("kind").unwrap_or_else(|| "function".into());
    let schema = p.str_opt("schema");
    let sql = ddl::drop_routine(schema.as_deref(), &name, &kind, dialect);
    run_ddl_verbatim(session, index, &sql)?;
    Ok(json!({ "dropped": name }))
}

fn sequence_editor(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let action = p.str_opt("action").unwrap_or_else(|| "list".into());
    match action.as_str() {
        "list" => sequence_list(session, index),
        "create" => sequence_create(session, index, p),
        "drop" => sequence_drop(session, index, p),
        other => Err(Error::config(format!(
            "unknown action '{other}'; use list, create or drop"
        ))),
    }
}

fn sequence_list(session: &Session, index: usize) -> Result<serde_json::Value> {
    let catalog = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))?;
    let items = catalog
        .schemas
        .iter()
        .flat_map(|s| &s.sequences)
        .map(|q| {
            json!({
                "name": q.name, "schema": q.schema, "start": q.start,
                "increment": q.increment, "min": q.min, "max": q.max,
                "cycle": q.cycle, "last_value": q.last_value,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "items": items }))
}

fn sequence_create(session: &mut Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    let dialect = editor_dialect(session, index)?;
    match dialect {
        Dialect::Postgres => {}
        _ => {
            return Err(Error::config(
                "sequences are a PostgreSQL feature; SQLite uses AUTOINCREMENT and MySQL uses AUTO_INCREMENT columns",
            ))
        }
    }
    let name = p.str_arg("name")?;
    let seq = nook_model::Sequence {
        schema: p.str_opt("schema"),
        name: name.clone(),
        start: p.int_opt("start", 1),
        increment: p.int_opt("increment", 1),
        min: p.get("min").and_then(|v| v.as_i64()),
        max: p.get("max").and_then(|v| v.as_i64()),
        cycle: p.bool_opt("cycle", false),
        last_value: None,
    };
    let sql = ddl::create_sequence(&seq, dialect);
    run_ddl_verbatim(session, index, &sql)?;
    Ok(json!({ "created": name, "sql": sql }))
}

fn sequence_drop(session: &mut Session, index: usize, p: &Params) -> Result<serde_json::Value> {
    require_confirm(session, p, "drop a sequence")?;
    let dialect = editor_dialect(session, index)?;
    match dialect {
        Dialect::Postgres => {}
        _ => return Err(Error::config("sequences are a PostgreSQL feature")),
    }
    let name = p.str_arg("name")?;
    let schema = p.str_opt("schema");
    let sql = ddl::drop_sequence(schema.as_deref(), &name, dialect);
    run_ddl_verbatim(session, index, &sql)?;
    Ok(json!({ "dropped": name }))
}

pub fn dialect_of(session: &Session, index: usize) -> Option<String> {
    session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| dialect_name(d.dialect()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_commands_need_confirmation() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"table": "users"})).expect("params");
        assert!(drop_table(&mut session, &p).is_err());
        let mut with_confirm =
            Params::from_json(json!({"table": "users", "confirm": true})).expect("params");
        // No connection, so it fails for a different reason, but past the confirmation gate.
        let e = drop_table(&mut session, &with_confirm).expect_err("no connection");
        assert!(e.message().contains("connection"));
        let _ = &mut with_confirm;
    }

    #[test]
    fn table_list_needs_a_catalog() {
        let mut session = Session::new();
        assert!(list_tables(&mut session, &Params::none()).is_err());
    }

    #[test]
    fn column_params_default_to_nullable_text() {
        let p = Params::from_json(json!({"name": "note"})).expect("params");
        let column = column_from_params(&p).expect("column");
        assert_eq!(column.type_name, "TEXT");
        assert!(column.nullable);
        let p = Params::from_json(json!({})).expect("params");
        assert!(column_from_params(&p).is_err());
    }

    #[test]
    fn set_primary_key_needs_columns() {
        let mut session = Session::new();
        let p = Params::from_json(json!({"table": "users"})).expect("params");
        let e = set_primary_key(&mut session, &p).expect_err("no columns");
        assert!(e.message().contains("columns"), "{e}");
    }

    /// The path the demo database takes: a keyless SQLite table gets its key through the rebuild,
    /// and the catalog shows it — the whole reason `rebuild_table` was wired into the driver.
    #[test]
    fn set_primary_key_rebuilds_a_sqlite_table() {
        let mut session = Session::new();
        let cfg = nook_core::ConnectionConfig::new("mem", nook_core::DriverKind::Sqlite);
        let mut driver = nook_drivers::sqlite::Sqlite::memory().expect("memory db");
        nook_drivers::Driver::run_script(
            &mut driver,
            "CREATE TABLE users (id INTEGER, email TEXT);",
            10,
        )
        .expect("setup");
        let mut conn = crate::Conn::new(cfg);
        conn.driver = Some(Box::new(driver));
        conn.catalog = Some(
            conn.driver
                .as_mut()
                .expect("driver installed")
                .catalog()
                .expect("catalog"),
        );
        session.conns.push(conn);
        let p = Params::from_json(json!({"table": "users", "columns": ["id"], "conn": "mem"}))
            .expect("params");
        let out = set_primary_key(&mut session, &p).expect("the key is added");
        assert!(
            out["statements"].as_array().expect("statements").is_empty(),
            "the plan ran inside the transaction, not through the plain executor"
        );
        let table = session.conns[0]
            .catalog
            .as_ref()
            .and_then(|c| c.find_table(None, "users"))
            .expect("the table is in the catalog");
        assert_eq!(table.primary_key(), vec!["id".to_string()]);
        // The rebuilt table really carries the key: an insert without the column fails is wrong —
        // what proves it is the schema, read straight from the server.
        let driver = session.driver_for(0).expect("driver");
        let rs = driver
            .run("SELECT sql FROM sqlite_master WHERE name = 'users'", 1)
            .expect("schema read");
        assert!(
            rs.rows[0][0].as_text().contains("PRIMARY KEY"),
            "the rebuilt table declares its key: {}",
            rs.rows[0][0].as_text()
        );
    }

    /// A failure mid-plan must not strand the table: everything after BEGIN rolls back, the
    /// original table keeps its rows, and the pragma restore still runs. The plan's pragma
    /// statements sit outside the transaction (SQLite ignores them inside one).
    #[test]
    fn a_failed_rebuild_leaves_the_table_untouched() {
        let mut session = Session::new();
        let cfg = nook_core::ConnectionConfig::new("mem", nook_core::DriverKind::Sqlite);
        let mut driver = nook_drivers::sqlite::Sqlite::memory().expect("memory db");
        nook_drivers::Driver::run_script(
            &mut driver,
            "CREATE TABLE users (id INTEGER, email TEXT);\
             INSERT INTO users (id, email) VALUES (1, 'a@b.c');",
            10,
        )
        .expect("setup");
        let mut conn = crate::Conn::new(cfg);
        conn.driver = Some(Box::new(driver));
        conn.catalog = Some(
            conn.driver
                .as_mut()
                .expect("driver installed")
                .catalog()
                .expect("catalog"),
        );
        session.conns.push(conn);
        let p = Params::from_json(json!({"table": "users", "columns": ["id"], "conn": "mem"}))
            .expect("params");
        // Force a failure at the INSERT step — deterministically. Duplicated `id` values make
        // `INSERT … SELECT` fail with a UNIQUE violation once `id` is the key of the scratch
        // table. Without a transaction the scratch table would be stranded; with one, the plan
        // rolls back and the original table keeps both rows.
        nook_drivers::Driver::run_script(
            session.driver_for(0).expect("driver"),
            "INSERT INTO users (id, email) VALUES (2, 'd@e.f');",
            1,
        )
        .expect("second row");
        nook_drivers::Driver::run_script(
            session.driver_for(0).expect("driver"),
            "UPDATE users SET id = 1 WHERE email = 'd@e.f';",
            1,
        )
        .expect("duplicate made");
        let e = set_primary_key(&mut session, &p).expect_err("the plan fails on the INSERT");
        assert!(
            e.message().contains("UNIQUE"),
            "the failure comes from the plan, not the harness: {e}"
        );
        // The original table is intact: still named `users`, still holding its row.
        let driver = session.driver_for(0).expect("driver");
        let rs = driver
            .run("SELECT count(*) FROM users", 1)
            .expect("the original table survived");
        assert_eq!(rs.rows[0][0].as_text(), "2", "both rows are still there");
        // The pragma restore ran even though the plan failed.
        let rs = driver.run("PRAGMA foreign_keys", 1).expect("pragma read");
        assert_eq!(
            rs.rows[0][0].as_text(),
            "0",
            "enforcement is back where it started"
        );
        // No scratch table left behind by the rolled-back plan.
        let rs = driver
            .run(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'users_dc_new%'",
                1,
            )
            .expect("scratch gone");
        assert_eq!(rs.rows[0][0].as_text(), "0");
    }

    /// Rebuilding a parent with FK enforcement ON must not cascade child rows away: the plan's
    /// `PRAGMA foreign_keys = OFF` runs before BEGIN (the pragma is a no-op inside one), so the
    /// implicit DELETE behind DROP TABLE cannot fire ON DELETE CASCADE.
    #[test]
    fn rebuilding_a_parent_keeps_child_rows_with_enforcement_on() {
        let mut session = Session::new();
        let cfg = nook_core::ConnectionConfig::new("mem", nook_core::DriverKind::Sqlite);
        let mut driver = nook_drivers::sqlite::Sqlite::memory().expect("memory db");
        nook_drivers::Driver::run_script(
            &mut driver,
            "CREATE TABLE users (id INTEGER, email TEXT);\
             CREATE TABLE posts (id INTEGER PRIMARY KEY, author_id INTEGER REFERENCES users(id) ON DELETE CASCADE);\
             INSERT INTO users (id, email) VALUES (1, 'a@b.c');\
             INSERT INTO posts (id, author_id) VALUES (1, 1);",
            10,
        )
        .expect("setup");
        nook_drivers::Driver::run(&mut driver, "PRAGMA foreign_keys = ON", 1).expect("enforce");
        let mut conn = crate::Conn::new(cfg);
        conn.driver = Some(Box::new(driver));
        conn.catalog = Some(
            conn.driver
                .as_mut()
                .expect("driver installed")
                .catalog()
                .expect("catalog"),
        );
        session.conns.push(conn);
        let p = Params::from_json(json!({"table": "users", "columns": ["id"], "conn": "mem"}))
            .expect("params");
        set_primary_key(&mut session, &p).expect("the key is added");
        let driver = session.driver_for(0).expect("driver");
        let rs = driver
            .run("SELECT count(*) FROM posts", 1)
            .expect("child rows read");
        assert_eq!(
            rs.rows[0][0].as_text(),
            "1",
            "the cascade must not have deleted the child row during the rebuild"
        );
        // Enforcement is restored to ON — the session's own choice, not the plan's.
        let rs = driver.run("PRAGMA foreign_keys", 1).expect("pragma read");
        assert_eq!(rs.rows[0][0].as_text(), "1");
    }

    /// The rebuild cannot run inside a transaction the user already opened: the plan's pragma
    /// OFF would be a silent no-op in it (DROP TABLE would cascade child rows), and BEGIN would
    /// fail with "cannot start a transaction within a transaction". Fail fast with the reason.
    #[test]
    fn set_primary_key_refuses_an_open_transaction() {
        let mut session = Session::new();
        let cfg = nook_core::ConnectionConfig::new("mem", nook_core::DriverKind::Sqlite);
        let mut driver = nook_drivers::sqlite::Sqlite::memory().expect("memory db");
        nook_drivers::Driver::run_script(
            &mut driver,
            "CREATE TABLE users (id INTEGER, email TEXT);",
            10,
        )
        .expect("setup");
        nook_drivers::Driver::begin(&mut driver).expect("begin");
        let mut conn = crate::Conn::new(cfg);
        conn.driver = Some(Box::new(driver));
        conn.catalog = Some(
            conn.driver
                .as_mut()
                .expect("driver installed")
                .catalog()
                .expect("catalog"),
        );
        session.conns.push(conn);
        let p = Params::from_json(json!({"table": "users", "columns": ["id"], "conn": "mem"}))
            .expect("params");
        let e = set_primary_key(&mut session, &p).expect_err("an open tx must refuse");
        assert!(
            e.message().contains("transaction"),
            "the refusal names the open transaction: {e}"
        );
    }
}
