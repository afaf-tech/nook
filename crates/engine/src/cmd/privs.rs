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
            CommandSpec::new("priv.roles", "Roles", "Privileges", roles)
            .params(r#"{"role":"readers","member":"analyst","action":"grant"}"#)
            .help("Role membership: who is in which role. Without \"action\" it lists the roles and their members; with \"grant\" or \"revoke\" it changes the membership."),
        CommandSpec::new("priv.audit", "Privilege audit", "Privileges", audit)
            .params(r#"{"schema":"public"}"#)
            .help("Every privilege on every object with where it came from: granted directly, through a role, by PUBLIC, or by ownership. SQLite files have no principals, so it refuses there."),
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

/// Role membership: who is in which role, and the write side of it.
///
/// Postgres keeps this in `pg_auth_members` joined to the role catalog; MySQL 8 in `mysql.role_edges`.
/// SQLite has no principals at all, so it refuses rather than returning an empty membership list that
/// reads as "nobody has any role".
fn roles(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    // `kind_and_dialect` already refuses a file database with its own sentence, so a second check
    // here would be unreachable.
    let (kind, _) = kind_and_dialect(session, index)?;
    let action = p.str_opt("action").unwrap_or_default();
    if action.is_empty() {
        let sql = match kind {
            DriverKind::Postgres => {
                "SELECT r.rolname AS role, m.rolname AS member, am.admin_option \
                 FROM pg_auth_members am \
                 JOIN pg_roles r ON r.oid = am.roleid \
                 JOIN pg_roles m ON m.oid = am.member \
                 ORDER BY r.rolname, m.rolname"
            }
            _ => {
                // `mysql.role_edges` has no `TO_ROLE`: it is (FROM_USER, FROM_HOST) → (TO_USER,
                 // TO_HOST), so the role is the FROM side and the member is the TO side. Naming the
                 // columns backwards made MySQL 8.4 answer `Unknown column 'TO_ROLE'`.
                "SELECT FROM_USER AS role, TO_USER AS member, WITH_ADMIN_OPTION AS admin_option \
                 FROM mysql.role_edges ORDER BY FROM_USER, TO_USER"
            }
        };
        let result = crate::cmd::run_sql(session, index, sql, None)?;
        return Ok(json!({
            "memberships": result.to_json()["rows"],
            "count": result.row_count(),
        }));
    }
    if action != "grant" && action != "revoke" {
        return Err(Error::config(format!(
            "unknown action {action:?}; use \"grant\" or \"revoke\", or leave it out to list"
        )));
    }
    let role = p.str_arg("role")?;
    let member = p.str_arg("member")?;
    // A role name cannot be a bound parameter in GRANT, so it is quoted by the dialect's own rule
    // rather than pasted.
    let dialect = session
        .conns
        .get(index)
        .and_then(|c| c.driver.as_ref())
        .map(|d| d.dialect())
        .unwrap_or(nook_core::Dialect::Postgres);
    let sql = match (kind, action.as_str()) {
        (DriverKind::Postgres, "grant") => format!(
            "GRANT {} TO {};",
            quote_ident(&role, dialect),
            quote_ident(&member, dialect)
        ),
        (DriverKind::Postgres, _) => format!(
            "REVOKE {} FROM {};",
            quote_ident(&role, dialect),
            quote_ident(&member, dialect)
        ),
        (_, "grant") => format!(
            "GRANT {} TO {};",
            quote_ident(&role, Dialect::MySql),
            mysql_account(p, &member)
        ),
        (_, _) => format!(
            "REVOKE {} FROM {};",
            quote_ident(&role, Dialect::MySql),
            mysql_account(p, &member)
        ),
    };
    crate::cmd::run_sql(session, index, &sql, Some(1))?;
    session.log(
        Level::Success,
        format!("{action}ed role {role} for {member}"),
    );
    Ok(json!({ "sql": sql, "action": action, "role": role, "member": member }))
}

/// Every privilege on every object, with where it came from.
///
/// `via` is the column that makes this an audit rather than a list: `direct`, `role:<name>`, `public`
/// or `owner`. A privilege a user only has because they own the table is a different risk from one
/// granted to PUBLIC, and a table that shows them the same way teaches the wrong lesson.
fn audit(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    // Refused above for a file database: an audit of principals a file does not have would be an
    // empty table claiming nothing is granted.
    let (kind, _) = kind_and_dialect(session, index)?;
    let schema = p.str_opt("schema").unwrap_or_else(|| "public".into());
    let sql = match kind {
        DriverKind::Postgres => format!(
            "WITH RECURSIVE membership(member, role, depth) AS ( \
               SELECT m.oid, m.oid, 0 FROM pg_roles m \
               UNION ALL \
               SELECT mb.member, am.roleid, mb.depth + 1 \
               FROM membership mb JOIN pg_auth_members am ON am.member = mb.role \
               WHERE mb.depth < 10 \
             ) \
             SELECT c.relname AS object, \
                    COALESCE(g.grantee, r.rolname) AS principal, \
                    g.privilege_type AS privilege, \
                    CASE \
                      WHEN g.grantee = 0 THEN 'public' \
                      WHEN g.grantee = c.relowner THEN 'owner' \
                      WHEN g.grantee = r.oid THEN 'direct' \
                      ELSE 'role:' || r.rolname \
                    END AS via \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN information_schema.role_table_grants g \
                    ON g.table_name = c.relname AND g.table_schema = n.nspname \
             LEFT JOIN pg_roles r ON r.oid = g.grantee \
             WHERE n.nspname = '{}' AND c.relkind IN ('r','v','m','p') \
             ORDER BY c.relname, principal, privilege",
            schema.replace('\'', "''")
        ),
        // MySQL 8 has `mysql.role_edges`, so the `via` column is answerable there too. On 5.7 the
        // table does not exist and the union's second arm is empty, which reads as "nothing came
        // through a role" — true on a server that has no roles.
        //
        // `TABLE_PRIVILEGES.GRANTEE` is `'alice'@'%'`, while `role_edges` holds bare names, so the
        // account has to be unwrapped before it can be matched. A grant made to a *role* is not the
        // role holder's grant: it has to be re-attributed to every account the role reaches, which is
        // the union's second arm. Joining on the grantee alone labelled the role's own row as
        // inherited and hid the holder entirely.
        _ => format!(
            "SELECT object, principal, privilege, via FROM ( \
               SELECT p.TABLE_NAME AS object, \
                      REPLACE(SUBSTRING_INDEX(p.GRANTEE,'@',1),CHAR(39),'') AS principal, \
                      p.PRIVILEGE_TYPE AS privilege, 'direct' AS via \
               FROM information_schema.TABLE_PRIVILEGES p \
               WHERE p.TABLE_SCHEMA = '{schema}' \
                 AND REPLACE(SUBSTRING_INDEX(p.GRANTEE,'@',1),CHAR(39),'') NOT IN \
                     (SELECT FROM_USER FROM mysql.role_edges) \
               UNION ALL \
               SELECT p.TABLE_NAME AS object, re.TO_USER AS principal, \
                      p.PRIVILEGE_TYPE AS privilege, CONCAT('role:', re.FROM_USER) AS via \
               FROM information_schema.TABLE_PRIVILEGES p \
               JOIN mysql.role_edges re \
                 ON re.FROM_USER = REPLACE(SUBSTRING_INDEX(p.GRANTEE,'@',1),CHAR(39),'') \
               WHERE p.TABLE_SCHEMA = '{schema}' \
             ) x ORDER BY object, principal, privilege",
            schema = schema.replace('\'', "''")
        ),
    };
    let result = crate::cmd::run_sql(session, index, &sql, None)?;
    Ok(json!({
        "schema": schema,
        "grants": result.to_json()["rows"],
        "count": result.row_count(),
    }))
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
    super::require_safe_mode_for_params(session, p)?;
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
