#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L1: what a database looks like to Nook — catalog, result sets, edit plans, structure diffs, ER layout.

pub mod ddl;
pub mod diagram;
pub mod diff;
pub mod edit;

pub use diagram::{layout as layout_diagram, Diagram, DiagramEdge, DiagramNode};
pub use diff::{diff as diff_catalogs, Change, ChangeKind};
pub use edit::{plan as plan_edits, plan_update, EditKind, RowEdit};

use nook_core::{quote_qualified, sql_literal, Dialect, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Column {
    pub name: String,
    /// Declared type as the server reports it (`varchar(255)`, `int8`, `TEXT`).
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub auto_increment: bool,
    pub default: Option<String>,
    pub comment: String,
    pub ordinal: i64,
    pub max_len: Option<i64>,
    pub numeric_precision: Option<i64>,
    pub numeric_scale: Option<i64>,
    pub enum_values: Vec<String>,
}

impl Column {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Column {
            name: name.into(),
            type_name: type_name.into(),
            nullable: true,
            ..Column::default()
        }
    }

    /// `type` plus the modifiers a CREATE TABLE needs.
    pub fn definition(&self) -> String {
        let mut s = self.type_name.clone();
        if self.auto_increment {
            return s;
        }
        if !self.nullable {
            s.push_str(" NOT NULL");
        }
        if let Some(d) = &self.default {
            if !d.is_empty() {
                s.push_str(&format!(" DEFAULT {d}"));
            }
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_delete: String,
    pub on_update: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    #[default]
    Table,
    View,
    MatView,
    System,
}

impl TableKind {
    pub fn label(self) -> &'static str {
        match self {
            TableKind::Table => "table",
            TableKind::View => "view",
            TableKind::MatView => "materialized view",
            TableKind::System => "system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Table {
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub foreign_keys: Vec<ForeignKey>,
    pub row_estimate: Option<i64>,
    pub size_bytes: Option<i64>,
    pub comment: String,
}

impl Table {
    pub fn new(schema: Option<String>, name: impl Into<String>) -> Self {
        Table {
            schema,
            name: name.into(),
            ..Table::default()
        }
    }

    /// Stable `schema.name` (or bare name) key used by tabs, diffs and diagrams.
    pub fn key(&self) -> String {
        match &self.schema {
            Some(s) if !s.is_empty() => format!("{s}.{}", self.name),
            _ => self.name.clone(),
        }
    }

    pub fn primary_key(&self) -> Vec<String> {
        let mut pk: Vec<String> = self
            .columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.clone())
            .collect();
        if pk.is_empty() {
            if let Some(idx) = self.indexes.iter().find(|i| i.primary) {
                pk = idx.columns.clone();
            }
        }
        pk
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn column_names(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }
}


/// A trigger as the catalog reports it: the event wiring plus the body, for display and DDL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Trigger {
    pub schema: Option<String>,
    pub name: String,
    pub table: String,
    /// `BEFORE INSERT`, `AFTER UPDATE`, … as the server spells it.
    pub timing: String,
    /// One or more events: `INSERT`, `INSERT OR UPDATE`, `INSERT, UPDATE, DELETE`, …
    pub events: String,
    pub enabled: bool,
    /// The full `CREATE TRIGGER` statement, as read back from the server.
    pub ddl: String,
}

/// A stored function or procedure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Routine {
    pub schema: Option<String>,
    pub name: String,
    /// `function`, `procedure` — spelled the way the server does.
    pub kind: String,
    pub returns: String,
    pub comment: String,
    /// The full `CREATE FUNCTION`/`CREATE PROCEDURE` statement, as read back from the server.
    pub ddl: String,
}

/// A sequence (PostgreSQL; `AUTO_INCREMENT` metadata on MySQL tables covers the same need).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub schema: Option<String>,
    pub name: String,
    #[serde(default)]
    pub start: i64,
    #[serde(default = "one")]
    pub increment: i64,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub cycle: bool,
    pub last_value: Option<i64>,
}

fn one() -> i64 {
    1
}

