//! DDL for all three dialects: the table designer, the structure diff and the dump all call these.

use nook_core::{quote_ident, quote_qualified, Dialect, Error, Result};

use crate::{Column, ForeignKey, Index, Table};

/// Column definition as it appears inside `CREATE TABLE`.
/// `inline_pk` is true when this is the table's single primary key column; composite keys become a table constraint instead.
pub fn column_ddl(col: &Column, dialect: Dialect, inline_pk: bool) -> String {
    let mut ty = col.type_name.trim().to_string();
    if ty.is_empty() {
        ty = "TEXT".into();
    }
    if col.auto_increment {
        match dialect {
            Dialect::Sqlite => {
                // SQLite only auto-assigns when the declared type is exactly INTEGER.
                if !ty.eq_ignore_ascii_case("integer") {
                    ty = "INTEGER".into();
                }
            }
            Dialect::Postgres => {
                let upper = ty.to_ascii_uppercase();
                if !upper.contains("SERIAL") && !upper.contains("IDENTITY") {
                    ty = if upper.contains("BIG") {
                        "bigserial".into()
                    } else if upper.contains("SMALL") {
                        "smallserial".into()
                    } else {
                        "serial".into()
                    };
                }
            }
            Dialect::MySql => {}
        }
    }
    let mut out = quote_ident(&col.name, dialect);
    out.push(' ');
    out.push_str(&ty);
    if inline_pk {
        out.push_str(" PRIMARY KEY");
        if col.auto_increment {
            match dialect {
                Dialect::Sqlite => out.push_str(" AUTOINCREMENT"),
                Dialect::MySql => out.push_str(" AUTO_INCREMENT"),
                Dialect::Postgres => {}
            }
        }
    } else if col.auto_increment && dialect == Dialect::MySql {
        out.push_str(" AUTO_INCREMENT");
    }
    if !col.nullable && !(inline_pk && dialect == Dialect::Sqlite) {
        out.push_str(" NOT NULL");
    }
    if let Some(d) = &col.default {
        if !d.is_empty() {
            out.push_str(" DEFAULT ");
            out.push_str(d);
        }
    }
    out
}

/// `CREATE TABLE`, primary key inlined. Indexes and foreign keys come from the sibling helpers —
/// except when `inline_foreign_keys` is set: SQLite cannot `ALTER TABLE … ADD FOREIGN KEY`, so a
/// rebuild has to carry the constraints inside the CREATE it runs.
pub fn create_table(table: &Table, dialect: Dialect) -> String {
    create_table_with(table, dialect, false)
}

