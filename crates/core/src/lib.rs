#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L0: errors, cells, identifiers and quoting for every SQL dialect.

pub mod config;
pub mod value;

pub use config::{ConnectionConfig, Dialect, DriverKind, SshTunnel, SslMode};
pub use value::Value;

use std::fmt;

/// House error type. Every fallible path in DataCraft returns this; nothing panics.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Filesystem or process failure.
    Io(String),
    /// Bad configuration, missing profile, unparsable connection string.
    Config(String),
    /// The feature exists in the catalog but not for this driver/pair.
    Unsupported(String),
    /// Could not reach or authenticate against the server.
    Connect(String),
    /// The server rejected a statement or a statement failed at runtime.
    Query(String),
    /// Asked for something that is not there.
    NotFound(String),
    /// The user cancelled.
    Canceled,
    /// A bug or a hostile input we refused.
    Internal(String),
}

impl Error {
    pub fn internal(m: impl Into<String>) -> Self {
        Error::Internal(m.into())
    }
    pub fn config(m: impl Into<String>) -> Self {
        Error::Config(m.into())
    }
    pub fn query(m: impl Into<String>) -> Self {
        Error::Query(m.into())
    }
    pub fn unsupported(m: impl Into<String>) -> Self {
        Error::Unsupported(m.into())
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Error::NotFound(m.into())
    }
    pub fn connect(m: impl Into<String>) -> Self {
        Error::Connect(m.into())
    }
    pub fn io(m: impl Into<String>) -> Self {
        Error::Io(m.into())
    }
    /// Short tag used by the UI and the CLI to colour and route the message.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Io(_) => "io",
            Error::Config(_) => "config",
            Error::Unsupported(_) => "unsupported",
            Error::Connect(_) => "connect",
            Error::Query(_) => "query",
            Error::NotFound(_) => "not_found",
            Error::Canceled => "canceled",
            Error::Internal(_) => "internal",
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Error::Io(m)
            | Error::Config(m)
            | Error::Unsupported(m)
            | Error::Connect(m)
            | Error::Query(m)
            | Error::NotFound(m)
            | Error::Internal(m) => m,
            Error::Canceled => "cancelled",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind(), self.message())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Config(format!("json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Quote an identifier the way `dialect` wants it.
pub fn quote_ident(name: &str, dialect: Dialect) -> String {
    let (open, close) = match dialect {
        Dialect::MySql => ('`', '`'),
        _ => ('"', '"'),
    };
    let mut out = String::with_capacity(name.len() + 2);
    out.push(open);
    for ch in name.chars() {
        if ch == close {
            out.push(close);
        }
        out.push(ch);
    }
    out.push(close);
    out
}

/// `schema.table` with both parts quoted where the dialect takes a schema.
pub fn quote_qualified(schema: Option<&str>, table: &str, dialect: Dialect) -> String {
    match schema {
        Some(s) if !s.is_empty() && dialect != Dialect::Sqlite => {
            format!(
                "{}.{}",
                quote_ident(s, dialect),
                quote_ident(table, dialect)
            )
        }
        _ => quote_ident(table, dialect),
    }
}

/// SQL literal for a cell. One code path feeds the grid, transfer, sync, dumps and data generation.
pub fn sql_literal(v: &Value, dialect: Dialect) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => {
            if f.is_finite() {
                format!("{f}")
            } else {
                "NULL".into()
            }
        }
        Value::Bool(b) => match dialect {
            Dialect::Sqlite | Dialect::Postgres => {
                if *b {
                    "TRUE".into()
                } else {
                    "FALSE".into()
                }
            }
            Dialect::MySql => {
                if *b {
                    "1".into()
                } else {
                    "0".into()
                }
            }
        },
        Value::Text(s) | Value::Json(s) | Value::Date(s) | Value::Time(s) | Value::Timestamp(s) => {
            format!("'{}'", s.replace('\'', "''"))
        }
        Value::Bytes(b) => {
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            match dialect {
                Dialect::Postgres => format!("'\\x{hex}'"),
                _ => format!("X'{hex}'"),
            }
        }
    }
}

/// True when a statement is expected to hand rows back, used to pick a query path.
pub fn returns_rows(sql: &str) -> bool {
    let head = sql.trim_start();
    let head = head
        .split(|c: char| c.is_whitespace() || c == '(')
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        head.as_str(),
        "select"
            | "show"
            | "describe"
            | "desc"
            | "explain"
            | "with"
            | "values"
            | "table"
            | "pragma"
    ) || sql.to_ascii_lowercase().contains("returning")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_per_dialect() {
        assert_eq!(quote_ident("user", Dialect::MySql), "`user`");
        assert_eq!(quote_ident("user", Dialect::Postgres), "\"user\"");
        assert_eq!(quote_ident("we\"ird", Dialect::Postgres), "\"we\"\"ird\"");
        assert_eq!(quote_ident("we`ird", Dialect::MySql), "`we``ird`");
        assert_eq!(
            quote_qualified(Some("public"), "t", Dialect::Postgres),
            "\"public\".\"t\""
        );
    }

    #[test]
    fn literals_escape() {
        assert_eq!(
            sql_literal(&Value::Text("o'x".into()), Dialect::Postgres),
            "'o''x'"
        );
        assert_eq!(sql_literal(&Value::Null, Dialect::Sqlite), "NULL");
        assert_eq!(
            sql_literal(&Value::Bytes(vec![0x00, 0xff]), Dialect::MySql),
            "X'00ff'"
        );
        assert_eq!(
            sql_literal(&Value::Bytes(vec![0x00, 0xff]), Dialect::Postgres),
            "'\\x00ff'"
        );
    }

    #[test]
    fn row_detection() {
        assert!(returns_rows("  select 1"));
        assert!(returns_rows("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(returns_rows("insert into t values (1) returning id"));
        assert!(!returns_rows("create table t (id int)"));
        assert!(!returns_rows("update t set a = 1"));
    }
}
