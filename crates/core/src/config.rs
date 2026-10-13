//! Connection configuration: what a profile holds, and how it becomes a URI.

use serde::{Deserialize, Serialize};

/// SQL dialect a driver speaks; also picks quoting and literal rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Sqlite,
    Postgres,
    MySql,
}

/// A database family. The last four are catalogued enterprise types DataCraft can list but not yet open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverKind {
    Sqlite,
    Postgres,
    MySql,
    MariaDb,
    SqlServer,
    Oracle,
    Mongo,
    Redis,
}

impl DriverKind {
    pub fn dialect(self) -> Option<Dialect> {
        match self {
            DriverKind::Sqlite => Some(Dialect::Sqlite),
            DriverKind::Postgres => Some(Dialect::Postgres),
            DriverKind::MySql | DriverKind::MariaDb => Some(Dialect::MySql),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Sqlite => "SQLite",
            DriverKind::Postgres => "PostgreSQL",
            DriverKind::MySql => "MySQL",
            DriverKind::MariaDb => "MariaDB",
            DriverKind::SqlServer => "SQL Server",
            DriverKind::Oracle => "Oracle",
            DriverKind::Mongo => "MongoDB",
            DriverKind::Redis => "Redis",
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            DriverKind::Postgres => 5432,
            DriverKind::MySql | DriverKind::MariaDb => 3306,
            DriverKind::SqlServer => 1433,
            DriverKind::Oracle => 1521,
            DriverKind::Mongo => 27017,
            DriverKind::Redis => 6379,
            DriverKind::Sqlite => 0,
        }
    }
    pub fn is_file_based(self) -> bool {
        matches!(self, DriverKind::Sqlite)
    }
    pub fn implemented(self) -> bool {
        matches!(
            self,
            DriverKind::Sqlite | DriverKind::Postgres | DriverKind::MySql | DriverKind::MariaDb
        )
    }
    pub fn parse(s: &str) -> Option<DriverKind> {
        match s.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
            "sqlite" | "sqlite3" => Some(DriverKind::Sqlite),
            "postgres" | "postgresql" | "pg" => Some(DriverKind::Postgres),
            "mysql" => Some(DriverKind::MySql),
            "mariadb" => Some(DriverKind::MariaDb),
            "sqlserver" | "mssql" => Some(DriverKind::SqlServer),
            "oracle" => Some(DriverKind::Oracle),
            "mongo" | "mongodb" => Some(DriverKind::Mongo),
            "redis" => Some(DriverKind::Redis),
            _ => None,
        }
    }
    /// Schemes accepted in a connection string.
    pub fn from_scheme(scheme: &str) -> Option<DriverKind> {
        match scheme.to_ascii_lowercase().as_str() {
            "sqlite" | "sqlite3" | "file" => Some(DriverKind::Sqlite),
            "postgres" | "postgresql" => Some(DriverKind::Postgres),
            "mysql" => Some(DriverKind::MySql),
            "mariadb" => Some(DriverKind::MariaDb),
            "mssql" | "sqlserver" => Some(DriverKind::SqlServer),
            "oracle" => Some(DriverKind::Oracle),
            "mongodb" => Some(DriverKind::Mongo),
            "redis" => Some(DriverKind::Redis),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub fn parse(s: &str) -> Option<SslMode> {
        match s.to_ascii_lowercase().as_str() {
            "disable" | "off" | "false" => Some(SslMode::Disable),
            "prefer" => Some(SslMode::Prefer),
            "require" | "on" | "true" => Some(SslMode::Require),
            "verify-ca" | "verifyca" => Some(SslMode::VerifyCa),
            "verify-full" | "verifyfull" => Some(SslMode::VerifyFull),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        }
    }
    /// Postgres `sslmode=` value.
    pub fn pg_value(self) -> &'static str {
        self.label()
    }
}

/// Navicat-style SSH tunnel: we shell out to the system `ssh`, so keys and the agent just work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SshTunnel {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Path to a private key; empty means ssh's own defaults (agent, `~/.ssh/config`).
    pub key_file: String,
    /// Local port to bind; 0 picks a free one.
    pub local_port: u16,
}

impl SshTunnel {
    /// `ssh -N -L …` argument vector, minus the executable. `prompt` drops `BatchMode=yes` so ssh
    /// may ask for a password — the password itself never travels through these arguments.
    pub fn ssh_args(
        &self,
        target_host: &str,
        target_port: u16,
        local_port: u16,
        prompt: bool,
    ) -> Vec<String> {
        let mut args = vec![
            "-N".to_string(),
            "-o".into(),
            "ExitOnForwardFailure=yes".into(),
        ];
        if !prompt {
            args.push("-o".into());
            args.push("BatchMode=yes".into());
        }
        args.push("-o".into());
        args.push("StrictHostKeyChecking=accept-new".into());
        args.push("-p".into());
        args.push(self.port.to_string());
        if !self.key_file.is_empty() {
            args.push("-i".into());
            args.push(self.key_file.clone());
        }
        args.push("-L".into());
        args.push(format!("{local_port}:{target_host}:{target_port}"));
        args.push(format!("{}@{}", self.user, self.host));
        args
    }
}

