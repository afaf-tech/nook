//! PostgreSQL driver.
//!
//! Uses the simple query protocol on purpose: it hands every value back as text, so `numeric`,
//! `int8` and `timestamptz` keep full precision without a per-type decoder, and multi-statement
//! scripts work the way the editor submits them.
//! ponytail: text protocol only, no binary results and no bind parameters — add a prepared path
//! when a feature needs parameter binding or binary blobs at scale.

use std::time::{Duration, Instant};

use nook_core::{ConnectionConfig, Dialect, DriverKind, Error, Result, Value};
use nook_model::{
    Catalog, Column, DbProcess, DbUser, ForeignKey, Index, ResultSet, Schema, ServerInfo, Table,
    TableKind,
};
use postgres::types::Type;
use postgres::{Client, NoTls, SimpleQueryMessage};
use postgres_native_tls::MakeTlsConnector;

use crate::{guard_read_only, Driver, MAX_ROWS};

pub struct Postgres {
    client: Client,
    cfg: ConnectionConfig,
    database: String,
    in_tx: bool,
}

impl Postgres {
    pub fn open(cfg: &ConnectionConfig) -> Result<Self> {
        let client = connect(cfg)?;
        Ok(Postgres {
            client,
            database: cfg.database.clone(),
            in_tx: false,
            cfg: cfg.clone(),
        })
    }

    fn simple(&mut self, sql: &str) -> Result<Vec<SimpleQueryMessage>> {
        self.client
            .simple_query(sql)
            .map_err(|e| Error::query(describe(&e, None)))
    }

