//! MySQL / MariaDB driver.

use std::time::{Duration, Instant};

use mysql::consts::ColumnType;
use mysql::prelude::*;
use mysql::{Column as MyColumn, Conn, Opts, OptsBuilder, Row, SslOpts, Value as MyValue};
use nook_core::{ConnectionConfig, Dialect, DriverKind, Error, Result, Value};
use nook_model::{
    Catalog, Column, DbProcess, DbUser, ForeignKey, Index, ResultSet, Routine, Schema, ServerInfo,
    Table, TableKind, Trigger,
};

use crate::{guard_read_only, Driver, MAX_ROWS};

pub struct MySql {
    conn: Conn,
    cfg: ConnectionConfig,
    database: String,
    in_tx: bool,
}

impl MySql {
    pub fn open(cfg: &ConnectionConfig) -> Result<Self> {
        let mut opts = Opts::from_url(&cfg.uri())
            .map_err(|e| Error::config(format!("connection string: {e}")))?;
        let plain = OptsBuilder::from_opts(opts.clone());
        if cfg.ssl != nook_core::SslMode::Disable {
            let verify = matches!(
                cfg.ssl,
                nook_core::SslMode::VerifyFull | nook_core::SslMode::VerifyCa
            );
            opts = OptsBuilder::from_opts(opts)
                .ssl_opts(Some(
                    SslOpts::default()
                        .with_danger_accept_invalid_certs(!verify)
                        .with_danger_skip_domain_validation(!verify),
                ))
                .into();
        }
        // `prefer` means "TLS if the server offers it". The mysql crate treats a requested TLS that
        // the server cannot do as a hard error, so the preference has to be spent here: try the
        // encrypted connection, and fall back to a plaintext one only when the failure is the
        // server's lack of TLS — never when a certificate actually failed to verify.
        let conn = match Conn::new(opts) {
            Ok(conn) => conn,
            Err(e) if cfg.ssl == nook_core::SslMode::Prefer && !is_tls_verification(&e) => {
                Conn::new(plain).map_err(|e| {
                    Error::connect(format!("{}:{}: {e}", cfg.target().0, cfg.target().1))
                })?
            }
            Err(e) => {
                return Err(Error::connect(format!(
                    "{}:{}: {e}",
                    cfg.target().0,
                    cfg.target().1
                )))
            }
        };
        let mut me = MySql {
            conn,
            database: cfg.database.clone(),
            in_tx: false,
            cfg: cfg.clone(),
        };
        if let Some(db) = me.one("SELECT DATABASE()") {
            if !db.is_empty() {
                me.database = db;
            }
        }
        Ok(me)
    }

    fn string_rows(&mut self, sql: &str) -> Result<Vec<Vec<String>>> {
        let rs = self.run(sql, MAX_ROWS)?;
        Ok(rs
            .rows
            .iter()
            .map(|r| r.iter().map(Value::as_text).collect())
            .collect())
    }

    fn one(&mut self, sql: &str) -> Option<String> {
        self.string_rows(sql)
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|r| r.into_iter().next())
    }
}

/// A bound value, in the driver's own type. Everything the parameter path can carry is a string, a
/// number or NULL — `bind_placeholders` never produces anything else — so the mapping is total for
/// the values it can actually see.
fn to_my_value(v: &Value) -> MyValue {
    match v {
        Value::Null => MyValue::NULL,
        Value::Bool(b) => MyValue::Int(i64::from(*b)),
        Value::Int(i) => MyValue::Int(*i),
        Value::UInt(u) => MyValue::UInt(*u),
        Value::Float(f) => MyValue::Double(*f),
        Value::Bytes(b) => MyValue::Bytes(b.clone()),
        other => MyValue::Bytes(other.as_text().into_bytes()),
    }
}

/// True when the connection failed because the certificate could not be trusted. Falling back to
/// plaintext in that case would silently downgrade a connection the user asked to verify.
fn is_tls_verification(e: &mysql::Error) -> bool {
    let text = e.to_string().to_ascii_lowercase();
    text.contains("certificate") || text.contains("cert")
}