/// One saved connection profile, plus everything needed to open it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DriverKind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    /// Kept out of the profile file when a keychain reference is used instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub database: String,
    /// SQLite only.
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub ssh: SshTunnel,
    #[serde(default)]
    pub ssl: SslMode,
    #[serde(default)]
    pub read_only: bool,
    /// Palette colour used by the tree and the tab strip.
    #[serde(default)]
    pub color: String,
    #[serde(default = "default_timeout")]
    pub connect_timeout_s: u64,
    /// Group folder in the connection list, Navicat-style.
    #[serde(default)]
    pub group: String,
}

fn default_timeout() -> u64 {
    15
}

impl ConnectionConfig {
    pub fn new(name: impl Into<String>, kind: DriverKind) -> Self {
        let name = name.into();
        ConnectionConfig {
            id: name.to_ascii_lowercase().replace(' ', "-"),
            name,
            kind,
            host: if kind.is_file_based() {
                String::new()
            } else {
                "127.0.0.1".into()
            },
            port: kind.default_port(),
            user: String::new(),
            password: None,
            database: String::new(),
            file: String::new(),
            ssh: SshTunnel::default(),
            ssl: SslMode::default(),
            read_only: false,
            color: String::new(),
            connect_timeout_s: default_timeout(),
            group: String::new(),
        }
    }

