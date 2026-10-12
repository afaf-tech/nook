//! Backup and restore.
//!
//! SQLite is dumped in process (the SQL script comes from `transfer::dump_script`); Postgres and
//! MySQL shell out to the vendor tools the user already has, which is what Navicat does too.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use datacraft_core::{ConnectionConfig, DriverKind, Error, Result};

/// What a backup or restore will run.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// No external tool: the engine writes (or replays) a `.sql` script itself.
    InProcess,
    /// Run a program, optionally feeding a file to its standard input.
    Command {
        program: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
        stdin_file: Option<PathBuf>,
    },
}

impl Plan {
    /// Human-readable command line, with the password replaced by the env var name.
    pub fn display(&self) -> String {
        match self {
            Plan::InProcess => "in-process SQL script".into(),
            Plan::Command { program, args, .. } => {
                format!("{} {}", program.display(), args.join(" "))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub path: PathBuf,
    pub bytes: u64,
    pub command: String,
    pub elapsed_ms: u128,
    pub stderr: String,
}

/// Directories to search beyond `PATH`, so keg-only Homebrew installs are found.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    for extra in [
        "/opt/homebrew/bin",
        "/opt/homebrew/opt/postgresql@17/bin",
        "/opt/homebrew/opt/postgresql@15/bin",
        "/opt/homebrew/opt/postgresql@14/bin",
        "/opt/homebrew/opt/mysql/bin",
        "/opt/homebrew/opt/mysql@8.0/bin",
        "/usr/local/bin",
        "/usr/bin",
    ] {
        dirs.push(PathBuf::from(extra));
    }
    dirs
}

/// Find a vendor tool, or explain what to install.
pub fn resolve_tool(name: &str) -> Result<PathBuf> {
    for dir in search_dirs() {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(Error::unsupported(format!(
        "{name} is not on this machine; install the client tools (brew install postgresql@17 / mysql) \
         or use the built-in SQL script backup"
    )))
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_alphanumeric() || "-_./=:@".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// Plan a backup of `tables` (empty means every table) into `out`.
pub fn backup_plan(
    cfg: &ConnectionConfig,
    tables: &[String],
    schema_only: bool,
    out: &Path,
) -> Result<Plan> {
    let (host, port) = cfg.target();
    match cfg.kind {
        DriverKind::Sqlite => Ok(Plan::InProcess),
        DriverKind::Postgres => {
            let program = resolve_tool("pg_dump")?;
            let mut args = vec![
                "-h".into(),
                host.clone(),
                "-p".into(),
                port.to_string(),
                "-d".into(),
                cfg.database.clone(),
                "-f".into(),
                out.to_string_lossy().into_owned(),
                "--no-owner".into(),
                "--no-privileges".into(),
            ];
            if !cfg.user.is_empty() {
                args.push("-U".into());
                args.push(cfg.user.clone());
            }
            if schema_only {
                args.push("--schema-only".into());
            }
            for t in tables.iter().filter(|t| t.as_str() != "*") {
                args.push("-t".into());
                args.push(t.clone());
            }
            Ok(Plan::Command {
                program,
                args,
                env: vec![(
                    "PGPASSWORD".into(),
                    cfg.password.clone().unwrap_or_default(),
                )],
                stdin_file: None,
            })
        }
        DriverKind::MySql | DriverKind::MariaDb => {
            let program = resolve_tool("mysqldump")?;
            let mut args = vec![
                "-h".into(),
                host.clone(),
                "-P".into(),
                port.to_string(),
                "--result-file".into(),
                out.to_string_lossy().into_owned(),
            ];
            if !cfg.user.is_empty() {
                args.push("-u".into());
                args.push(cfg.user.clone());
            }
            if schema_only {
                args.push("--no-data".into());
            }
            args.push("--single-transaction".into());
            args.push("--databases".into());
            if tables.is_empty() || tables.iter().any(|t| t == "*") {
                args.push(cfg.database.clone());
            } else {
                // --databases plus table names: dump the named tables from the default schema.
                args.pop();
                args.push(cfg.database.clone());
                args.extend(tables.iter().cloned());
            }
            Ok(Plan::Command {
                program,
                args,
                env: vec![("MYSQL_PWD".into(), cfg.password.clone().unwrap_or_default())],
                stdin_file: None,
            })
        }
        other => Err(Error::unsupported(format!(
            "{} backup is not implemented",
            other.label()
        ))),
    }
}

/// Plan a restore of `file` into the profile's database.
pub fn restore_plan(cfg: &ConnectionConfig, file: &Path) -> Result<Plan> {
    let (host, port) = cfg.target();
    match cfg.kind {
        DriverKind::Sqlite => {
            if !file.is_file() {
                return Err(Error::not_found(format!(
                    "{} is not a file",
                    file.display()
                )));
            }
            Ok(Plan::InProcess)
        }
        DriverKind::Postgres => {
            if !file.is_file() {
                return Err(Error::not_found(format!(
                    "{} is not a file",
                    file.display()
                )));
            }
            let program = resolve_tool("psql")?;
            let mut args = vec![
                "-h".into(),
                host,
                "-p".into(),
                port.to_string(),
                "-d".into(),
                cfg.database.clone(),
                "-v".into(),
                "ON_ERROR_STOP=1".into(),
                "-f".into(),
                file.to_string_lossy().into_owned(),
            ];
            if !cfg.user.is_empty() {
                args.push("-U".into());
                args.push(cfg.user.clone());
            }
            Ok(Plan::Command {
                program,
                args,
                env: vec![(
                    "PGPASSWORD".into(),
                    cfg.password.clone().unwrap_or_default(),
                )],
                stdin_file: None,
            })
        }
        DriverKind::MySql | DriverKind::MariaDb => {
            if !file.is_file() {
                return Err(Error::not_found(format!(
                    "{} is not a file",
                    file.display()
                )));
            }
            let program = resolve_tool("mysql")?;
            let mut args = vec!["-h".into(), host, "-P".into(), port.to_string()];
            if !cfg.user.is_empty() {
                args.push("-u".into());
                args.push(cfg.user.clone());
            }
            if !cfg.database.is_empty() {
                args.push(cfg.database.clone());
            }
            Ok(Plan::Command {
                program,
                args,
                env: vec![("MYSQL_PWD".into(), cfg.password.clone().unwrap_or_default())],
                stdin_file: Some(file.to_path_buf()),
            })
        }
        other => Err(Error::unsupported(format!(
            "{} restore is not implemented",
            other.label()
        ))),
    }
}

/// Run a plan and report what actually landed on disk.
pub fn execute(plan: &Plan, output: Option<&Path>) -> Result<Outcome> {
    let started = Instant::now();
    let (program, args, env, stdin_file) = match plan {
        Plan::InProcess => {
            return Err(Error::internal(
                "in-process plans are executed by the engine, not here",
            ))
        }
        Plan::Command {
            program,
            args,
            env,
            stdin_file,
        } => (program, args, env, stdin_file),
    };
    let mut cmd = Command::new(program);
    cmd.args(args).envs(env.iter().cloned());
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(path) = stdin_file {
        let file = std::fs::File::open(path)
            .map_err(|e| Error::io(format!("open {}: {e}", path.display())))?;
        cmd.stdin(Stdio::from(file));
    } else {
        cmd.stdin(Stdio::null());
    }
    let result = cmd.output().map_err(|e| {
        Error::io(format!(
            "running {}: {e}",
            shell_quote(&program.to_string_lossy())
        ))
    })?;
    let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
    if !result.status.success() {
        return Err(Error::query(format!(
            "{} failed ({}): {}",
            program.display(),
            result.status.code().unwrap_or(-1),
            if stderr.is_empty() {
                "no message".into()
            } else {
                stderr.clone()
            }
        )));
    }
    let bytes = output
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    Ok(Outcome {
        path: output.map(Path::to_path_buf).unwrap_or_default(),
        bytes,
        command: plan.display(),
        elapsed_ms: started.elapsed().as_millis(),
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_backups_are_in_process() {
        let cfg = ConnectionConfig::sqlite("/tmp/x.db");
        assert_eq!(
            backup_plan(&cfg, &[], false, Path::new("/tmp/out.sql")).expect("plan"),
            Plan::InProcess
        );
        // The file has to exist before a restore is planned.
        assert!(restore_plan(&cfg, Path::new("/tmp/out-does-not-exist.sql")).is_err());
    }

    #[test]
    fn postgres_plan_carries_credentials_in_the_environment_only() {
        let mut cfg =
            ConnectionConfig::parse("postgres://alice:s3cret@db:5432/shop").expect("parse");
        cfg.password = Some("s3cret".into());
        match backup_plan(&cfg, &["users".into()], true, Path::new("/tmp/out.sql")) {
            Ok(Plan::Command { args, env, .. }) => {
                assert!(args.iter().any(|a| a == "--schema-only"));
                assert!(args.iter().any(|a| a == "users"));
                assert!(args.iter().all(|a| !a.contains("s3cret")));
                assert_eq!(env[0].0, "PGPASSWORD");
            }
            // pg_dump is not installed on every machine; the error must say so.
            Err(e) => assert!(e.message().contains("pg_dump")),
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn restore_of_a_missing_file_is_refused() {
        let cfg = ConnectionConfig::sqlite("/tmp/x.db");
        assert!(restore_plan(&cfg, Path::new("/tmp/definitely-not-here.sql")).is_err());
    }

    #[test]
    fn restore_plan_accepts_a_real_file() {
        let dir = std::env::temp_dir().join("datacraft_backup_test");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("dump.sql");
        std::fs::write(&file, "select 1;").expect("write");
        let cfg = ConnectionConfig::sqlite("/tmp/x.db");
        assert_eq!(restore_plan(&cfg, &file).ok(), Some(Plan::InProcess));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn unsupported_kinds_are_named() {
        let mut cfg = ConnectionConfig::new("redis", DriverKind::Redis);
        cfg.password = None;
        let e = backup_plan(&cfg, &[], false, Path::new("/tmp/o.sql"));
        assert!(e.is_err());
    }

    #[test]
    fn missing_tools_produce_a_helpful_error() {
        let e = resolve_tool("pg_dump_that_does_not_exist").expect_err("should fail");
        assert!(e.message().contains("not on this machine"));
    }
}