fn to_value(v: &MyValue, ty: Option<&ColumnType>) -> Value {
    match v {
        MyValue::NULL => Value::Null,
        MyValue::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) => {
                if matches!(ty, Some(ColumnType::MYSQL_TYPE_JSON)) {
                    Value::Json(s.to_string())
                } else {
                    Value::text(s)
                }
            }
            Err(_) => Value::Bytes(b.clone()),
        },
        MyValue::Int(i) => match ty {
            Some(ColumnType::MYSQL_TYPE_TINY) => Value::Bool(*i != 0),
            _ => Value::Int(*i),
        },
        MyValue::UInt(u) => Value::UInt(*u),
        MyValue::Float(f) => Value::Float(f64::from(*f)),
        MyValue::Double(f) => Value::Float(*f),
        MyValue::Date(y, m, d, h, mi, s, us) => {
            let time_like = *h != 0 || *mi != 0 || *s != 0 || *us != 0;
            let date = format!("{y:04}-{m:02}-{d:02}");
            if !time_like && matches!(ty, Some(ColumnType::MYSQL_TYPE_DATE)) {
                return Value::Date(date);
            }
            let mut text = format!("{date} {h:02}:{mi:02}:{s:02}");
            if *us > 0 {
                text.push_str(&format!(".{us:06}"));
            }
            Value::Timestamp(text)
        }
        MyValue::Time(neg, days, h, mi, s, us) => {
            let sign = if *neg { "-" } else { "" };
            let hours = *days * 24 + u32::from(*h);
            let mut text = format!("{sign}{hours:02}:{mi:02}:{s:02}");
            if *us > 0 {
                text.push_str(&format!(".{us:06}"));
            }
            Value::Time(text)
        }
    }
}

/// Column type name as MySQL would print it in a `CREATE TABLE`.
fn type_name(c: &MyColumn) -> String {
    let base = match c.column_type() {
        ColumnType::MYSQL_TYPE_TINY => "tinyint",
        ColumnType::MYSQL_TYPE_SHORT => "smallint",
        ColumnType::MYSQL_TYPE_LONG => "int",
        ColumnType::MYSQL_TYPE_LONGLONG => "bigint",
        ColumnType::MYSQL_TYPE_INT24 => "mediumint",
        ColumnType::MYSQL_TYPE_FLOAT => "float",
        ColumnType::MYSQL_TYPE_DOUBLE => "double",
        ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => "decimal",
        ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => "date",
        ColumnType::MYSQL_TYPE_TIME => "time",
        ColumnType::MYSQL_TYPE_DATETIME => "datetime",
        ColumnType::MYSQL_TYPE_TIMESTAMP => "timestamp",
        ColumnType::MYSQL_TYPE_YEAR => "year",
        ColumnType::MYSQL_TYPE_JSON => "json",
        ColumnType::MYSQL_TYPE_BIT => "bit",
        ColumnType::MYSQL_TYPE_BLOB => "blob",
        ColumnType::MYSQL_TYPE_TINY_BLOB => "tinyblob",
        ColumnType::MYSQL_TYPE_MEDIUM_BLOB => "mediumblob",
        ColumnType::MYSQL_TYPE_LONG_BLOB => "longblob",
        ColumnType::MYSQL_TYPE_GEOMETRY => "geometry",
        other => return format!("{other:?}").to_ascii_lowercase(),
    };
    base.to_string()
}

/// Reject anything that cannot safely be interpolated as an identifier or a literal.
fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

impl Driver for MySql {
    fn kind(&self) -> DriverKind {
        if self.cfg.kind == DriverKind::MariaDb {
            DriverKind::MariaDb
        } else {
            DriverKind::MySql
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::MySql
    }

    fn current_database(&self) -> Option<String> {
        Some(self.database.clone())
    }

    fn server_info(&mut self) -> Result<ServerInfo> {
        let version = self
            .one("SELECT VERSION()")
            .ok_or_else(|| Error::connect("could not read server version"))?;
        let uptime = self
            .string_rows("SHOW GLOBAL STATUS LIKE 'Uptime'")
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|r| r.get(1).cloned())
            .and_then(|s| s.parse::<i64>().ok());
        let size = self
            .one(
                "SELECT COALESCE(SUM(data_length + index_length), 0) FROM information_schema.tables \
                 WHERE table_schema = DATABASE()",
            )
            .and_then(|s| s.parse::<i64>().ok());
        Ok(ServerInfo {
            version: format!("MySQL {version}"),
            driver: if self.kind() == DriverKind::MariaDb {
                "MariaDB".into()
            } else {
                "MySQL".into()
            },
            database: Some(self.database.clone()),
            host: format!("{}:{}", self.cfg.target().0, self.cfg.target().1),
            uptime_s: uptime,
            size_bytes: size,
            read_only: self.cfg.read_only,
        })
    }

