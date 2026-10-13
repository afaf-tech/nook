//! Structure synchronization: compare two catalogs and emit the DDL that closes the gap.

use nook_core::Dialect;

use crate::ddl;
use crate::{Catalog, Table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    AddTable,
    DropTable,
    AddColumn,
    DropColumn,
    AlterColumnType,
    SetNullable,
    AddIndex,
    DropIndex,
    AddForeignKey,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            ChangeKind::AddTable => "add table",
            ChangeKind::DropTable => "drop table",
            ChangeKind::AddColumn => "add column",
            ChangeKind::DropColumn => "drop column",
            ChangeKind::AlterColumnType => "change type",
            ChangeKind::SetNullable => "change nullability",
            ChangeKind::AddIndex => "add index",
            ChangeKind::DropIndex => "drop index",
            ChangeKind::AddForeignKey => "add foreign key",
        }
    }

    /// Dropping and type changes are the ones that lose data, so the UI asks twice.
    pub fn destructive(self) -> bool {
        matches!(
            self,
            ChangeKind::DropTable | ChangeKind::DropColumn | ChangeKind::AlterColumnType
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub kind: ChangeKind,
    pub schema: Option<String>,
    pub table: String,
    pub object: String,
    /// Statement that closes the gap, in the target dialect.
    pub sql: String,
    /// Not applied automatically even when selected: needs a rebuild or a manual step.
    pub manual: bool,
    pub note: String,
}

impl Change {
    pub fn destructive(&self) -> bool {
        self.kind.destructive()
    }
}

/// `source` is what you want, `target` is what is there. Emitted DDL runs on `target`.
pub fn diff(source: &Catalog, target: &Catalog, dialect: Dialect) -> Vec<Change> {
    let mut out = Vec::new();
    for sc in &source.schemas {
        for table in &sc.tables {
            match target.find_table(Some(&sc.name), &table.name) {
                None if table.kind != crate::TableKind::Table => out.push(Change {
                    kind: ChangeKind::AddTable,
                    schema: table.schema.clone(),
                    table: table.name.clone(),
                    object: table.key(),
                    sql: String::new(),
                    manual: true,
                    note: format!(
                        "{}: copy the definition by hand, Nook does not move views yet",
                        table.kind.label()
                    ),
                }),
                None => out.push(Change {
                    kind: ChangeKind::AddTable,
                    schema: table.schema.clone(),
                    table: table.name.clone(),
                    object: table.key(),
                    sql: ddl::create_table(table, dialect),
                    manual: false,
                    note: format!("{} columns", table.columns.len()),
                }),
                Some(existing) if table.kind != crate::TableKind::Table => {
                    let _ = existing;
                }
                Some(existing) => diff_table(table, existing, dialect, &mut out),
            }
        }
    }
    for sc in &target.schemas {
        for table in &sc.tables {
            if source.find_table(Some(&sc.name), &table.name).is_none() {
                let is_table = table.kind == crate::TableKind::Table;
                out.push(Change {
                    kind: ChangeKind::DropTable,
                    schema: table.schema.clone(),
                    table: table.name.clone(),
                    object: table.key(),
                    sql: if is_table {
                        ddl::drop_table(table.schema.as_deref(), &table.name, dialect, true)
                    } else {
                        String::new()
                    },
                    manual: !is_table,
                    note: if is_table {
                        "exists only in the target".into()
                    } else {
                        format!("{}: drop it by hand", table.kind.label())
                    },
                });
            }
        }
    }
    out
}

fn diff_table(table: &Table, existing: &Table, dialect: Dialect, out: &mut Vec<Change>) {
    let schema = table.schema.clone();
    for col in &table.columns {
        match existing.column(&col.name) {
            None => out.push(Change {
                kind: ChangeKind::AddColumn,
                schema: schema.clone(),
                table: table.name.clone(),
                object: format!("{}.{}", table.name, col.name),
                sql: ddl::add_column(schema.as_deref(), &table.name, col, dialect),
                manual: false,
                note: col.definition(),
            }),
            Some(have) => {
                if !type_equivalent(&col.type_name, &have.type_name) {
                    match ddl::alter_column_type(
                        schema.as_deref(),
                        &table.name,
                        &col.name,
                        &col.type_name,
                        dialect,
                    ) {
                        Ok(sql) => out.push(Change {
                            kind: ChangeKind::AlterColumnType,
                            schema: schema.clone(),
                            table: table.name.clone(),
                            object: format!("{}.{}", table.name, col.name),
                            sql,
                            manual: false,
                            note: format!("{} → {}", have.type_name, col.type_name),
                        }),
                        Err(e) => out.push(Change {
                            kind: ChangeKind::AlterColumnType,
                            schema: schema.clone(),
                            table: table.name.clone(),
                            object: format!("{}.{}", table.name, col.name),
                            sql: String::new(),
                            manual: true,
                            note: e.message().to_string(),
                        }),
                    }
                }
                if col.nullable != have.nullable {
                    match ddl::set_nullable(
                        schema.as_deref(),
                        &table.name,
                        &col.name,
                        col.nullable,
                        &col.type_name,
                        dialect,
                    ) {
                        Ok(sql) => out.push(Change {
                            kind: ChangeKind::SetNullable,
                            schema: schema.clone(),
                            table: table.name.clone(),
                            object: format!("{}.{}", table.name, col.name),
                            sql,
                            manual: false,
                            note: if col.nullable {
                                "drop NOT NULL".into()
                            } else {
                                "set NOT NULL".into()
                            },
                        }),
                        Err(e) => out.push(Change {
                            kind: ChangeKind::SetNullable,
                            schema: schema.clone(),
                            table: table.name.clone(),
                            object: format!("{}.{}", table.name, col.name),
                            sql: String::new(),
                            manual: true,
                            note: e.message().to_string(),
                        }),
                    }
                }
            }
        }
    }
    for col in &existing.columns {
        if table.column(&col.name).is_none() {
            out.push(Change {
                kind: ChangeKind::DropColumn,
                schema: schema.clone(),
                table: table.name.clone(),
                object: format!("{}.{}", table.name, col.name),
                sql: ddl::drop_column(schema.as_deref(), &table.name, &col.name, dialect),
                manual: false,
                note: "exists only in the target".into(),
            });
        }
    }
    for idx in &table.indexes {
        if idx.primary {
            continue;
        }
        if !existing.indexes.iter().any(|i| i.name == idx.name) {
            out.push(Change {
                kind: ChangeKind::AddIndex,
                schema: schema.clone(),
                table: table.name.clone(),
                object: idx.name.clone(),
                sql: ddl::create_index(schema.as_deref(), &table.name, idx, dialect),
                manual: false,
                note: idx.columns.join(", "),
            });
        }
    }
    for fk in &table.foreign_keys {
        if !existing.foreign_keys.iter().any(|f| f.name == fk.name) {
            out.push(Change {
                kind: ChangeKind::AddForeignKey,
                schema: schema.clone(),
                table: table.name.clone(),
                object: fk.name.clone(),
                sql: ddl::add_foreign_key(schema.as_deref(), &table.name, fk, dialect),
                manual: dialect == Dialect::Sqlite,
                note: if dialect == Dialect::Sqlite {
                    "SQLite cannot add a constraint to an existing table".into()
                } else {
                    format!("{} → {}", fk.columns.join(", "), fk.ref_table)
                },
            });
        }
    }
}

/// Types that look different but mean the same, so a diff is not noisy: `int4` vs `integer`, `varchar(20)` vs `character varying(20)`.
fn type_equivalent(want: &str, have: &str) -> bool {
    normalize(want) == normalize(have)
}

fn normalize(t: &str) -> String {
    let lower = t.to_ascii_lowercase();
    let base = lower.split('(').next().unwrap_or("").trim().to_string();
    let size = lower
        .split_once('(')
        .and_then(|(_, rest)| rest.split(')').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let base = match base.as_str() {
        "int8" | "bigint" => "bigint",
        "int4" | "integer" | "int" => "int",
        "int2" | "smallint" => "smallint",
        "serial" => "int",
        "bigserial" => "bigint",
        "bool" => "boolean",
        "varchar" | "character varying" => "varchar",
        "char" | "character" => "char",
        "float8" | "double precision" | "double" => "double",
        "float4" | "real" => "real",
        "numeric" | "decimal" => "numeric",
        "timestamptz" | "timestamp with time zone" => "timestamp",
        "timestamp" | "timestamp without time zone" | "datetime" => "timestamp",
        "text" | "longtext" | "mediumtext" | "clob" => "text",
        other => other,
    }
    .to_string();
    if size.is_empty() {
        base
    } else {
        format!("{base}({size})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, Index, Schema};

    fn cat(tables: Vec<Table>) -> Catalog {
        Catalog {
            driver: "test".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables,
            }],
            ..Catalog::default()
        }
    }

    fn table(name: &str, cols: &[(&str, &str, bool)]) -> Table {
        let mut t = Table::new(Some("public".into()), name);
        for (c, ty, nn) in cols {
            let mut col = Column::new(*c, *ty);
            col.nullable = !*nn;
            if *c == "id" {
                col.primary_key = true;
            }
            t.columns.push(col);
        }
        t
    }

    #[test]
    fn added_table_and_column_and_dropped_column() {
        let source = cat(vec![
            table(
                "users",
                &[("id", "int", true), ("email", "varchar(120)", true)],
            ),
            table("audit", &[("id", "int", true)]),
        ]);
        let target = cat(vec![table(
            "users",
            &[("id", "int", true), ("old", "text", false)],
        )]);
        let changes = diff(&source, &target, Dialect::Postgres);
        let kinds: Vec<ChangeKind> = changes.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&ChangeKind::AddTable));
        assert!(kinds.contains(&ChangeKind::AddColumn));
        assert!(kinds.contains(&ChangeKind::DropColumn));
        let add = changes
            .iter()
            .find(|c| c.kind == ChangeKind::AddColumn)
            .expect("add column");
        assert!(add
            .sql
            .contains("ADD COLUMN \"email\" varchar(120) NOT NULL"));
        assert!(!add.destructive());
        assert!(changes
            .iter()
            .find(|c| c.kind == ChangeKind::DropColumn)
            .expect("drop")
            .destructive());
    }

    #[test]
    fn views_are_reported_but_never_silently_created() {
        let mut view = Table::new(Some("public".into()), "v_users");
        view.kind = crate::TableKind::View;
        let source = cat(vec![view]);
        let target = cat(vec![]);
        let changes = diff(&source, &target, Dialect::Postgres);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].manual);
        assert!(changes[0].sql.is_empty());
        assert!(changes[0].note.contains("views"));
    }

    #[test]
    fn equivalent_types_are_not_a_change() {
        let source = cat(vec![table(
            "t",
            &[("id", "integer", true), ("n", "int8", false)],
        )]);
        let target = cat(vec![table(
            "t",
            &[("id", "int4", true), ("n", "bigint", false)],
        )]);
        assert!(diff(&source, &target, Dialect::Postgres).is_empty());
        // but a size change is
        let source = cat(vec![table("t", &[("v", "varchar(50)", false)])]);
        let target = cat(vec![table("t", &[("v", "varchar(20)", false)])]);
        assert_eq!(diff(&source, &target, Dialect::Postgres).len(), 1);
    }

    #[test]
    fn sqlite_type_change_becomes_a_manual_note() {
        let source = cat(vec![table("t", &[("v", "bigint", false)])]);
        let target = cat(vec![table("t", &[("v", "int", false)])]);
        let changes = diff(&source, &target, Dialect::Sqlite);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].manual);
        assert!(changes[0].sql.is_empty());
        assert!(changes[0].note.contains("rebuild"));
    }

    #[test]
    fn indexes_and_tables_only_in_target() {
        let mut source_table = table("t", &[("id", "int", true), ("email", "text", false)]);
        source_table.indexes.push(Index {
            name: "ix_email".into(),
            columns: vec!["email".into()],
            unique: false,
            ..Index::default()
        });
        let source = cat(vec![source_table]);
        let target = cat(vec![
            table("t", &[("id", "int", true), ("email", "text", false)]),
            table("gone", &[("id", "int", true)]),
        ]);
        let changes = diff(&source, &target, Dialect::Postgres);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|c| c.kind == ChangeKind::AddIndex));
        assert!(changes
            .iter()
            .any(|c| c.kind == ChangeKind::DropTable && c.sql.contains("DROP TABLE IF EXISTS")));
    }
}
