//! Schema commands: the table designer, object DDL, indexes and foreign keys.

use nook_core::{quote_ident, quote_qualified, Dialect, Error, Result};
use nook_model::ddl;
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
        CommandSpec::new("schema.triggerEditor", "Trigger editor", "Design", trigger_editor)
            .params(r#"{"action":"create","table":"users","name":"audit_email","timing":"AFTER","event":"INSERT","body":"UPDATE users SET email = lower(email);"}"#)
            .help("list/create/drop via \"action\"; create builds the CREATE TRIGGER per dialect."),
        CommandSpec::new("schema.functionEditor", "Function editor", "Design", function_editor)
            .params(r#"{"action":"create","name":"add_one","kind":"function","sql":"(n int) RETURNS int AS $$ SELECT n + 1 $$ LANGUAGE sql"}"#)
            .help("list/create/drop stored routines via \"action\"; the sql body is passed through verbatim; SQLite refuses."),
        CommandSpec::new("schema.sequenceEditor", "Sequence editor", "Design", sequence_editor)
            .params(r#"{"action":"create","name":"user_ids","start":1,"increment":1}"#)
            .help("list/create/drop sequences via \"action\"; PostgreSQL only."),
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
    Ok(())
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
    let (index, reference, _) = table_from_params(session, p)?;
    let column = p.str_arg("column")?;
    let new_type = p.str_arg("type")?;
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
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
    let (index, reference, table) = table_from_params(session, p)?;
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
    let (index, reference, _) = table_from_params(session, p)?;
    let column = p.str_arg("column")?;
    let default = p.str_opt("default");
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .ok_or_else(|| Error::config("the connection is not open"))?;
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
}