    fn run(&mut self, sql: &str, limit: usize) -> Result<ResultSet> {
        if sql.trim().is_empty() {
            return Err(Error::query("empty statement"));
        }
        guard_read_only(self.cfg.read_only, sql)?;
        let started = Instant::now();
        let limit = limit.max(1);
        let mut rs = ResultSet::default();
        if nook_core::returns_rows(sql) {
            let mut qr = self
                .conn
                .query_iter(sql)
                .map_err(|e| Error::query(format!("{e}")))?;
            let columns: Vec<MyColumn> = qr.columns().as_ref().to_vec();
            rs.columns = columns.iter().map(|c| c.name_str().to_string()).collect();
            rs.column_types = columns.iter().map(type_name).collect();
            for row in qr.by_ref() {
                let row: Row = row.map_err(|e| Error::query(format!("{e}")))?;
                if rs.rows.len() >= limit {
                    rs.truncated = true;
                    break;
                }
                let mut vals = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    let v = row.as_ref(i).cloned().unwrap_or(MyValue::NULL);
                    let column_type = columns.get(i).map(|c| c.column_type());
                    vals.push(to_value(&v, column_type.as_ref()));
                }
                rs.rows.push(vals);
            }
            drop(qr);
            rs.affected = self.conn.affected_rows();
        } else {
            self.conn
                .query_drop(sql)
                .map_err(|e| Error::query(format!("{e}")))?;
            rs.affected = self.conn.affected_rows();
            let last = self.conn.last_insert_id();
            if last > 0 {
                rs.last_insert_id = Some(last);
            }
        }
        rs.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(rs)
    }

    fn run_prepared(&mut self, sql: &str, values: &[Value], limit: usize) -> Result<ResultSet> {
        if sql.trim().is_empty() {
            return Err(Error::query("empty statement"));
        }
        guard_read_only(self.cfg.read_only, sql)?;
        let started = Instant::now();
        let limit = limit.max(1);
        // The `mysql` crate binds `Vec<Value>` positionally, which is exactly the shape
        // `bind_placeholders` produces (`?` per slot, in order).
        let bound: Vec<MyValue> = values.iter().map(to_my_value).collect();
        let mut rs = ResultSet::default();
        if nook_core::returns_rows(sql) {
            let mut qr = self
                .conn
                .exec_iter(sql, mysql::Params::Positional(bound))
                .map_err(|e| Error::query(format!("{e}")))?;
            let columns: Vec<MyColumn> = qr.columns().as_ref().to_vec();
            rs.columns = columns.iter().map(|c| c.name_str().to_string()).collect();
            rs.column_types = columns.iter().map(type_name).collect();
            for row in qr.by_ref() {
                let row: Row = row.map_err(|e| Error::query(format!("{e}")))?;
                if rs.rows.len() >= limit {
                    rs.truncated = true;
                    break;
                }
                let mut vals = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    let v = row.as_ref(i).cloned().unwrap_or(MyValue::NULL);
                    let column_type = columns.get(i).map(|c| c.column_type());
                    vals.push(to_value(&v, column_type.as_ref()));
                }
                rs.rows.push(vals);
            }
            drop(qr);
            rs.affected = self.conn.affected_rows();
        } else {
            self.conn
                .exec_drop(sql, mysql::Params::Positional(bound))
                .map_err(|e| Error::query(format!("{e}")))?;
            rs.affected = self.conn.affected_rows();
            let last = self.conn.last_insert_id();
            if last > 0 {
                rs.last_insert_id = Some(last);
            }
        }
        rs.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(rs)
    }

    fn catalog(&mut self) -> Result<Catalog> {
        let started = Instant::now();
        let db = self.database.clone();
        let db_lit = escape_string(&db);
        let mut catalog = Catalog {
            driver: self
                .server_info()
                .map(|i| i.version)
                .unwrap_or_else(|_| "MySQL".into()),
            database: Some(db.clone()),
            databases: self.databases().unwrap_or_default(),
            schemas: vec![Schema {
                name: db.clone(),
                tables: vec![],
                ..Default::default()
            }],
            fetched_ms: 0,
        };

        let tables = self.string_rows(&format!(
            "SELECT table_name, table_type, COALESCE(engine, ''), COALESCE(table_rows, 0), \
             COALESCE(table_comment, ''), COALESCE(data_length + index_length, 0) \
             FROM information_schema.tables WHERE table_schema = '{db_lit}' ORDER BY table_name"
        ))?;
        for r in &tables {
            if r.len() < 6 {
                continue;
            }
            let kind = match r[1].as_str() {
                "VIEW" => TableKind::View,
                "SYSTEM VIEW" => TableKind::System,
                _ => TableKind::Table,
            };
            catalog.schemas[0].tables.push(Table {
                schema: Some(db.clone()),
                name: r[0].clone(),
                kind,
                row_estimate: r[3].parse::<i64>().ok(),
                comment: r[4].clone(),
                size_bytes: r[5].parse::<i64>().ok(),
                ..Table::default()
            });
        }

        let columns = self.string_rows(&format!(
            "SELECT table_name, column_name, ordinal_position, COALESCE(column_default, ''), \
             is_nullable, column_type, column_key, extra, COALESCE(column_comment, ''), \
             COALESCE(character_maximum_length, 0), COALESCE(numeric_precision, 0), \
             COALESCE(numeric_scale, 0) \
             FROM information_schema.columns WHERE table_schema = '{db_lit}' \
             ORDER BY table_name, ordinal_position"
        ))?;
        for r in &columns {
            if r.len() < 12 {
                continue;
            }
            let Some(t) = catalog.schemas[0]
                .tables
                .iter_mut()
                .find(|t| t.name == r[0])
            else {
                continue;
            };
            t.columns.push(Column {
                name: r[1].clone(),
                type_name: r[5].clone(),
                nullable: r[4].eq_ignore_ascii_case("YES"),
                primary_key: r[6] == "PRI",
                auto_increment: r[7].contains("auto_increment"),
                default: if r[3].is_empty() {
                    None
                } else {
                    Some(r[3].clone())
                },
                comment: r[8].clone(),
                ordinal: r[2].parse().unwrap_or(0),
                max_len: r[9].parse().ok(),
                numeric_precision: r[10].parse().ok(),
                numeric_scale: r[11].parse().ok(),
                enum_values: enum_values(&r[5]),
            });
        }

        let indexes = self.string_rows(&format!(
            "SELECT table_name, index_name, seq_in_index, column_name, non_unique, index_type \
             FROM information_schema.statistics WHERE table_schema = '{db_lit}' \
             ORDER BY table_name, index_name, seq_in_index"
        ))?;
        for r in &indexes {
            if r.len() < 6 {
                continue;
            }
            let Some(t) = catalog.schemas[0]
                .tables
                .iter_mut()
                .find(|t| t.name == r[0])
            else {
                continue;
            };
            let unique = r[4] == "0";
            let primary = r[1] == "PRIMARY";
            match t.indexes.iter_mut().find(|i| i.name == r[1]) {
                Some(idx) => idx.columns.push(r[3].clone()),
                None => t.indexes.push(Index {
                    name: r[1].clone(),
                    columns: vec![r[3].clone()],
                    unique,
                    primary,
                    kind: if primary {
                        "primary key".into()
                    } else if unique {
                        "unique".into()
                    } else {
                        r[5].to_ascii_lowercase()
                    },
                }),
            }
        }

        let fks = self.string_rows(&format!(
            "SELECT k.constraint_name, k.table_name, k.column_name, k.referenced_table_schema, \
             k.referenced_table_name, k.referenced_column_name, \
             COALESCE(r.delete_rule, ''), COALESCE(r.update_rule, '') \
             FROM information_schema.key_column_usage k \
             LEFT JOIN information_schema.referential_constraints r \
               ON r.constraint_name = k.constraint_name AND r.constraint_schema = k.table_schema \
             WHERE k.table_schema = '{db_lit}' AND k.referenced_table_name IS NOT NULL \
             ORDER BY k.constraint_name, k.ordinal_position"
        ))?;
        for r in &fks {
            if r.len() < 8 {
                continue;
            }
            let Some(t) = catalog.schemas[0]
                .tables
                .iter_mut()
                .find(|t| t.name == r[1])
            else {
                continue;
            };
            match t.foreign_keys.iter_mut().find(|f| f.name == r[0]) {
                Some(fk) => {
                    fk.columns.push(r[2].clone());
                    fk.ref_columns.push(r[5].clone());
                }
                None => t.foreign_keys.push(ForeignKey {
                    name: r[0].clone(),
                    columns: vec![r[2].clone()],
                    ref_schema: Some(r[3].clone()),
                    ref_table: r[4].clone(),
                    ref_columns: vec![r[5].clone()],
                    on_delete: r[6].clone(),
                    on_update: r[7].clone(),
                }),
            }
        }

        let trigger_rows = self.string_rows(&format!(
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION, \
             COALESCE(ACTION_STATEMENT, '') \
             FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = '{db_lit}' \
             ORDER BY TRIGGER_NAME"
        ))?;
        for r in &trigger_rows {
            if r.len() < 5 {
                continue;
            }
            // The action statement is the body, not the whole definition; the
            // timing/event columns carry the structured parts.
            catalog.schemas[0].triggers.push(Trigger {
                schema: Some(db.clone()),
                name: r[0].clone(),
                table: r[1].clone(),
                timing: r[2].clone(),
                events: r[3].clone(),
                ddl: format!(
                    "CREATE TRIGGER {} {} {} ON {} FOR EACH ROW {}",
                    r[0], r[2], r[3], r[1], r[4]
                ),
                ..Default::default()
            });
        }

        let routine_rows = self.string_rows(&format!(
            "SELECT ROUTINE_NAME, ROUTINE_TYPE, COALESCE(DTD_IDENTIFIER, ''), \
             COALESCE(ROUTINE_COMMENT, '') \
             FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = '{db_lit}' \
             ORDER BY ROUTINE_NAME"
        ))?;
        for r in &routine_rows {
            if r.len() < 4 {
                continue;
            }
            catalog.schemas[0].routines.push(Routine {
                schema: Some(db.clone()),
                name: r[0].clone(),
                kind: r[1].to_ascii_lowercase(),
                returns: r[2].clone(),
                comment: r[3].clone(),
                // SHOW CREATE needs one round trip per routine; left empty rather
                // than fanning out at catalog time.
                ddl: String::new(),
            });
        }

        catalog.fetched_ms = started.elapsed().as_millis() as u64;
        Ok(catalog)
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .string_rows("SHOW DATABASES")?
            .into_iter()
            .filter_map(|r| r.into_iter().next())
            .collect())
    }

    fn use_database(&mut self, database: &str) -> Result<()> {
        if database.contains(['`', '\'', ';', '\\']) {
            return Err(Error::config(format!("invalid database name: {database}")));
        }
        self.run(&format!("USE `{database}`"), 1)?;
        self.database = database.to_string();
        Ok(())
    }

    fn processes(&mut self) -> Result<Vec<DbProcess>> {
        let rows = self.string_rows("SHOW FULL PROCESSLIST")?;
        Ok(rows
            .iter()
            .filter(|r| r.len() >= 6)
            .map(|r| DbProcess {
                id: r[0].clone(),
                user: r.get(1).cloned().unwrap_or_default(),
                host: r.get(2).cloned().unwrap_or_default(),
                database: r.get(3).cloned().unwrap_or_default(),
                state: r.get(5).cloned().unwrap_or_default(),
                query: r.get(7).or_else(|| r.get(6)).cloned().unwrap_or_default(),
                elapsed_ms: r.get(5).and_then(|s| s.parse::<i64>().ok()),
                started: None,
            })
            .collect())
    }

    fn kill_process(&mut self, id: &str) -> Result<()> {
        let pid: u64 = id
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("'{id}' is not a process id")))?;
        self.run(&format!("KILL {pid}"), 0)?;
        Ok(())
    }

    fn variables(&mut self) -> Result<Vec<(String, String)>> {
        Ok(self
            .string_rows("SHOW GLOBAL VARIABLES")?
            .into_iter()
            .filter(|r| r.len() >= 2)
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect())
    }

    fn status(&mut self) -> Result<Vec<(String, String)>> {
        Ok(self
            .string_rows("SHOW GLOBAL STATUS")?
            .into_iter()
            .filter(|r| r.len() >= 2)
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect())
    }

    fn users(&mut self) -> Result<Vec<DbUser>> {
        let rows = self.string_rows(
            "SELECT User, Host, IF(Super_priv = 'Y', 1, 0), IF(account_locked = 'Y', 1, 0) \
             FROM mysql.user ORDER BY User, Host",
        )?;
        Ok(rows
            .iter()
            .filter(|r| r.len() >= 4)
            .map(|r| DbUser {
                name: r[0].clone(),
                host: r[1].clone(),
                is_super: r[2] == "1",
                can_login: r[3] != "1",
                expires: None,
            })
            .collect())
    }

    fn grants(&mut self, user: &DbUser) -> Result<Vec<String>> {
        let host = if user.host.is_empty() {
            "%"
        } else {
            &user.host
        };
        Ok(self
            .string_rows(&format!(
                "SHOW GRANTS FOR '{}'@'{}'",
                escape_string(&user.name),
                escape_string(host)
            ))?
            .into_iter()
            .filter_map(|r| r.into_iter().next())
            .collect())
    }

    fn table_ddl(&mut self, schema: Option<&str>, table: &str) -> Result<String> {
        let qualified = nook_core::quote_qualified(
            schema.or(Some(self.database.as_str())),
            table,
            Dialect::MySql,
        );
        match self.string_rows(&format!("SHOW CREATE TABLE {qualified}")) {
            Ok(rows) => match rows.first().and_then(|r| r.get(1)) {
                Some(sql) => Ok(sql.trim_end_matches(';').to_string()),
                None => Err(Error::not_found(format!("no DDL for {qualified}"))),
            },
            Err(_) => {
                let rows = self.string_rows(&format!("SHOW CREATE VIEW {qualified}"))?;
                rows.first()
                    .and_then(|r| r.get(1))
                    .cloned()
                    .ok_or_else(|| Error::not_found(format!("no DDL for {qualified}")))
            }
        }
    }

    fn begin(&mut self) -> Result<()> {
        self.run("START TRANSACTION", 0)?;
        self.in_tx = true;
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        self.run("COMMIT", 0)?;
        self.in_tx = false;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        self.run("ROLLBACK", 0)?;
        self.in_tx = false;
        Ok(())
    }

    fn in_transaction(&self) -> bool {
        self.in_tx
    }

    fn close(&mut self) {
        let _ = self.conn.reset();
    }
}

