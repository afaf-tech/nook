//! Grid edit buffer → DML. Pure: the UI records edits, this turns them into statements.

use datacraft_core::{quote_ident, quote_qualified, sql_literal, Dialect, Error, Result, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// A single cell changed in an existing row.
    SetCell,
    /// A new row added in the grid, not yet on the server.
    InsertRow,
    /// An existing row marked for deletion.
    DeleteRow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowEdit {
    pub kind: EditKind,
    /// Row index in the grid the edit came from, so the UI can map failures back.
    pub row_index: usize,
    pub schema: Option<String>,
    pub table: String,
    /// Primary key columns and their values. Required for update and delete.
    pub pk: Vec<(String, Value)>,
    /// Columns to write. For `InsertRow` this is the full column list.
    pub changes: Vec<(String, Value)>,
}

/// One `UPDATE` for an existing row, keyed by primary key.
pub fn plan_update(
    schema: Option<&str>,
    table: &str,
    pk: &[(String, Value)],
    changes: &[(String, Value)],
    dialect: Dialect,
) -> Result<String> {
    if pk.is_empty() {
        return Err(Error::unsupported(format!(
            "{table} has no primary key, so DataCraft will not guess which row to change"
        )));
    }
    if changes.is_empty() {
        return Err(Error::internal("nothing to update"));
    }
    let sets = changes
        .iter()
        .map(|(c, v)| format!("{} = {}", quote_ident(c, dialect), sql_literal(v, dialect)))
        .collect::<Vec<_>>()
        .join(", ");
    let where_clause = where_from_pk(pk, dialect);
    Ok(format!(
        "UPDATE {} SET {sets} WHERE {where_clause};",
        quote_qualified(schema, table, dialect)
    ))
}

fn where_from_pk(pk: &[(String, Value)], dialect: Dialect) -> String {
    pk.iter()
        .map(|(c, v)| {
            if v.is_null() {
                format!("{} IS NULL", quote_ident(c, dialect))
            } else {
                format!("{} = {}", quote_ident(c, dialect), sql_literal(v, dialect))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn plan_insert(
    schema: Option<&str>,
    table: &str,
    changes: &[(String, Value)],
    dialect: Dialect,
) -> Result<String> {
    if changes.is_empty() {
        return Err(Error::internal("insert with no columns"));
    }
    let cols = changes
        .iter()
        .map(|(c, _)| quote_ident(c, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let vals = changes
        .iter()
        .map(|(_, v)| sql_literal(v, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "INSERT INTO {} ({cols}) VALUES ({vals});",
        quote_qualified(schema, table, dialect)
    ))
}

fn plan_delete(
    schema: Option<&str>,
    table: &str,
    pk: &[(String, Value)],
    dialect: Dialect,
) -> Result<String> {
    if pk.is_empty() {
        return Err(Error::unsupported(format!(
            "{table} has no primary key, so DataCraft will not guess which row to delete"
        )));
    }
    Ok(format!(
        "DELETE FROM {} WHERE {};",
        quote_qualified(schema, table, dialect),
        where_from_pk(pk, dialect)
    ))
}

/// Statements for a batch of grid edits, in the order the user made them.
pub fn plan(edits: &[RowEdit], dialect: Dialect) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(edits.len());
    for e in edits {
        let sql = match e.kind {
            EditKind::SetCell => {
                plan_update(e.schema.as_deref(), &e.table, &e.pk, &e.changes, dialect)?
            }
            EditKind::InsertRow => plan_insert(e.schema.as_deref(), &e.table, &e.changes, dialect)?,
            EditKind::DeleteRow => plan_delete(e.schema.as_deref(), &e.table, &e.pk, dialect)?,
        };
        out.push(sql);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_uses_primary_key() {
        let sql = plan_update(
            Some("public"),
            "users",
            &[("id".into(), Value::Int(7))],
            &[("email".into(), Value::text("a@b.c"))],
            Dialect::Postgres,
        )
        .expect("planned");
        assert_eq!(
            sql,
            "UPDATE \"public\".\"users\" SET \"email\" = 'a@b.c' WHERE \"id\" = 7;"
        );
    }

    #[test]
    fn update_refuses_without_primary_key() {
        let e = plan_update(
            None,
            "t",
            &[],
            &[("a".into(), Value::Int(1))],
            Dialect::MySql,
        );
        assert!(e.is_err());
    }

    #[test]
    fn null_keys_use_is_null() {
        let sql = plan_update(
            None,
            "t",
            &[("k".into(), Value::Null)],
            &[("v".into(), Value::Int(1))],
            Dialect::Sqlite,
        )
        .expect("planned");
        assert!(sql.contains("WHERE \"k\" IS NULL"));
    }

    #[test]
    fn batch_keeps_order_and_kinds() {
        let edits = vec![
            RowEdit {
                kind: EditKind::InsertRow,
                row_index: 0,
                schema: None,
                table: "t".into(),
                pk: vec![],
                changes: vec![("a".into(), Value::Int(1))],
            },
            RowEdit {
                kind: EditKind::DeleteRow,
                row_index: 3,
                schema: None,
                table: "t".into(),
                pk: vec![("id".into(), Value::Int(9))],
                changes: vec![],
            },
        ];
        let sql = plan(&edits, Dialect::Sqlite).expect("planned");
        assert_eq!(sql.len(), 2);
        assert!(sql[0].starts_with("INSERT INTO \"t\""));
        assert!(sql[1].starts_with("DELETE FROM \"t\" WHERE \"id\" = 9"));
    }

    #[test]
    fn injection_through_a_cell_is_escaped() {
        let sql = plan_update(
            None,
            "t",
            &[("id".into(), Value::Int(1))],
            &[("v".into(), Value::text("x'; DROP TABLE t; --"))],
            Dialect::Postgres,
        )
        .expect("planned");
        assert!(sql.contains("'x''; DROP TABLE t; --'"));
        assert_eq!(sql.matches("DROP TABLE").count(), 1);
    }
}