pub fn create_table_with(table: &Table, dialect: Dialect, inline_foreign_keys: bool) -> String {
    let target = quote_qualified(table.schema.as_deref(), &table.name, dialect);
    let pk = table.primary_key();
    let inline = pk.len() == 1;
    let mut defs: Vec<String> = table
        .columns
        .iter()
        .map(|c| column_ddl(c, dialect, inline && c.primary_key))
        .collect();
    if pk.len() > 1 {
        defs.push(format!(
            "PRIMARY KEY ({})",
            pk.iter()
                .map(|c| quote_ident(c, dialect))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if inline_foreign_keys {
        for fk in &table.foreign_keys {
            let cols = fk
                .columns
                .iter()
                .map(|c| quote_ident(c, dialect))
                .collect::<Vec<_>>()
                .join(", ");
            let ref_target =
                quote_qualified(fk.ref_schema.as_deref().or(table.schema.as_deref()), &fk.ref_table, dialect);
            let ref_cols = fk
                .ref_columns
                .iter()
                .map(|c| quote_ident(c, dialect))
                .collect::<Vec<_>>()
                .join(", ");
            let mut constraint = format!(
                "FOREIGN KEY ({cols}) REFERENCES {ref_target} ({ref_cols})",
            );
            if !fk.on_delete.is_empty() && !fk.on_delete.eq_ignore_ascii_case("no action") {
                constraint.push_str(&format!(" ON DELETE {}", fk.on_delete.to_ascii_uppercase()));
            }
            if !fk.on_update.is_empty() && !fk.on_update.eq_ignore_ascii_case("no action") {
                constraint.push_str(&format!(" ON UPDATE {}", fk.on_update.to_ascii_uppercase()));
            }
            defs.push(constraint);
        }
    }
    format!("CREATE TABLE {target} (\n  {}\n);", defs.join(",\n  "))
}

pub fn create_indexes(table: &Table, dialect: Dialect) -> Vec<String> {
    table
        .indexes
        .iter()
        .filter(|i| !i.primary)
        .map(|i| create_index(table.schema.as_deref(), &table.name, i, dialect))
        .collect()
}

pub fn create_index(schema: Option<&str>, table: &str, index: &Index, dialect: Dialect) -> String {
    let cols = index
        .columns
        .iter()
        .map(|c| quote_ident(c, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let unique = if index.unique { "UNIQUE " } else { "" };
    let target = quote_qualified(schema, table, dialect);
    format!(
        "CREATE {unique}INDEX {} ON {target} ({cols});",
        quote_ident(&index.name, dialect)
    )
}

pub fn create_foreign_keys(table: &Table, dialect: Dialect) -> Vec<String> {
    table
        .foreign_keys
        .iter()
        .map(|fk| add_foreign_key(table.schema.as_deref(), &table.name, fk, dialect))
        .collect()
}

pub fn add_foreign_key(
    schema: Option<&str>,
    table: &str,
    fk: &ForeignKey,
    dialect: Dialect,
) -> String {
    let target = quote_qualified(schema, table, dialect);
    let cols = fk
        .columns
        .iter()
        .map(|c| quote_ident(c, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let ref_target = quote_qualified(fk.ref_schema.as_deref().or(schema), &fk.ref_table, dialect);
    let ref_cols = fk
        .ref_columns
        .iter()
        .map(|c| quote_ident(c, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "ALTER TABLE {target} ADD CONSTRAINT {} FOREIGN KEY ({cols}) REFERENCES {ref_target} ({ref_cols})",
        quote_ident(&fk.name, dialect)
    );
    if !fk.on_delete.is_empty() && !fk.on_delete.eq_ignore_ascii_case("no action") {
        sql.push_str(&format!(" ON DELETE {}", fk.on_delete.to_ascii_uppercase()));
    }
    if !fk.on_update.is_empty() && !fk.on_update.eq_ignore_ascii_case("no action") {
        sql.push_str(&format!(" ON UPDATE {}", fk.on_update.to_ascii_uppercase()));
    }
    sql.push(';');
    sql
}

pub fn drop_table(schema: Option<&str>, table: &str, dialect: Dialect, if_exists: bool) -> String {
    let target = quote_qualified(schema, table, dialect);
    let guard = if if_exists { "IF EXISTS " } else { "" };
    match dialect {
        Dialect::MySql => format!("DROP TABLE {guard}{target};"),
        _ => format!("DROP TABLE {guard}{target};"),
    }
}

pub fn drop_table_cascade(schema: Option<&str>, table: &str, dialect: Dialect) -> String {
    let target = quote_qualified(schema, table, dialect);
    match dialect {
        Dialect::Postgres => format!("DROP TABLE IF EXISTS {target} CASCADE;"),
        Dialect::MySql => format!("DROP TABLE IF EXISTS {target};"),
        Dialect::Sqlite => format!("DROP TABLE IF EXISTS {target};"),
    }
}

pub fn truncate_table(schema: Option<&str>, table: &str, dialect: Dialect) -> String {
    let target = quote_qualified(schema, table, dialect);
    match dialect {
        Dialect::Sqlite => format!("DELETE FROM {target};"),
        Dialect::MySql => format!("TRUNCATE TABLE {target};"),
        Dialect::Postgres => format!("TRUNCATE TABLE {target} RESTART IDENTITY CASCADE;"),
    }
}

pub fn rename_table(schema: Option<&str>, from: &str, to: &str, dialect: Dialect) -> String {
    // Postgres and SQLite take a bare target name; MySQL allows a qualified one.
    let (target, dest) = match dialect {
        Dialect::MySql => (
            quote_qualified(schema, from, dialect),
            quote_qualified(schema, to, dialect),
        ),
        _ => (
            quote_qualified(None, from, dialect),
            quote_qualified(None, to, dialect),
        ),
    };
    format!("ALTER TABLE {target} RENAME TO {dest};")
}

pub fn add_column(schema: Option<&str>, table: &str, col: &Column, dialect: Dialect) -> String {
    format!(
        "ALTER TABLE {} ADD COLUMN {};",
        quote_qualified(schema, table, dialect),
        column_ddl(col, dialect, false)
    )
}

pub fn drop_column(schema: Option<&str>, table: &str, column: &str, dialect: Dialect) -> String {
    format!(
        "ALTER TABLE {} DROP COLUMN {};",
        quote_qualified(schema, table, dialect),
        quote_ident(column, dialect)
    )
}

pub fn rename_column(
    schema: Option<&str>,
    table: &str,
    from: &str,
    to: &str,
    dialect: Dialect,
) -> Result<String> {
    let target = quote_qualified(schema, table, dialect);
    Ok(match dialect {
        Dialect::MySql => format!(
            "ALTER TABLE {target} RENAME COLUMN {} TO {};",
            quote_ident(from, dialect),
            quote_ident(to, dialect)
        ),
        Dialect::Postgres => format!(
            "ALTER TABLE {target} RENAME COLUMN {} TO {};",
            quote_ident(from, dialect),
            quote_ident(to, dialect)
        ),
        // SQLite has supported RENAME COLUMN since 3.25.
        Dialect::Sqlite => format!(
            "ALTER TABLE {target} RENAME COLUMN {} TO {};",
            quote_ident(from, dialect),
            quote_ident(to, dialect)
        ),
    })
}

pub fn alter_column_type(
    schema: Option<&str>,
    table: &str,
    column: &str,
    new_type: &str,
    dialect: Dialect,
) -> Result<String> {
    let target = quote_qualified(schema, table, dialect);
    match dialect {
        Dialect::Postgres => Ok(format!(
            "ALTER TABLE {target} ALTER COLUMN {} TYPE {new_type} USING {}::{new_type};",
            quote_ident(column, dialect),
            quote_ident(column, dialect)
        )),
        Dialect::MySql => Ok(format!(
            "ALTER TABLE {target} MODIFY COLUMN {} {new_type};",
            quote_ident(column, dialect)
        )),
        Dialect::Sqlite => Err(Error::unsupported(
            "SQLite cannot change a column type in place; rebuild the table (Create new table, copy, drop, rename)",
        )),
    }
}

pub fn set_nullable(
    schema: Option<&str>,
    table: &str,
    column: &str,
    nullable: bool,
    type_name: &str,
    dialect: Dialect,
) -> Result<String> {
    let target = quote_qualified(schema, table, dialect);
    let clause = if nullable {
        "DROP NOT NULL"
    } else {
        "SET NOT NULL"
    };
    match dialect {
        Dialect::Postgres => Ok(format!(
            "ALTER TABLE {target} ALTER COLUMN {} {clause};",
            quote_ident(column, dialect)
        )),
        Dialect::MySql => Ok(format!(
            "ALTER TABLE {target} MODIFY COLUMN {} {type_name}{};",
            quote_ident(column, dialect),
            if nullable { " NULL" } else { " NOT NULL" }
        )),
        Dialect::Sqlite => Err(Error::unsupported(
            "SQLite cannot change NOT NULL in place; rebuild the table",
        )),
    }
}

pub fn set_default(
    schema: Option<&str>,
    table: &str,
    column: &str,
    default: Option<&str>,
    dialect: Dialect,
) -> Result<String> {
    let target = quote_qualified(schema, table, dialect);
    let value = default.unwrap_or("");
    Ok(match dialect {
        Dialect::Postgres => format!(
            "ALTER TABLE {target} ALTER COLUMN {} {};",
            quote_ident(column, dialect),
            if value.is_empty() {
                "DROP DEFAULT".to_string()
            } else {
                format!("SET DEFAULT {value}")
            }
        ),
        Dialect::MySql | Dialect::Sqlite => {
            return Err(Error::unsupported(format!(
                "{} cannot change a default in place{}",
                dialect_label(dialect),
                if dialect == Dialect::Sqlite {
                    "; rebuild the table"
                } else {
                    ""
                }
            )))
        }
    })
}

/// Give a keyless table its primary key. SQLite cannot ALTER a key in place — it needs the full
/// table rebuild, which the designer does not drive yet — so it refuses with the reason.
pub fn add_primary_key(
    schema: Option<&str>,
    table: &str,
    columns: &[String],
    dialect: Dialect,
) -> Result<String> {
    match dialect {
        Dialect::Sqlite => Err(Error::unsupported(
            "SQLite cannot add a primary key in place; rebuild the table with the columns marked as keys",
        )),
        _ => {
            let cols = columns
                .iter()
                .map(|c| quote_ident(c, dialect))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "ALTER TABLE {} ADD PRIMARY KEY ({});",
                quote_qualified(schema, table, dialect),
                cols
            ))
        }
    }
}

pub fn drop_index(schema: Option<&str>, table: &str, index: &str, dialect: Dialect) -> String {
    match dialect {
        Dialect::MySql => format!(
            "DROP INDEX {} ON {};",
            quote_ident(index, dialect),
            quote_qualified(schema, table, dialect)
        ),
        _ => format!("DROP INDEX IF EXISTS {};", quote_ident(index, dialect)),
    }
}

pub fn dialect_label(d: Dialect) -> &'static str {
    match d {
        Dialect::Sqlite => "SQLite",
        Dialect::Postgres => "PostgreSQL",
        Dialect::MySql => "MySQL",
    }
}

/// `CREATE TRIGGER` for the three dialects. The body is the server-side language as-is
/// (`BEGIN … END` / `EXECUTE FUNCTION …`); we wire the header, the caller owns the body.
pub fn create_trigger(
    schema: Option<&str>,
    name: &str,
    table: &str,
    timing: &str,
    events: &str,
    body: &str,
    dialect: Dialect,
) -> String {
    let quoted = quote_ident(name, dialect);
    let target = quote_qualified(schema, table, dialect);
    match dialect {
        // SQLite has no schemas in the PostgreSQL sense: the table alone is right.
        Dialect::Sqlite => format!(
            "CREATE TRIGGER {quoted} {timing} {events} ON {target}\nBEGIN\n{body}\nEND;"
        ),
        Dialect::Postgres => format!(
            "CREATE TRIGGER {quoted} {timing} {events} ON {target} FOR EACH ROW EXECUTE FUNCTION {body};"
        ),
        Dialect::MySql => format!(
            "CREATE TRIGGER {quoted} {timing} {events} ON {target} FOR EACH ROW\nBEGIN\n{body}\nEND;"
        ),
    }
}

/// PG requires an unqualified trigger name plus the trigger's table in
/// `DROP TRIGGER … ON table`; SQLite and MySQL reject `ON`, so it is only
/// interpolated for Postgres.
pub fn drop_trigger(schema: Option<&str>, name: &str, table: &str, dialect: Dialect) -> String {
    let target = quote_ident(name, dialect);
    match dialect {
        // SQLite has no IF EXISTS for triggers.
        Dialect::Sqlite => format!("DROP TRIGGER {target};"),
        Dialect::Postgres => {
            format!("DROP TRIGGER IF EXISTS {target} ON {};", quote_qualified(schema, table, dialect))
        }
        Dialect::MySql => format!("DROP TRIGGER IF EXISTS {target};"),
    }
}

/// `CREATE FUNCTION`/`CREATE PROCEDURE`: the body is passed through verbatim — the dialects'
/// function languages are too different to generate, so we only enforce the create-or-replace
/// header per dialect.
pub fn create_routine(
    schema: Option<&str>,
    name: &str,
    kind: &str,
    body: &str,
    dialect: Dialect,
) -> String {
    let target = quote_qualified(schema, name, dialect);
    match (dialect, kind) {
        (Dialect::Postgres, "function") => {
            format!("CREATE OR REPLACE FUNCTION {target} {body}")
        }
        (Dialect::Postgres, _) => format!("CREATE OR REPLACE PROCEDURE {target} {body}"),
        (Dialect::MySql, "function") => format!("CREATE FUNCTION {target} {body}"),
        (Dialect::MySql, _) => format!("CREATE PROCEDURE {target} {body}"),
        // SQLite has no stored functions: routes go through `create_function` at connection
        // time, not DDL, so this arm only exists to keep the match total.
        (Dialect::Sqlite, _) => format!("CREATE TRIGGER {target} {body}"),
    }
}

pub fn drop_routine(schema: Option<&str>, name: &str, kind: &str, dialect: Dialect) -> String {
    let target = quote_qualified(schema, name, dialect);
    let kw = if kind == "function" { "FUNCTION" } else { "PROCEDURE" };
    match dialect {
        Dialect::Sqlite => format!("DROP {kw} {target};"),
        _ => format!("DROP {kw} IF EXISTS {target};"),
    }
}

pub fn create_sequence(seq: &crate::Sequence, dialect: Dialect) -> String {
    let target = quote_qualified(seq.schema.as_deref(), &seq.name, dialect);
    let mut s = format!("CREATE SEQUENCE {target}");
    if seq.start != 0 {
        s.push_str(&format!(" START WITH {}", seq.start));
    }
    if seq.increment != 1 {
        s.push_str(&format!(" INCREMENT BY {}", seq.increment));
    }
    if let Some(min) = seq.min {
        s.push_str(&format!(" MINVALUE {min}"));
    }
    if let Some(max) = seq.max {
        s.push_str(&format!(" MAXVALUE {max}"));
    }
    if seq.cycle {
        s.push_str(" CYCLE");
    }
    s.push(';');
    s
}

pub fn drop_sequence(schema: Option<&str>, name: &str, dialect: Dialect) -> String {
    format!(
        "DROP SEQUENCE IF EXISTS {};",
        quote_qualified(schema, name, dialect)
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    fn users() -> Table {
        let mut t = Table::new(Some("public".into()), "users");
        let mut id = Column::new("id", "int8");
        id.primary_key = true;
        id.nullable = false;
        t.columns.push(id);
        let mut email = Column::new("email", "text");
        email.nullable = false;
        t.columns.push(email);
        t.columns.push(Column::new("note", "text"));
        t
    }

    #[test]
    fn create_table_per_dialect() {
        let t = users();
        let pg = create_table(&t, Dialect::Postgres);
        assert!(pg.starts_with("CREATE TABLE \"public\".\"users\" ("));
        assert!(pg.contains("\"id\" int8 PRIMARY KEY"));
        let my = create_table(&t, Dialect::MySql);
        assert!(my.starts_with("CREATE TABLE `public`.`users` ("));
    }

    #[test]
    fn autoincrement_maps_to_serial_and_autoincrement() {
        let mut t = Table::new(None, "t");
        let mut id = Column::new("id", "bigint");
        id.primary_key = true;
        id.auto_increment = true;
        t.columns.push(id);
        assert!(create_table(&t, Dialect::Postgres).contains("\"id\" bigserial"));
        assert!(create_table(&t, Dialect::MySql).contains("AUTO_INCREMENT"));
        assert!(create_table(&t, Dialect::Sqlite).contains("INTEGER PRIMARY KEY AUTOINCREMENT"));
    }

    #[test]
    fn composite_primary_key_is_a_table_constraint() {
        let mut t = Table::new(None, "m");
        let mut a = Column::new("a", "int");
        a.primary_key = true;
        let mut b = Column::new("b", "int");
        b.primary_key = true;
        t.columns.push(a);
        t.columns.push(b);
        assert!(create_table(&t, Dialect::Postgres).contains("PRIMARY KEY (\"a\", \"b\")"));
    }

    #[test]
    fn add_primary_key_per_dialect() {
        assert_eq!(
            add_primary_key(None, "users", &["id".into()], Dialect::Postgres).expect("pg"),
            "ALTER TABLE \"users\" ADD PRIMARY KEY (\"id\");"
        );
        assert_eq!(
            add_primary_key(Some("public"), "users", &["id".into()], Dialect::MySql)
                .expect("mysql"),
            "ALTER TABLE `public`.`users` ADD PRIMARY KEY (`id`);"
        );
        assert_eq!(
            add_primary_key(None, "m", &["a".into(), "b".into()], Dialect::Postgres).expect("pg"),
            "ALTER TABLE \"m\" ADD PRIMARY KEY (\"a\", \"b\");"
        );
        let err = add_primary_key(None, "t", &["id".into()], Dialect::Sqlite)
            .expect_err("sqlite cannot alter a key in place");
        assert!(err.to_string().contains("rebuild"));
    }

    #[test]
    fn truncate_and_indexes_follow_the_dialect() {
        assert_eq!(
            truncate_table(None, "t", Dialect::Sqlite),
            "DELETE FROM \"t\";"
        );
        assert_eq!(
            truncate_table(Some("s"), "t", Dialect::MySql),
            "TRUNCATE TABLE `s`.`t`;"
        );
        let idx = Index {
            name: "ix_email".into(),
            columns: vec!["email".into()],
            unique: true,
            ..Index::default()
        };
        assert_eq!(
            create_index(Some("public"), "users", &idx, Dialect::Postgres),
            "CREATE UNIQUE INDEX \"ix_email\" ON \"public\".\"users\" (\"email\");"
        );
        assert_eq!(
            drop_index(Some("s"), "t", "ix", Dialect::MySql),
            "DROP INDEX `ix` ON `s`.`t`;"
        );
    }

    #[test]
    fn unsupported_in_place_changes_say_so() {
        assert!(alter_column_type(None, "t", "c", "bigint", Dialect::Sqlite).is_err());
        assert!(set_nullable(None, "t", "c", false, "int", Dialect::Sqlite).is_err());
        assert!(alter_column_type(None, "t", "c", "bigint", Dialect::Postgres).is_ok());
    }

    #[test]
    fn foreign_key_with_actions() {
        let fk = ForeignKey {
            name: "fk_a".into(),
            columns: vec!["author_id".into()],
            ref_schema: Some("public".into()),
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            on_delete: "cascade".into(),
            on_update: String::new(),
        };
        let sql = add_foreign_key(Some("public"), "posts", &fk, Dialect::Postgres);
        assert!(sql.contains("FOREIGN KEY (\"author_id\") REFERENCES \"public\".\"users\" (\"id\") ON DELETE CASCADE"));
    }

    #[test]
    fn trigger_routine_sequence_ddl_per_dialect() {
        let sql = create_trigger(
            Some("public"),
            "touch",
            "users",
            "BEFORE",
            "UPDATE",
            "UPDATE users SET touched = 1;",
            Dialect::Sqlite,
        );
        assert_eq!(
            sql,
            "CREATE TRIGGER \"touch\" BEFORE UPDATE ON \"users\"\nBEGIN\nUPDATE users SET touched = 1;\nEND;"
        );
        let sql = create_trigger(
            Some("public"),
            "touch",
            "users",
            "AFTER",
            "INSERT",
            "notify_inserted()",
            Dialect::Postgres,
        );
        assert_eq!(
            sql,
            "CREATE TRIGGER \"touch\" AFTER INSERT ON \"public\".\"users\" FOR EACH ROW EXECUTE FUNCTION notify_inserted();"
        );
        assert_eq!(
            drop_trigger(None, "touch", "users", Dialect::Sqlite),
            "DROP TRIGGER \"touch\";"
        );
        assert_eq!(
            drop_trigger(Some("public"), "touch", "users", Dialect::Postgres),
            "DROP TRIGGER IF EXISTS \"touch\" ON \"public\".\"users\";"
        );

        let sql = create_routine(
            Some("public"),
            "add",
            "function",
            "(a int, b int) RETURNS int AS $$ SELECT a + b $$ LANGUAGE SQL;",
            Dialect::Postgres,
        );
        assert!(sql.starts_with("CREATE OR REPLACE FUNCTION \"public\".\"add\""));
        assert!(sql.ends_with("LANGUAGE SQL;"));
        assert_eq!(
            drop_routine(Some("public"), "add", "function", Dialect::Postgres),
            "DROP FUNCTION IF EXISTS \"public\".\"add\";"
        );
        assert_eq!(
            drop_routine(None, "add", "procedure", Dialect::MySql),
            "DROP PROCEDURE IF EXISTS `add`;"
        );

        let seq = crate::Sequence {
            schema: Some("public".into()),
            name: "order_seq".into(),
            start: 10,
            increment: 5,
            min: Some(10),
            max: Some(1000),
            cycle: true,
            ..Default::default()
        };
        assert_eq!(
            create_sequence(&seq, Dialect::Postgres),
            "CREATE SEQUENCE \"public\".\"order_seq\" START WITH 10 INCREMENT BY 5 MINVALUE 10 MAXVALUE 1000 CYCLE;"
        );
        // Defaults stay silent: increment 1 and no bounds emit a bare create.
        let plain = crate::Sequence { name: "s".into(), ..Default::default() };
        assert_eq!(create_sequence(&plain, Dialect::Postgres), "CREATE SEQUENCE \"s\";");
        assert_eq!(
            drop_sequence(Some("public"), "s", Dialect::Postgres),
            "DROP SEQUENCE IF EXISTS \"public\".\"s\";"
        );
    }
}