    pub fn sqlite(path: impl Into<String>) -> Self {
        let file = path.into();
        let mut cfg = ConnectionConfig::new(
            std::path::Path::new(&file)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.clone()),
            DriverKind::Sqlite,
        );
        cfg.file = file;
        cfg
    }

    /// Effective host/port after an SSH tunnel rewrites the target.
    pub fn target(&self) -> (String, u16) {
        if self.ssh.enabled {
            ("127.0.0.1".into(), self.ssh.local_port)
        } else {
            (self.host.clone(), self.port)
        }
    }

    /// Connection URI for the driver, with the password in place.
    pub fn uri(&self) -> String {
        match self.kind {
            DriverKind::Sqlite => self.file.clone(),
            _ => {
                let (host, port) = self.target();
                format!(
                    "{}://{}:{}@{}:{}/{}",
                    if self.kind == DriverKind::Postgres {
                        "postgres"
                    } else {
                        "mysql"
                    },
                    url_encode(&self.user),
                    url_encode(self.password.as_deref().unwrap_or("")),
                    host,
                    port,
                    url_encode(&self.database),
                )
            }
        }
    }

    /// Same as [`ConnectionConfig::uri`] with the password replaced, safe to log or display.
    pub fn display_uri(&self) -> String {
        if self.kind == DriverKind::Sqlite {
            return format!("sqlite:{}", self.file);
        }
        self.uri().replace(
            &format!(":{}@", url_encode(self.password.as_deref().unwrap_or(""))),
            ":***@",
        )
    }

    /// Parse `sqlite:./x.db`, `postgres://user:pass@host:5432/db`, `mysql://…` or a bare `.db` path.
    pub fn parse(raw: &str) -> Result<ConnectionConfig, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("empty connection string".into());
        }
        let (scheme, rest) = match trimmed.split_once("://") {
            Some((s, r)) => (s.to_ascii_lowercase(), r.to_string()),
            None => match trimmed.split_once(':') {
                Some((s, r)) if DriverKind::from_scheme(s).is_some() && !r.starts_with("//") => {
                    (s.to_ascii_lowercase(), r.to_string())
                }
                _ => ("sqlite".to_string(), trimmed.to_string()),
            },
        };
        let kind =
            DriverKind::from_scheme(&scheme).ok_or_else(|| format!("unknown scheme '{scheme}'"))?;
        if !kind.implemented() {
            return Err(format!(
                "{} is catalogued but not implemented yet",
                kind.label()
            ));
        }
        if kind.is_file_based() {
            let file = rest.trim_start_matches("//").to_string();
            if file.is_empty() {
                return Err("sqlite needs a file path".into());
            }
            let mut cfg = ConnectionConfig::sqlite(file);
            cfg.id = cfg.file.clone();
            return Ok(cfg);
        }
        let (creds, hostpart) = match rest.rsplit_once('@') {
            Some((c, h)) => (Some(c), h),
            None => (None, rest.as_str()),
        };
        let (user, password) = match creds {
            Some(c) => match c.split_once(':') {
                Some((u, p)) => (url_decode(u), Some(url_decode(p))),
                None => (url_decode(c), None),
            },
            None => (String::new(), None),
        };
        let (hostport, database) = match hostpart.split_once('/') {
            Some((h, d)) => (h, url_decode(d)),
            None => (hostpart, String::new()),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse::<u16>().map_err(|_| format!("bad port '{p}'"))?,
            ),
            None => (hostport.to_string(), kind.default_port()),
        };
        let mut cfg = ConnectionConfig::new(
            format!(
                "{}@{}",
                if database.is_empty() { "?" } else { &database },
                host
            ),
            kind,
        );
        cfg.id = trimmed.to_string();
        cfg.host = host;
        cfg.port = port;
        cfg.user = user;
        cfg.password = password;
        cfg.database = database;
        Ok(cfg)
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_postgres_url() {
        let c = ConnectionConfig::parse("postgres://alice:s3cret@db.internal:5433/shop").ok();
        let c = c.expect("parsed");
        assert_eq!(c.kind, DriverKind::Postgres);
        assert_eq!(c.user, "alice");
        assert_eq!(c.password.as_deref(), Some("s3cret"));
        assert_eq!(c.host, "db.internal");
        assert_eq!(c.port, 5433);
        assert_eq!(c.database, "shop");
        assert!(c.display_uri().contains(":***@"));
    }

    #[test]
    fn parses_sqlite_paths_and_mysql_scheme() {
        let a = ConnectionConfig::parse("sqlite:./app.db").expect("sqlite");
        assert_eq!(a.kind, DriverKind::Sqlite);
        assert_eq!(a.file, "./app.db");
        let b = ConnectionConfig::parse("mysql://root@localhost").expect("mysql");
        assert_eq!(b.port, 3306);
        let c = ConnectionConfig::parse("/tmp/data.sqlite3").expect("bare path");
        assert_eq!(c.kind, DriverKind::Sqlite);
        assert!(ConnectionConfig::parse("mongo://x").is_err());
        assert!(ConnectionConfig::parse("").is_err());
    }

    #[test]
    fn ssh_args_are_well_formed() {
        let t = SshTunnel {
            enabled: true,
            host: "jump.example.com".into(),
            port: 2222,
            user: "ops".into(),
            key_file: "~/.ssh/id_ed25519".into(),
            local_port: 15432,
        };
        let args = t.ssh_args("10.0.0.5", 5432, 15432, false);
        assert!(args.contains(&"-L".to_string()));
        assert!(args.contains(&"15432:10.0.0.5:5432".to_string()));
        assert_eq!(
            args.last().map(String::as_str),
            Some("ops@jump.example.com")
        );
        assert!(args.contains(&"BatchMode=yes".into()));
    }

    #[test]
    fn only_a_prompting_tunnel_lets_ssh_ask_for_a_password() {
        let t = SshTunnel {
            enabled: true,
            host: "jump.example.com".into(),
            port: 2222,
            user: "ops".into(),
            key_file: String::new(),
            local_port: 15432,
        };
        let quiet = t.ssh_args("10.0.0.5", 5432, 15432, false);
        let asking = t.ssh_args("10.0.0.5", 5432, 15432, true);
        assert!(quiet.contains(&"BatchMode=yes".into()));
        assert!(!asking.contains(&"BatchMode=yes".into()));
        // Asking is exactly the quiet vector minus the `-o BatchMode=yes` pair.
        let mut without_batch = quiet.clone();
        let at = without_batch
            .iter()
            .position(|a| a == "BatchMode=yes")
            .expect("BatchMode");
        without_batch.drain(at - 1..=at);
        assert_eq!(without_batch, asking);
        // `-i` is the key file's, not the prompt flag's.
        assert!(!asking.contains(&"-i".into()));
        let mut keyed = t.clone();
        keyed.key_file = "~/.ssh/id_ed25519".into();
        assert!(keyed
            .ssh_args("10.0.0.5", 5432, 15432, true)
            .contains(&"-i".into()));
    }

    #[test]
    fn url_encoding_round_trips() {
        let s = "p@ss:w/rd";
        assert_ne!(url_encode(s), s);
        assert_eq!(url_decode(&url_encode(s)), s);
    }

    #[test]
    fn kinds_and_dialects_agree() {
        assert_eq!(DriverKind::MariaDb.dialect(), Some(Dialect::MySql));
        assert!(DriverKind::Redis.dialect().is_none());
        assert_eq!(DriverKind::parse("mssql"), Some(DriverKind::SqlServer));
        assert_eq!(SslMode::parse("verify-full"), Some(SslMode::VerifyFull));
    }
}