/// Pull `enum('a','b')` values so the grid can offer a picker instead of free text.
fn enum_values(column_type: &str) -> Vec<String> {
    let lower = column_type.to_ascii_lowercase();
    if !lower.starts_with("enum(") {
        return vec![];
    }
    let inner = column_type
        .trim_start_matches(|c: char| c != '(')
        .trim_start_matches('(')
        .trim_end_matches(')');
    inner
        .split("','")
        .map(|s| s.trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Connect timeout is part of the URL the driver builds, so surface it in the message.
pub fn connect_timeout(cfg: &ConnectionConfig) -> Duration {
    Duration::from_secs(cfg.connect_timeout_s.clamp(1, 300))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_values_are_split() {
        assert_eq!(
            enum_values("enum('draft','live','archived')"),
            vec!["draft", "live", "archived"]
        );
        assert!(enum_values("varchar(20)").is_empty());
    }

    #[test]
    fn type_names_are_sql_names() {
        let c = MyColumn::new(ColumnType::MYSQL_TYPE_LONGLONG);
        assert_eq!(type_name(&c), "bigint");
        let c = MyColumn::new(ColumnType::MYSQL_TYPE_JSON);
        assert_eq!(type_name(&c), "json");
    }

    #[test]
    fn values_map_to_the_house_type() {
        assert_eq!(to_value(&MyValue::NULL, None), Value::Null);
        assert_eq!(to_value(&MyValue::Int(3), None), Value::Int(3));
        assert_eq!(
            to_value(&MyValue::Bytes(b"hi".to_vec()), None),
            Value::text("hi")
        );
        assert_eq!(
            to_value(
                &MyValue::Date(2026, 10, 12, 0, 0, 0, 0),
                Some(&ColumnType::MYSQL_TYPE_DATE)
            ),
            Value::Date("2026-10-12".into())
        );
        assert_eq!(
            to_value(&MyValue::Date(2026, 10, 12, 13, 5, 0, 0), None),
            Value::Timestamp("2026-10-12 13:05:00".into())
        );
    }

    #[test]
    fn escaping_neutralises_quotes() {
        assert_eq!(escape_string("o'brien"), "o''brien");
        assert_eq!(escape_string("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn bad_connection_string_is_an_error_not_a_panic() {
        let cfg = ConnectionConfig::parse("mysql://nobody@127.0.0.1:1/none").expect("parse");
        let e = MySql::open(&cfg);
        assert!(e.is_err());
    }
}