    /// Column names and server types for a statement. Prepared but never executed: this is how the
    /// grid gets real `numeric(10,2)` / `timestamptz` labels while values still arrive as text.
    fn declared_columns(&mut self, sql: &str) -> Vec<(String, Type)> {
        match self.client.prepare(sql) {
            Ok(stmt) => stmt
                .columns()
                .iter()
                .map(|c| (c.name().to_string(), c.type_().clone()))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn string_rows(&mut self, sql: &str) -> Result<Vec<Vec<String>>> {
        let messages = self.simple(sql)?;
        let mut out: Vec<Vec<String>> = Vec::new();
        let mut ncols = 0usize;
        for m in messages {
            match m {
                SimpleQueryMessage::RowDescription(desc) => ncols = desc.len(),
                SimpleQueryMessage::Row(r) => {
                    if out.len() >= MAX_ROWS {
                        break;
                    }
                    let mut row = Vec::with_capacity(ncols);
                    for i in 0..ncols {
                        row.push(r.get(i).unwrap_or("").to_string());
                    }
                    out.push(row);
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn one(&mut self, sql: &str) -> Option<String> {
        self.string_rows(sql)
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|r| r.into_iter().next())
    }
}

fn connect(cfg: &ConnectionConfig) -> Result<Client> {
    let (host, port) = cfg.target();
    let mut pc = postgres::Config::new();
    pc.host(&host)
        .port(port)
        .connect_timeout(Duration::from_secs(cfg.connect_timeout_s.clamp(1, 300)));
    if !cfg.user.is_empty() {
        pc.user(&cfg.user);
    }
    if let Some(pw) = &cfg.password {
        pc.password(pw);
    }
    if !cfg.database.is_empty() {
        pc.dbname(&cfg.database);
    }
    pc.ssl_mode(match cfg.ssl {
        nook_core::SslMode::Disable => postgres::config::SslMode::Disable,
        nook_core::SslMode::Prefer => postgres::config::SslMode::Prefer,
        _ => postgres::config::SslMode::Require,
    });
    let target = if cfg.ssh.enabled {
        format!("{host}:{port} via ssh tunnel")
    } else {
        format!("{host}:{port}")
    };
    if cfg.ssl == nook_core::SslMode::Disable {
        return pc
            .connect(NoTls)
            .map_err(|e| Error::connect(format!("{target}: {}", describe(&e, Some(cfg)))));
    }
    let verify = matches!(
        cfg.ssl,
        nook_core::SslMode::VerifyFull | nook_core::SslMode::VerifyCa
    );
    let builder = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(!verify)
        .danger_accept_invalid_hostnames(!verify)
        .build()
        .map_err(|e| Error::config(format!("tls: {e}")))?;
    pc.connect(MakeTlsConnector::new(builder))
        .map_err(|e| Error::connect(format!("{target}: {}", describe(&e, Some(cfg)))))
}

/// The sentence to show for a failed connect. tokio-postgres's `Display` prints the error's *kind*
/// alone, so the server's own answer to a rejected login arrives as the bare "db error", and its
/// "password missing" — raised when the server asks and the config carries none — arrives as the bare
/// "invalid configuration". Both leave the user with nothing to act on.
fn describe(error: &postgres::Error, cfg: Option<&ConnectionConfig>) -> String {
    if let Some(db) = error.as_db_error() {
        return db.to_string();
    }
    let text = error.to_string();
    // On a connect, host, port and user are always set, which leaves "password missing" as the one
    // `Kind::Config` failure there is — and `Kind::Config`'s own message never reaches `Display`. If
    // the wording ever changes, the raw text is what the user reads: no worse than before.
    if text == "invalid configuration" && cfg.is_some_and(|cfg| cfg.password.is_none()) {
        return "no password: this profile carries none and the server asked for one — type it in \
                Password and save, so the keychain keeps it"
            .to_string();
    }
    text
}

fn parse_value(raw: Option<&str>, ty: &Type) -> Value {
    let Some(text) = raw else { return Value::Null };
    match *ty {
        Type::BOOL => Value::Bool(text == "t" || text == "true"),
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => text
            .parse::<i64>()
            .map(Value::Int)
            .unwrap_or_else(|_| Value::text(text)),
        Type::FLOAT4 | Type::FLOAT8 => text
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or_else(|_| Value::text(text)),
        Type::JSON | Type::JSONB => Value::Json(text.to_string()),
        Type::DATE => Value::Date(text.to_string()),
        Type::TIME | Type::TIMETZ => Value::Time(text.to_string()),
        Type::TIMESTAMP | Type::TIMESTAMPTZ => Value::Timestamp(text.to_string()),
        Type::BYTEA => {
            let hex = text.strip_prefix("\\x").unwrap_or(text);
            match decode_hex(hex) {
                Some(b) => Value::Bytes(b),
                None => Value::text(text),
            }
        }
        _ => Value::text(text),
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    // `% 2` rather than the 1.87 `is_multiple_of`: the workspace declares `rust-version = "1.80"`.
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

/// Postgres prints booleans as `true`/`false` through `::text` and `t`/`f` in the wire format.
fn pg_bool(cell: &str) -> bool {
    matches!(
        cell.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "yes" | "1"
    )
}

fn split_list(cell: &str) -> Vec<String> {
    cell.split('|')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Fallback type labels when the statement could not be prepared (DDL, multi-statement scripts).
fn infer_types(rows: &[Vec<Value>], columns: usize) -> Vec<String> {
    (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .find(|v| !v.is_null())
                .map(|v| v.type_name().to_ascii_uppercase())
                .unwrap_or_else(|| "TEXT".into())
        })
        .collect()
}

const EXCLUDE_SCHEMAS: &str =
    "'information_schema','pg_catalog','pg_toast','pg_temp_1','pg_toast_temp_1'";

impl Driver for Postgres {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }

    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn current_database(&self) -> Option<String> {
        Some(self.database.clone())
    }

    fn server_info(&mut self) -> Result<ServerInfo> {
        let version = self
            .one("SELECT version()")
            .ok_or_else(|| Error::connect("could not read server version"))?;
        let short = version.split(" on ").next().unwrap_or(&version).to_string();
        let size = self
            .one("SELECT pg_database_size(current_database())::text")
            .and_then(|s| s.parse::<i64>().ok());
        let uptime = self
            .one("SELECT EXTRACT(EPOCH FROM (now() - pg_postmaster_start_time()))::bigint::text")
            .and_then(|s| s.parse::<i64>().ok());
        Ok(ServerInfo {
            version: short,
            driver: "PostgreSQL".into(),
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
        // Types come from a prepared (but unexecuted) statement; values come from the simple
        // protocol as text, which keeps numeric and timestamps exact.
        let declared = self.declared_columns(sql);
        let messages = self.simple(sql)?;
        let mut rs = ResultSet::default();
        let mut names: Vec<String> = declared.iter().map(|(n, _)| n.clone()).collect();
        let mut truncated = false;
        for m in messages {
            match m {
                SimpleQueryMessage::RowDescription(desc) => {
                    if names.is_empty() {
                        names = desc.iter().map(|c| c.name().to_string()).collect();
                    }
                }
                SimpleQueryMessage::Row(row) => {
                    if rs.rows.len() >= limit {
                        truncated = true;
                        continue;
                    }
                    let mut vals = Vec::with_capacity(names.len().max(row.len()));
                    for i in 0..row.len() {
                        let ty = declared.get(i).map(|(_, t)| t.clone());
                        vals.push(match ty.as_ref() {
                            Some(t) => parse_value(row.get(i), t),
                            None => match row.get(i) {
                                Some(v) => Value::text(v),
                                None => Value::Null,
                            },
                        });
                    }
                    rs.rows.push(vals);
                }
                SimpleQueryMessage::CommandComplete(n) => rs.affected += n,
                _ => {}
            }
        }
        rs.columns = names;
        rs.column_types = if declared.is_empty() {
            infer_types(&rs.rows, rs.columns.len())
        } else {
            declared.iter().map(|(_, t)| t.name().to_string()).collect()
        };
        rs.truncated = truncated;
        rs.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(rs)
    }

    fn catalog(&mut self) -> Result<Catalog> {
        let started = Instant::now();
        let schemas: Vec<String> = self
            .string_rows(&format!(
                "SELECT nspname FROM pg_namespace WHERE nspname NOT IN ({EXCLUDE_SCHEMAS}) \
                 AND nspname NOT LIKE 'pg_temp%' ORDER BY nspname"
            ))?
            .into_iter()
            .filter_map(|r| r.into_iter().next())
            .collect();

        let table_rows = self.string_rows(&format!(
            "SELECT n.nspname, c.relname, c.relkind::text, COALESCE(obj_description(c.oid, 'pg_class'), ''), \
             GREATEST(c.reltuples::bigint, 0)::text, pg_total_relation_size(c.oid)::text \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','p','v','m','f') AND n.nspname NOT IN ({EXCLUDE_SCHEMAS}) \
             AND n.nspname NOT LIKE 'pg_temp%' ORDER BY n.nspname, c.relname"
        ))?;

        let column_rows = self.string_rows(&format!(
            "SELECT c.table_schema, c.table_name, c.column_name, c.ordinal_position::text, \
             COALESCE(c.column_default, ''), c.is_nullable, \
             CASE WHEN c.data_type = 'USER-DEFINED' THEN c.udt_name ELSE c.data_type END, \
             COALESCE(c.character_maximum_length::text, ''), \
             COALESCE(c.numeric_precision::text, ''), COALESCE(c.numeric_scale::text, '') \
             FROM information_schema.columns c \
             WHERE c.table_schema NOT IN ({EXCLUDE_SCHEMAS}) \
             ORDER BY c.table_schema, c.table_name, c.ordinal_position"
        ))?;

        let pk_rows = self.string_rows(&format!(
            "SELECT tc.table_schema, tc.table_name, kcu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema NOT IN ({EXCLUDE_SCHEMAS})"
        ))?;

        let index_rows = self.string_rows(&format!(
            "SELECT n.nspname, t.relname, i.relname, ix.indisunique::text, ix.indisprimary::text, \
             COALESCE(array_to_string(ARRAY(SELECT pg_get_indexdef(ix.indexrelid, k + 1, true) \
               FROM generate_subscripts(ix.indkey, 1) AS k ORDER BY k), '|'), '') \
             FROM pg_index ix JOIN pg_class i ON i.oid = ix.indexrelid \
             JOIN pg_class t ON t.oid = ix.indrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE n.nspname NOT IN ({EXCLUDE_SCHEMAS}) ORDER BY 1,2,3"
        ))?;

        let fk_rows = self.string_rows(&format!(
            "SELECT tc.constraint_name, tc.table_schema, tc.table_name, kcu.column_name, \
             ccu.table_schema, ccu.table_name, ccu.column_name, rc.delete_rule, rc.update_rule \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
             JOIN information_schema.referential_constraints rc \
               ON rc.constraint_name = tc.constraint_name AND rc.constraint_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema NOT IN ({EXCLUDE_SCHEMAS})"
        ))?;

        let mut out = Catalog {
            driver: self
                .server_info()
                .map(|i| i.version)
                .unwrap_or_else(|_| "PostgreSQL".into()),
            database: Some(self.database.clone()),
            databases: self.databases().unwrap_or_default(),
            schemas: schemas
                .iter()
                .map(|name| Schema {
                    name: name.clone(),
                    tables: vec![],
                })
                .collect(),
            fetched_ms: 0,
        };

        for row in &table_rows {
            if row.len() < 6 {
                continue;
            }
            let schema_name = row[0].clone();
            let Some(schema) = out.schemas.iter_mut().find(|s| s.name == schema_name) else {
                continue;
            };
            let kind = match row[2].as_str() {
                "v" => TableKind::View,
                "m" => TableKind::MatView,
                "f" => TableKind::Table,
                _ => TableKind::Table,
            };
            schema.tables.push(Table {
                schema: Some(schema_name.clone()),
                name: row[1].clone(),
                kind,
                comment: row[3].clone(),
                row_estimate: row[4].parse::<i64>().ok(),
                size_bytes: row[5].parse::<i64>().ok(),
                ..Table::default()
            });
        }

        for row in &column_rows {
            if row.len() < 10 {
                continue;
            }
            let Some(table) = out
                .schema(&row[0])
                .and_then(|s| s.tables.iter().find(|t| t.name == row[1]))
                .cloned()
            else {
                continue;
            };
            let col = Column {
                name: row[2].clone(),
                type_name: row[6].clone(),
                nullable: row[5].eq_ignore_ascii_case("YES"),
                default: if row[4].is_empty() {
                    None
                } else {
                    Some(row[4].clone())
                },
                ordinal: row[3].parse().unwrap_or(0),
                max_len: row[7].parse().ok(),
                numeric_precision: row[8].parse().ok(),
                numeric_scale: row[9].parse().ok(),
                ..Column::default()
            };
            if let Some(schema) = out.schemas.iter_mut().find(|s| s.name == row[0]) {
                if let Some(t) = schema.tables.iter_mut().find(|t| t.name == table.name) {
                    t.columns.push(col);
                }
            }
        }

        for row in &pk_rows {
            if row.len() < 3 {
                continue;
            }
            if let Some(schema) = out.schemas.iter_mut().find(|s| s.name == row[0]) {
                if let Some(t) = schema.tables.iter_mut().find(|t| t.name == row[1]) {
                    if let Some(c) = t.columns.iter_mut().find(|c| c.name == row[2]) {
                        c.primary_key = true;
                        c.nullable = false;
                    }
                }
            }
        }

        for row in &index_rows {
            if row.len() < 6 {
                continue;
            }
            let unique = pg_bool(&row[3]);
            let primary = pg_bool(&row[4]);
            if let Some(schema) = out.schemas.iter_mut().find(|s| s.name == row[0]) {
                if let Some(t) = schema.tables.iter_mut().find(|t| t.name == row[1]) {
                    t.indexes.push(Index {
                        name: row[2].clone(),
                        columns: split_list(&row[5]),
                        unique,
                        primary,
                        kind: if primary {
                            "primary key".into()
                        } else if unique {
                            "unique".into()
                        } else {
                            "index".into()
                        },
                    });
                }
            }
        }

        for row in &fk_rows {
            if row.len() < 9 {
                continue;
            }
            if let Some(schema) = out.schemas.iter_mut().find(|s| s.name == row[1]) {
                if let Some(t) = schema.tables.iter_mut().find(|t| t.name == row[2]) {
                    match t.foreign_keys.iter_mut().find(|f| f.name == row[0]) {
                        Some(fk) => {
                            fk.columns.push(row[3].clone());
                            fk.ref_columns.push(row[6].clone());
                        }
                        None => t.foreign_keys.push(ForeignKey {
                            name: row[0].clone(),
                            columns: vec![row[3].clone()],
                            ref_schema: Some(row[4].clone()),
                            ref_table: row[5].clone(),
                            ref_columns: vec![row[6].clone()],
                            on_delete: row[7].clone(),
                            on_update: row[8].clone(),
                        }),
                    }
                }
            }
        }

        out.fetched_ms = started.elapsed().as_millis() as u64;
        Ok(out)
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .string_rows(
                "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
            )?
            .into_iter()
            .filter_map(|r| r.into_iter().next())
            .collect())
    }

    fn use_database(&mut self, database: &str) -> Result<()> {
        if self.in_tx {
            return Err(Error::query(
                "finish or roll back the open transaction before switching database",
            ));
        }
        let mut cfg = self.cfg.clone();
        cfg.database = database.to_string();
        let fresh = connect(&cfg)?;
        // Replacing the client closes the old connection.
        self.client = fresh;
        self.cfg = cfg;
        self.database = database.to_string();
        Ok(())
    }

    fn processes(&mut self) -> Result<Vec<DbProcess>> {
        let rows = self.string_rows(
            "SELECT pid::text, COALESCE(usename, ''), COALESCE(datname, ''), COALESCE(client_addr::text, 'local'), \
             COALESCE(state, ''), COALESCE(query, ''), \
             COALESCE(EXTRACT(MILLISECONDS FROM (now() - query_start))::bigint::text, ''), \
             COALESCE(query_start::text, '') \
             FROM pg_stat_activity WHERE pid <> pg_backend_pid() ORDER BY query_start NULLS LAST LIMIT 1000",
        )?;
        Ok(rows
            .iter()
            .filter(|r| r.len() >= 8)
            .map(|r| DbProcess {
                id: r[0].clone(),
                user: r[1].clone(),
                database: r[2].clone(),
                host: r[3].clone(),
                state: r[4].clone(),
                query: r[5].clone(),
                elapsed_ms: r[6].parse().ok(),
                started: if r[7].is_empty() {
                    None
                } else {
                    Some(r[7].clone())
                },
            })
            .collect())
    }

    fn kill_process(&mut self, id: &str) -> Result<()> {
        let pid: i32 = id
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("'{id}' is not a process id")))?;
        let rs = self.run(&format!("SELECT pg_terminate_backend({pid})"), 1)?;
        let ok = rs
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if ok {
            Ok(())
        } else {
            Err(Error::query(format!("session {pid} was already gone")))
        }
    }

    fn variables(&mut self) -> Result<Vec<(String, String)>> {
        Ok(self
            .string_rows("SELECT name, setting FROM pg_settings ORDER BY name LIMIT 2000")?
            .into_iter()
            .filter(|r| r.len() >= 2)
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect())
    }

    fn status(&mut self) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for (label, sql) in [
            ("version", "SELECT version()"),
            ("started", "SELECT pg_postmaster_start_time()::text"),
            (
                "database size",
                "SELECT pg_size_pretty(pg_database_size(current_database()))",
            ),
            ("sessions", "SELECT count(*)::text FROM pg_stat_activity"),
            (
                "max connections",
                "SELECT current_setting('max_connections')",
            ),
            (
                "commits",
                "SELECT sum(xact_commit)::text FROM pg_stat_database",
            ),
            (
                "rollbacks",
                "SELECT sum(xact_rollback)::text FROM pg_stat_database",
            ),
            (
                "cache hit ratio",
                "SELECT CASE WHEN sum(blks_hit) + sum(blks_read) = 0 THEN 'n/a' \
                 ELSE round(100.0 * sum(blks_hit) / (sum(blks_hit) + sum(blks_read)), 2)::text END \
                 FROM pg_stat_database",
            ),
        ] {
            if let Some(v) = self.one(sql) {
                out.push((label.to_string(), v));
            }
        }
        Ok(out)
    }

    fn users(&mut self) -> Result<Vec<DbUser>> {
        Ok(self
            .string_rows(
                "SELECT rolname, rolcanlogin::text, rolsuper::text, COALESCE(rolvaliduntil::text, '') \
                 FROM pg_roles ORDER BY rolname",
            )?
            .into_iter()
            .filter(|r| r.len() >= 4)
            .map(|r| DbUser {
                name: r[0].clone(),
                host: String::new(),
                can_login: pg_bool(&r[1]),
                is_super: pg_bool(&r[2]),
                expires: if r[3].is_empty() { None } else { Some(r[3].clone()) },
            })
            .collect())
    }

    fn grants(&mut self, user: &DbUser) -> Result<Vec<String>> {
        let who = user.name.replace('\'', "''");
        let mut out: Vec<String> = self
            .string_rows(&format!(
                "SELECT privilege_type || ' on ' || table_name \
                 FROM information_schema.role_table_grants WHERE grantee = '{who}' LIMIT 500"
            ))?
            .into_iter()
            .filter_map(|r| r.into_iter().next())
            .collect();
        out.extend(
            self.string_rows(&format!(
                "SELECT 'member of ' || r.rolname FROM pg_auth_members m \
                 JOIN pg_roles r ON r.oid = m.roleid JOIN pg_roles u ON u.oid = m.member \
                 WHERE u.rolname = '{who}'"
            ))?
            .into_iter()
            .filter_map(|r| r.into_iter().next()),
        );
        if user.is_super {
            out.insert(0, "SUPERUSER".into());
        }
        Ok(out)
    }

    fn table_ddl(&mut self, schema: Option<&str>, table: &str) -> Result<String> {
        // Generated from the catalog: Postgres has no SHOW CREATE TABLE.
        let catalog = self.catalog()?;
        let t = catalog
            .find_table(schema, table)
            .ok_or_else(|| Error::not_found(format!("no such table: {table}")))?
            .clone();
        let mut out = nook_model::ddl::create_table(&t, Dialect::Postgres);
        out.push('\n');
        for idx in nook_model::ddl::create_indexes(&t, Dialect::Postgres) {
            out.push_str(&idx);
            out.push('\n');
        }
        for fk in nook_model::ddl::create_foreign_keys(&t, Dialect::Postgres) {
            out.push_str(&fk);
            out.push('\n');
        }
        Ok(out.trim_end().to_string())
    }

    fn begin(&mut self) -> Result<()> {
        self.simple("BEGIN")?;
        self.in_tx = true;
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        self.simple("COMMIT")?;
        self.in_tx = false;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        self.simple("ROLLBACK")?;
        self.in_tx = false;
        Ok(())
    }

    fn in_transaction(&self) -> bool {
        self.in_tx
    }

    fn close(&mut self) {
        // Dropping the client closes the connection; nothing to do explicitly.
    }
}

/// Row estimate for the tree, cheap enough to run for every table.
pub fn fetch_estimates(db: &mut Postgres) -> Result<Vec<(String, i64)>> {
    Ok(db
        .string_rows(&format!(
            "SELECT n.nspname || '.' || c.relname, GREATEST(c.reltuples::bigint, 0)::text \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind = 'r' AND n.nspname NOT IN ({EXCLUDE_SCHEMAS})"
        ))?
        .into_iter()
        .filter(|r| r.len() >= 2)
        .filter_map(|r| r[1].parse::<i64>().ok().map(|n| (r[0].clone(), n)))
        .collect())
}