/// `increment` defaults to 1, not 0: a bare `CREATE SEQUENCE` must stay silent.
impl Default for Sequence {
    fn default() -> Self {
        Sequence {
            increment: 1,
            ..Sequence {
                schema: None,
                name: String::new(),
                start: 0,
                increment: 0,
                min: None,
                max: None,
                cycle: false,
                last_value: None,
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Schema {
    pub name: String,
    pub tables: Vec<Table>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub routines: Vec<Routine>,
    #[serde(default)]
    pub sequences: Vec<Sequence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Catalog {
    /// Driver label the catalog came from, e.g. `PostgreSQL 15.4`.
    pub driver: String,
    pub database: Option<String>,
    pub databases: Vec<String>,
    pub schemas: Vec<Schema>,
    pub fetched_ms: u64,
}

impl Catalog {
    pub fn schema_names(&self) -> Vec<String> {
        self.schemas.iter().map(|s| s.name.clone()).collect()
    }

    pub fn schema(&self, name: &str) -> Option<&Schema> {
        self.schemas.iter().find(|s| s.name == name)
    }

    pub fn table_count(&self) -> usize {
        self.schemas.iter().map(|s| s.tables.len()).sum()
    }

    pub fn column_count(&self) -> usize {
        self.schemas
            .iter()
            .flat_map(|s| s.tables.iter())
            .map(|t| t.columns.len())
            .sum()
    }

    /// Find a table by `name`, `schema.name`, or the first match on a bare name.
    pub fn find_table(&self, schema: Option<&str>, name: &str) -> Option<&Table> {
        match schema {
            Some(s) if !s.is_empty() => self
                .schema(s)
                .and_then(|sc| sc.tables.iter().find(|t| t.name == name)),
            _ => self
                .schemas
                .iter()
                .find_map(|sc| {
                    sc.tables
                        .iter()
                        .find(|t| t.name == name && t.kind == TableKind::Table)
                })
                .or_else(|| {
                    self.schemas
                        .iter()
                        .find_map(|sc| sc.tables.iter().find(|t| t.name == name))
                }),
        }
    }

    /// Every object key, for completion and for the tree.
    pub fn table_keys(&self) -> Vec<String> {
        self.schemas
            .iter()
            .flat_map(|s| s.tables.iter().map(Table::key))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DbProcess {
    pub id: String,
    pub user: String,
    pub database: String,
    pub host: String,
    pub state: String,
    pub query: String,
    pub elapsed_ms: Option<i64>,
    pub started: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DbUser {
    pub name: String,
    pub host: String,
    pub is_super: bool,
    pub can_login: bool,
    pub expires: Option<String>,
}

impl DbUser {
    /// `neil@%` for MySQL, `neil` for Postgres and SQLite.
    pub fn display(&self) -> String {
        if self.host.is_empty() {
            self.name.clone()
        } else {
            format!("{}@{}", self.name, self.host)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ServerInfo {
    pub version: String,
    pub driver: String,
    pub database: Option<String>,
    pub host: String,
    pub uptime_s: Option<i64>,
    pub size_bytes: Option<i64>,
    pub read_only: bool,
}

/// One grid page returned by a driver. Always typed, always bounded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub column_types: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub affected: u64,
    pub last_insert_id: Option<u64>,
    pub elapsed_ms: f64,
    pub notices: Vec<String>,
    /// True when the driver stopped reading early because a limit was hit.
    pub truncated: bool,
}

impl ResultSet {
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
    pub fn is_tabular(&self) -> bool {
        self.column_count() > 0
    }

    /// Bound a result so a hostile `SELECT *` cannot exhaust memory.
    pub fn bound(mut self, max_rows: usize) -> Self {
        if self.rows.len() > max_rows {
            self.rows.truncate(max_rows);
            self.truncated = true;
        }
        self
    }

    pub fn to_json(&self) -> serde_json::Value {
        let rows: Vec<serde_json::Value> = self
            .rows
            .iter()
            .map(|r| {
                let mut obj = serde_json::Map::new();
                for (i, name) in self.columns.iter().enumerate() {
                    let v = r.get(i).cloned().unwrap_or(Value::Null);
                    obj.insert(name.clone(), v.to_json());
                }
                serde_json::Value::Object(obj)
            })
            .collect();
        serde_json::json!({
            "columns": self.columns,
            "types": self.column_types,
            "rows": rows,
            "affected": self.affected,
            "elapsed_ms": self.elapsed_ms,
            "truncated": self.truncated,
            "notices": self.notices,
        })
    }

    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        out.push_str(
            &self
                .columns
                .iter()
                .map(|c| csv_field(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
        for row in &self.rows {
            let cells: Vec<String> = (0..self.columns.len())
                .map(|i| csv_field(&row.get(i).cloned().unwrap_or(Value::Null).as_text()))
                .collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        out
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("| ");
        out.push_str(&self.columns.join(" | "));
        out.push_str(" |\n|");
        for _ in &self.columns {
            out.push_str("---|");
        }
        out.push('\n');
        for row in &self.rows {
            out.push_str("| ");
            let cells: Vec<String> = (0..self.columns.len())
                .map(|i| {
                    row.get(i)
                        .cloned()
                        .unwrap_or(Value::Null)
                        .as_text()
                        .replace('|', "\\|")
                        .replace('\n', " ")
                })
                .collect();
            out.push_str(&cells.join(" | "));
            out.push_str(" |\n");
        }
        out
    }

    /// `INSERT INTO …` script; also the payload of import, transfer and dump.
    pub fn to_sql_inserts(&self, schema: Option<&str>, table: &str, dialect: Dialect) -> String {
        let target = quote_qualified(schema, table, dialect);
        let cols = self
            .columns
            .iter()
            .map(|c| nook_core::quote_ident(c, dialect))
            .collect::<Vec<_>>()
            .join(", ");
        let mut out = String::new();
        for row in &self.rows {
            let vals = (0..self.columns.len())
                .map(|i| sql_literal(row.get(i).unwrap_or(&Value::Null), dialect))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("INSERT INTO {target} ({cols}) VALUES ({vals});\n"));
        }
        out
    }

    /// Column index by name, for grid edits.
    pub fn index_of(&self, column: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == column)
    }

    pub fn value_at(&self, row: usize, column: &str) -> Option<&Value> {
        let ci = self.index_of(column)?;
        self.rows.get(row)?.get(ci)
    }
}

/// CSV quoting per RFC 4180.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> Catalog {
        let mut users = Table::new(Some("public".into()), "users");
        let mut id = Column::new("id", "int8");
        id.primary_key = true;
        id.nullable = false;
        users.columns.push(id);
        users.columns.push(Column::new("email", "text"));
        let mut posts = Table::new(Some("public".into()), "posts");
        posts.columns.push(Column::new("id", "int8"));
        posts.columns.push(Column::new("author_id", "int8"));
        posts.foreign_keys.push(ForeignKey {
            name: "posts_author_fk".into(),
            columns: vec!["author_id".into()],
            ref_schema: Some("public".into()),
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            ..ForeignKey::default()
        });
        Catalog {
            driver: "PostgreSQL 15".into(),
            database: Some("shop".into()),
            databases: vec!["shop".into()],
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![users, posts],
                ..Default::default()
            }],
            fetched_ms: 3,
        }
    }

    #[test]
    fn catalog_lookup() {
        let c = sample_catalog();
        assert_eq!(c.table_count(), 2);
        assert_eq!(c.column_count(), 4);
        assert_eq!(
            c.find_table(None, "users").map(Table::key),
            Some("public.users".to_string())
        );
        assert!(c.find_table(Some("public"), "posts").is_some());
        assert!(c.find_table(Some("nope"), "posts").is_none());
        assert_eq!(
            c.find_table(None, "users").map(Table::primary_key),
            Some(vec!["id".to_string()])
        );
    }

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn resultset_exports() {
        let rs = ResultSet {
            columns: vec!["id".into(), "name".into()],
            column_types: vec!["int".into(), "text".into()],
            rows: vec![
                vec![Value::Int(1), Value::text("a,b")],
                vec![Value::Int(2), Value::Null],
            ],
            ..ResultSet::default()
        };
        assert_eq!(rs.to_csv(), "id,name\n1,\"a,b\"\n2,\n");
        assert!(rs.to_markdown().contains("| id | name |"));
        let sql = rs.to_sql_inserts(Some("public"), "t", Dialect::Postgres);
        assert!(sql.contains("INSERT INTO \"public\".\"t\" (\"id\", \"name\") VALUES (1, 'a,b');"));
        assert!(sql.contains("VALUES (2, NULL);"));
        assert_eq!(rs.clone().bound(1).row_count(), 1);
        assert!(rs.bound(1).truncated);
    }

    #[test]
    fn json_shape_has_named_columns() {
        let rs = ResultSet {
            columns: vec!["id".into()],
            column_types: vec!["int".into()],
            rows: vec![vec![Value::Int(7)]],
            ..ResultSet::default()
        };
        let j = rs.to_json();
        assert_eq!(j["rows"][0]["id"], serde_json::json!(7));
    }
}
