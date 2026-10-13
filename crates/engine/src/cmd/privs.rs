//! Privileges: users, roles and grants. Postgres and MySQL only — SQLite has no users.

use nook_core::{quote_ident, quote_qualified, Dialect, DriverKind, Error, Result};
use serde_json::json;

use crate::{CommandSpec, Level, Params, Session};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("priv.users", "Users and roles", "Privileges", users)
            .params(r#"{"conn":"local"}"#),
        CommandSpec::new("priv.grants", "Show grants", "Privileges", grants)
            .params(r#"{"user":"app","host":"%"}"#),
        CommandSpec::new("priv.createUser", "Create user", "Privileges", create_user)
            .params(r#"{"user":"app","password":"…","can_login":true}"#)
            .help("Postgres: CREATE ROLE. MySQL: CREATE USER 'u'@'host'. Password is not echoed back."),
        CommandSpec::new("priv.dropUser", "Drop user", "Privileges", drop_user)
            .params(r#"{"user":"app","confirm":true}"#),
        CommandSpec::new("priv.setPassword", "Set password", "Privileges", set_password)
            .params(r#"{"user":"app","password":"…"}"#),
        CommandSpec::new("priv.grant", "Grant", "Privileges", grant)
            .params(r#"{"user":"app","privileges":"SELECT, INSERT","table":"users"}"#)
            .help("Without \"table\" the grant applies to the whole database."),
        CommandSpec::new("priv.revoke", "Revoke", "Privileges", revoke)
            .params(r#"{"user":"app","privileges":"INSERT","table":"users"}"#),
    ]
}

fn require_user_server(session: &Session, index: usize) -> Result<DriverKind> {
    let kind = session
        .conns
        .get(index)
        .map(|c| c.cfg.kind)
        .ok_or_else(|| Error::not_found("no such connection"))?;
    match kind {
        DriverKind::Postgres | DriverKind::MySql | DriverKind::MariaDb => Ok(kind),
        other => Err(Error::unsupported(format!(
            "{} has no user accounts; it is a file database",
            other.label()
        ))),
    }
}

fn kind_and_dialect(session: &Session, index: usize) -> Result<(DriverKind, Dialect)> {
    let kind = require_user_server(session, index)?;
    let dialect = kind
        .dialect()
        .ok_or_else(|| Error::internal("kind without a dialect"))?;
    Ok((kind, dialect))
}

fn users(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let rows = {
        let driver = session.driver_for(index)?;
        driver.users()?
    };
    Ok(json!({
        "count": rows.len(),
        "users": rows.iter().map(|u| json!({
            "name": u.name, "host": u.host, "display": u.display(),
            "superuser": u.is_super, "can_login": u.can_login, "expires": u.expires,
        })).collect::<Vec<_>>(),
    }))
}

fn grants(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let name = p.str_arg("user")?;
    let host = p.str_opt("host").unwrap_or_default();
    let user = nook_model::DbUser {
        name,
        host,
        ..nook_model::DbUser::default()
    };
    let list = {
        let driver = session.driver_for(index)?;
        driver.grants(&user)?
    };
    Ok(json!({ "user": user.display(), "grants": list }))
}

fn mysql_account(p: &Params, user: &str) -> String {
    let host = p.str_opt("host").unwrap_or_else(|| "%".into());
    format!(
        "'{}'@'{}'",
        user.replace('\'', "''"),
        host.replace('\'', "''")
    )
}

fn create_user(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let (kind, _) = kind_and_dialect(session, index)?;
    let user = p.str_arg("user")?;
    let password = p.str_opt("password").unwrap_or_default();
    let can_login = p.bool_opt("can_login", true);
    let superuser = p.bool_opt("superuser", false);
    let sql = match kind {
        DriverKind::Postgres => {
            let mut sql = format!(
                "CREATE ROLE {} WITH {} PASSWORD '{}'",
                quote_ident(&user, Dialect::Postgres),
                if can_login { "LOGIN" } else { "NOLOGIN" },
                password.replace('\'', "''")
            );
            if superuser {
                sql.push_str(" SUPERUSER");
            }
            sql.push(';');
            sql
        }
        _ => {
            let account = mysql_account(p, &user);
            let mut sql = if password.is_empty() {
                format!("CREATE USER {account}")
            } else {
                format!(
                    "CREATE USER {account} IDENTIFIED BY '{}'",
                    password.replace('\'', "''")
                )
            };
            if !can_login {
                sql = if password.is_empty() {
                    format!("CREATE USER {account} ACCOUNT LOCK")
                } else {
                    format!(
                        "CREATE USER {account} IDENTIFIED BY '{}' ACCOUNT LOCK",
                        password.replace('\'', "''")
                    )
                };
            }
            sql.push(';');
            sql
        }
    };
    crate::cmd::run_sql(session, index, &sql, Some(1))?;
    session.log(
        Level::Success,
        format!("created user {user}; the statement was run, not logged"),
    );
    Ok(json!({ "user": user, "created": true }))
}

fn drop_user(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    if session.settings.confirm_destructive && !p.bool_opt("confirm", false) {
        return Err(Error::config(
            "refusing to drop a user without \"confirm\": true",
        ));
    }
    let index = session.resolve_conn(p)?;
    let (kind, dialect) = kind_and_dialect(session, index)?;
    let user = p.str_arg("user")?;
    let sql = match kind {
        DriverKind::Postgres => format!("DROP ROLE IF EXISTS {};", quote_ident(&user, dialect)),
        _ => format!("DROP USER IF EXISTS {};", mysql_account(p, &user)),
    };
    crate::cmd::run_sql(session, index, &sql, Some(1))?;
    session.log(Level::Warn, format!("dropped user {user}"));
    Ok(json!({ "user": user, "dropped": true }))
}

fn set_password(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let (kind, dialect) = kind_and_dialect(session, index)?;
    let user = p.str_arg("user")?;
    let password = p.str_arg("password")?;
    let sql = match kind {
        DriverKind::Postgres => format!(
            "ALTER ROLE {} WITH PASSWORD '{}';",
            quote_ident(&user, dialect),
            password.replace('\'', "''")
        ),
        _ => format!(
            "ALTER USER {} IDENTIFIED BY '{}';",
            mysql_account(p, &user),
            password.replace('\'', "''")
        ),
    };
    crate::cmd::run_sql(session, index, &sql, Some(1))?;
    Ok(json!({ "user": user, "password_changed": true }))
}

/// `SELECT, INSERT` into a checked keyword list — grants cannot be parameterised.
fn privilege_list(p: &Params) -> Result<String> {
    let raw = p
        .str_opt("privileges")
        .ok_or_else(|| Error::config("pass \"privileges\", for example \"SELECT, INSERT\""))?
        .to_ascii_uppercase();
    let mut out = Vec::new();
    for item in raw.split(',') {
        let token = item.trim();
        if token.is_empty() {
            continue;
        }
        let valid = token
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '_');
        if !valid || !KNOWN_PRIVILEGES.contains(&token) {
            return Err(Error::config(format!(
                "'{token}' is not a known privilege; try SELECT, INSERT, UPDATE, DELETE, ALL PRIVILEGES"
            )));
        }
        out.push(token.to_string());
    }
    if out.is_empty() {
        return Err(Error::config(
            "pass \"privileges\", for example \"SELECT, INSERT\"",
        ));
    }
    Ok(out.join(", "))
}

const KNOWN_PRIVILEGES: &[&str] = &[
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "TRUNCATE",
    "REFERENCES",
    "TRIGGER",
    "CREATE",
    "CONNECT",
    "TEMPORARY",
    "EXECUTE",
    "USAGE",
    "ALL PRIVILEGES",
];

fn grant(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let (kind, dialect) = kind_and_dialect(session, index)?;
    let user = p.str_arg("user")?;
    let privileges = privilege_list(p)?;
    let table = p.str_opt("table");
    let schema = p.str_opt("schema");
    let database = p
        .str_opt("database")
        .or_else(|| session.conns.get(index).map(|c| c.cfg.database.clone()))
        .unwrap_or_default();
    let sql = match kind {
        DriverKind::Postgres => {
            let target = match &table {
                Some(t) => {
                    let reference = crate::TableRef::parse(t);
                    quote_qualified(
                        reference.schema.as_deref().or(schema.as_deref()),
                        &reference.table,
                        dialect,
                    )
                }
                None => quote_ident(&database, dialect).replace('"', ""),
            };
            let on = if table.is_some() {
                format!("TABLE {target}")
            } else {
                format!(
                    "DATABASE {}",
                    quote_qualified(None, &database, Dialect::Postgres)
                )
            };
            format!(
                "GRANT {privileges} ON {on} TO {};",
                quote_ident(&user, dialect)
            )
        }
        _ => {
            let scope = match &table {
                Some(t) => {
                    let reference = crate::TableRef::parse(t);
                    format!(
                        "{}.{}",
                        reference.schema.as_deref().unwrap_or(&database),
                        reference.table
                    )
                }
                None => format!("{database}.*"),
            };
            format!(
                "GRANT {privileges} ON {scope} TO {};",
                mysql_account(p, &user)
            )
        }
    };
    crate::cmd::run_sql(session, index, &sql, Some(1))?;
    session.log(Level::Success, format!("granted {privileges} to {user}"));
    Ok(json!({ "sql": sql }))
}

fn revoke(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let (kind, dialect) = kind_and_dialect(session, index)?;
    let user = p.str_arg("user")?;
    let privileges = privilege_list(p)?;
    let table = p.str_opt("table");
    let database = p
        .str_opt("database")
        .or_else(|| session.conns.get(index).map(|c| c.cfg.database.clone()))
        .unwrap_or_default();
    let sql = match kind {
        DriverKind::Postgres => {
            let on = match &table {
                Some(t) => {
                    let reference = crate::TableRef::parse(t);
                    format!(
                        "TABLE {}",
                        quote_qualified(reference.schema.as_deref(), &reference.table, dialect)
                    )
                }
                None => format!(
                    "DATABASE {}",
                    quote_qualified(None, &database, Dialect::Postgres)
                ),
            };
            format!(
                "REVOKE {privileges} ON {on} FROM {};",
                quote_ident(&user, dialect)
            )
        }
        _ => {
            let scope = match &table {
                Some(t) => {
                    let reference = crate::TableRef::parse(t);
                    format!(
                        "{}.{}",
                        reference.schema.as_deref().unwrap_or(&database),
                        reference.table
                    )
                }
                None => format!("{database}.*"),
            };
            format!(
                "REVOKE {privileges} ON {scope} FROM {};",
                mysql_account(p, &user)
            )
        }
    };
    crate::cmd::run_sql(session, index, &sql, Some(1))?;
    session.log(Level::Warn, format!("revoked {privileges} from {user}"));
    Ok(json!({ "sql": sql }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileges_are_checked_against_a_known_list() {
        let p = Params::from_json(json!({"privileges": "select, insert"})).expect("params");
        assert_eq!(privilege_list(&p).expect("list"), "SELECT, INSERT");
        let p = Params::from_json(json!({"privileges": "DROP DATABASE"})).expect("params");
        assert!(privilege_list(&p).is_err());
        let p = Params::from_json(json!({"privileges": "select; drop table x"})).expect("params");
        assert!(privilege_list(&p).is_err());
        let p = Params::from_json(json!({})).expect("params");
        assert!(privilege_list(&p).is_err(), "privileges are required");
        // A closed connection is a different failure from a missing privilege.
        let mut session = Session::new();
        assert!(users(&mut session, &Params::none()).is_err());
    }

    #[test]
    fn sqlite_has_no_privilege_commands() {
        let mut session = Session::new();
        session
            .conns
            .push(crate::Conn::new(nook_core::ConnectionConfig::sqlite(
                "/tmp/x.db",
            )));
        session.active_conn = Some(0);
        assert!(users(&mut session, &Params::none()).is_err());
        let p = Params::from_json(json!({"user": "app", "password": "x"})).expect("params");
        let e = create_user(&mut session, &p).expect_err("no users on sqlite");
        assert!(e.message().contains("file database"));
    }

    #[test]
    fn mysql_accounts_escape_quotes() {
        let p = Params::from_json(json!({"host": "%"})).expect("params");
        assert_eq!(mysql_account(&p, "o'brien"), "'o''brien'@'%'");
        assert_eq!(mysql_account(&Params::none(), "app"), "'app'@'%'");
    }
}
