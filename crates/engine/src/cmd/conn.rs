//! Connection commands: profiles, opening, closing, testing, SSH tunnels, saved passwords.

use datacraft_core::{ConnectionConfig, DriverKind, Error, Result, SslMode};
use serde_json::json;

use crate::{CommandSpec, Conn, ConnState, Level, Session};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("conn.list", "Connection list", "Connection", list)
            .params("{}")
            .help("Every known connection with its state and server version."),
        CommandSpec::new("conn.new", "New connection", "Connection", new_conn)
            .params(r#"{"name":"Local PG","kind":"postgres","host":"127.0.0.1","port":5432,"user":"me","database":"shop"}"#)
            .help("Create and save a connection profile. SQLite takes {\"file\":\"/path/app.db\"}."),
        CommandSpec::new("conn.edit", "Edit connection", "Connection", edit)
            .params(r#"{"conn":"local","host":"db.internal"}"#),
        CommandSpec::new("conn.delete", "Delete connection", "Connection", delete),
        CommandSpec::new("conn.duplicate", "Duplicate connection", "Connection", duplicate),
        CommandSpec::new("conn.open", "Open connection", "Connection", open).shortcut("cmd+o"),
        CommandSpec::new("conn.close", "Close connection", "Connection", close),
        CommandSpec::new("conn.closeAll", "Close all connections", "Connection", close_all),
        CommandSpec::new("conn.test", "Test connection", "Connection", test)
            .help("Connect, read the version and disconnect without touching the open connection."),
        CommandSpec::new("conn.reconnect", "Reconnect", "Connection", reconnect),
        CommandSpec::new("conn.select", "Make active", "Connection", select),
        CommandSpec::new("conn.refresh", "Refresh server", "Connection", refresh)
            .help("Re-read the catalog: schemas, tables, columns, indexes and foreign keys."),
        CommandSpec::new("conn.databases", "List databases", "Connection", databases),
        CommandSpec::new("conn.useDatabase", "Switch database", "Connection", use_database)
            .params(r#"{"database":"shop"}"#),
        CommandSpec::new("conn.serverInfo", "Server information", "Connection", server_info),
        CommandSpec::new("conn.savePassword", "Save password in the keychain", "Connection", save_password)
            .params(r#"{"password":"…"}"#)
            .help("macOS: stored in the login keychain, never in the profile file."),
        CommandSpec::new("conn.forgetPassword", "Remove the saved password", "Connection", forget_password),
        CommandSpec::new("conn.setReadOnly", "Read-only mode", "Connection", set_read_only)
            .params(r#"{"read_only":true}"#),
        CommandSpec::new("conn.setSshTunnel", "SSH tunnel settings", "Connection", set_ssh)
            .params(r#"{"host":"jump.example.com","port":22,"user":"ops","key_file":"~/.ssh/id_ed25519","enabled":true}"#),
        CommandSpec::new("conn.setSsl", "SSL settings", "Connection", set_ssl)
            .params(r#"{"ssl":"require"}"#),
        CommandSpec::new("conn.groups", "Connection groups", "Connection", groups),
        CommandSpec::new("conn.parseUri", "Parse a connection string", "Connection", parse_uri)
            .params(r#"{"uri":"postgres://user:pass@host:5432/db"}"#),
    ]
}

fn list(session: &mut Session, _p: &crate::Params) -> Result<serde_json::Value> {
    Ok(json!({
        "active": session.active_conn,
        "connections": session.conns.iter().map(|c| json!({
            "id": c.cfg.id,
            "name": c.cfg.name,
            "kind": c.cfg.kind.label(),
            "state": match c.state { ConnState::Connected => "connected", ConnState::Failed => "failed", ConnState::Disconnected => "closed" },
            "target": c.cfg.display_uri(),
            "database": c.cfg.database,
            "group": c.cfg.group,
            "ssh": c.cfg.ssh.enabled,
            "ssl": c.cfg.ssl.label(),
            "read_only": c.cfg.read_only,
            "server": c.server.as_ref().map(|s| s.version.clone()),
            "tables": c.catalog.as_ref().map(|cat| cat.table_count()),
            "error": c.error,
        })).collect::<Vec<_>>(),
        "saved_profiles": session.profiles.list().len(),
    }))
}

fn build_config(params: &crate::Params, name: &str) -> Result<ConnectionConfig> {
    let kind = params
        .str_opt("kind")
        .and_then(|k| DriverKind::parse(&k))
        .unwrap_or(DriverKind::Sqlite);
    let mut cfg = ConnectionConfig::new(name, kind);
    if let Some(file) = params.str_opt("file") {
        cfg.file = file.clone();
        cfg.id = if cfg.file.is_empty() {
            cfg.id
        } else {
            cfg.file.clone()
        };
    }
    if let Some(host) = params.str_opt("host") {
        cfg.host = host;
    }
    if params.has("port") {
        cfg.port = params.int_opt("port", i64::from(kind.default_port())) as u16;
    }
    if let Some(user) = params.str_opt("user") {
        cfg.user = user;
    }
    if let Some(db) = params.str_opt("database") {
        cfg.database = db;
    }
    if params.has("password") {
        cfg.password = params.str_opt("password");
    }
    if let Some(group) = params.str_opt("group") {
        cfg.group = group;
    }
    if let Some(id) = params.str_opt("id") {
        cfg.id = id;
    }
    if let Some(color) = params.str_opt("color") {
        cfg.color = color;
    }
    if params.has("read_only") {
        cfg.read_only = params.bool_opt("read_only", false);
    }
    if let Some(ssl) = params.str_opt("ssl") {
        cfg.ssl = SslMode::parse(&ssl).ok_or_else(|| {
            Error::config("ssl must be disable, prefer, require, verify-ca or verify-full")
        })?;
    }
    if params.has("timeout") {
        cfg.connect_timeout_s = params.int_opt("timeout", 15).clamp(1, 300) as u64;
    }
    Ok(cfg)
}

fn new_conn(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let name = p
        .str_opt("name")
        .or_else(|| p.str_opt("file"))
        .ok_or_else(|| Error::config("a connection needs a name"))?;
    let cfg = build_config(p, &name)?;
    if cfg.kind.is_file_based() && cfg.file.trim().is_empty() {
        return Err(Error::config("a SQLite connection needs \"file\""));
    }
    if cfg.kind.implemented() && cfg.id.trim().is_empty() {
        return Err(Error::config("a connection needs a non-empty id"));
    }
    let id = cfg.id.clone();
    session.profiles.upsert(cfg.clone())?;
    if session.conn_index(&id).is_err() {
        session.conns.push(Conn::new(cfg.clone()));
    }
    // Passwords belong in the keychain, never in the profile file.
    if let Some(password) = &cfg.password {
        let wants_keychain = crate::profiles::keychain_available() && p.bool_opt("keychain", true);
        let stored_in_keychain =
            wants_keychain && crate::profiles::keychain_store(&id, password).is_ok();
        if stored_in_keychain {
            if let Some(stored) = session.profiles.get(&id).cloned() {
                let mut cleared = stored;
                cleared.password = None;
                let _ = session.profiles.upsert(cleared.clone());
                if let Some(conn) = session.conns.iter_mut().find(|c| c.cfg.id == id) {
                    conn.cfg.password = None;
                }
            }
        }
    }
    session.log(Level::Info, format!("saved connection '{name}'"));
    list(session, p)
}

fn edit(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let name = session.conns[index].cfg.name.clone();
    let mut cfg = session.conns[index].cfg.clone();
    if let Some(host) = p.str_opt("host") {
        cfg.host = host;
    }
    if p.has("port") {
        cfg.port = p.int_opt("port", i64::from(cfg.port)) as u16;
    }
    if let Some(user) = p.str_opt("user") {
        cfg.user = user;
    }
    if let Some(db) = p.str_opt("database") {
        cfg.database = db;
    }
    if p.has("password") {
        cfg.password = p.str_opt("password");
    }
    if let Some(group) = p.str_opt("group") {
        cfg.group = group;
    }
    if let Some(color) = p.str_opt("color") {
        cfg.color = color;
    }
    if p.has("read_only") {
        cfg.read_only = p.bool_opt("read_only", false);
    }
    if let Some(ssl) = p.str_opt("ssl") {
        cfg.ssl = SslMode::parse(&ssl).ok_or_else(|| Error::config("bad ssl mode"))?;
    }
    cfg.name = name;
    session.profiles.upsert(cfg.clone())?;
    session.conns[index].cfg = cfg;
    session.log(Level::Info, "connection updated");
    list(session, p)
}

fn delete(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let cfg = session.profiles.remove(&session.conns[index].cfg.id)?;
    super::close_conn(session, index)?;
    session.conns.remove(index);
    session.active_conn = if session.conns.is_empty() {
        None
    } else {
        Some(
            session
                .active_conn
                .unwrap_or(0)
                .min(session.conns.len() - 1),
        )
    };
    if crate::profiles::keychain_available() {
        let _ = crate::profiles::keychain_delete(&cfg.id);
    }
    Ok(json!({ "removed": cfg.name }))
}

fn duplicate(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let mut cfg = session.conns[index].cfg.clone();
    let new_name = p
        .str_opt("name")
        .unwrap_or_else(|| format!("{} copy", cfg.name));
    cfg.id = format!("{}-{}", cfg.id, crate::jobs::now());
    cfg.name = new_name;
    cfg.password = crate::profiles::keychain_lookup(&session.conns[index].cfg.id)?;
    session.profiles.upsert(cfg.clone())?;
    session.conns.push(Conn::new(cfg));
    list(session, p)
}

fn open(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    super::open_conn(session, index)?;
    list(session, p)
}

fn close(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    super::close_conn(session, index)?;
    list(session, p)
}

fn close_all(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    for index in 0..session.conns.len() {
        let _ = super::close_conn(session, index);
    }
    session.active_conn = None;
    list(session, p)
}

fn test(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let cfg = session.conns[index].cfg.clone();
    let mut probe = crate::Conn::new(cfg.clone());
    let started = std::time::Instant::now();
    let mut effective = cfg.clone();
    let mut tunnel = None;
    if cfg.ssh.enabled {
        let t = crate::Tunnel::start(&cfg)?;
        effective.ssh.local_port = t.local_port;
        tunnel = Some(t);
    }
    let mut driver = datacraft_drivers::open(&effective)?;
    let server = driver.server_info()?;
    let tables = driver.catalog().map(|c| c.table_count()).unwrap_or(0);
    driver.close();
    probe.state = ConnState::Connected;
    let ms = started.elapsed().as_millis() as u64;
    drop(tunnel);
    Ok(json!({
        "ok": true, "version": server.version, "driver": server.driver,
        "tables": tables, "elapsed_ms": ms,
    }))
}

fn reconnect(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let _ = super::close_conn(session, index);
    super::open_conn(session, index)?;
    list(session, p)
}

fn select(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    session.active_conn = Some(index);
    Ok(json!({ "active": index, "name": session.conns[index].cfg.name }))
}

fn refresh(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let ms = super::refresh_catalog(session, index)?;
    let conn = &session.conns[index];
    Ok(json!({
        "connection": conn.cfg.name,
        "catalog_ms": ms,
        "tables": conn.catalog.as_ref().map(|c| c.table_count()).unwrap_or(0),
        "columns": conn.catalog.as_ref().map(|c| c.column_count()).unwrap_or(0),
        "schemas": conn.catalog.as_ref().map(|c| c.schema_names()).unwrap_or_default(),
    }))
}

fn databases(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let current = session.conns[index].cfg.database.clone();
    let list = {
        let driver = session.driver_for(index)?;
        driver.databases()?
    };
    Ok(json!({ "current": current, "databases": list }))
}

fn use_database(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let db = p.str_arg("database")?;
    {
        let driver = session.driver_for(index)?;
        driver.use_database(&db)?;
    }
    if let Ok(conn) = session.conn_mut(index) {
        conn.cfg.database = db.clone();
    }
    super::refresh_catalog(session, index)?;
    Ok(json!({ "database": db }))
}

fn server_info(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let info = {
        let driver = session.driver_for(index)?;
        driver.server_info()?
    };
    Ok(json!({
        "version": info.version, "driver": info.driver, "database": info.database,
        "host": info.host, "uptime_s": info.uptime_s, "size_bytes": info.size_bytes,
        "read_only": info.read_only,
    }))
}

fn save_password(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    if !crate::profiles::keychain_available() {
        return Err(Error::unsupported(
            "the keychain is macOS only here; passwords stay in the profile file (mode 0600)",
        ));
    }
    let index = session.resolve_conn(p)?;
    let id = session.conns[index].cfg.id.clone();
    let password = p
        .str_opt("password")
        .ok_or_else(|| Error::config("pass \"password\""))?;
    crate::profiles::keychain_store(&id, &password)?;
    if let Some(conn) = session.conns.get_mut(index) {
        conn.cfg.password = None;
    }
    if let Some(stored) = session.profiles.get(&id).cloned() {
        let mut cleared = stored;
        cleared.password = None;
        session.profiles.upsert(cleared)?;
    }
    Ok(json!({ "stored": true, "keychain": "login" }))
}

fn forget_password(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let id = session.conns[index].cfg.id.clone();
    crate::profiles::keychain_delete(&id)?;
    Ok(json!({ "removed": id }))
}

fn set_read_only(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let value = p.bool_opt("read_only", true);
    let cfg = {
        let conn = session.conn_mut(index)?;
        conn.cfg.read_only = value;
        conn.cfg.clone()
    };
    session.profiles.upsert(cfg)?;
    Ok(json!({ "read_only": value }))
}

fn set_ssh(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let cfg = {
        let conn = session.conn_mut(index)?;
        if p.has("enabled") {
            conn.cfg.ssh.enabled = p.bool_opt("enabled", false);
        }
        if let Some(host) = p.str_opt("host") {
            conn.cfg.ssh.host = host;
        }
        if p.has("port") {
            conn.cfg.ssh.port = p.int_opt("port", 22).clamp(1, 65535) as u16;
        }
        if let Some(user) = p.str_opt("user") {
            conn.cfg.ssh.user = user;
        }
        if let Some(key) = p.str_opt("key_file") {
            conn.cfg.ssh.key_file = key;
        }
        if p.has("local_port") {
            conn.cfg.ssh.local_port = p.int_opt("local_port", 0).clamp(0, 65535) as u16;
        }
        conn.cfg.clone()
    };
    session.profiles.upsert(cfg.clone())?;
    Ok(json!({
        "enabled": cfg.ssh.enabled, "host": cfg.ssh.host, "port": cfg.ssh.port,
        "user": cfg.ssh.user, "key_file": cfg.ssh.key_file, "local_port": cfg.ssh.local_port,
    }))
}

fn set_ssl(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let value = p.str_arg("ssl")?;
    let mode = SslMode::parse(&value).ok_or_else(|| {
        Error::config("ssl must be disable, prefer, require, verify-ca or verify-full")
    })?;
    let cfg = {
        let conn = session.conn_mut(index)?;
        conn.cfg.ssl = mode;
        conn.cfg.clone()
    };
    session.profiles.upsert(cfg)?;
    Ok(json!({ "ssl": mode.label() }))
}

fn groups(session: &mut Session, _p: &crate::Params) -> Result<serde_json::Value> {
    Ok(json!({ "groups": session.profiles.groups() }))
}

fn parse_uri(_session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let uri = p.str_arg("uri")?;
    let cfg = ConnectionConfig::parse(&uri).map_err(Error::config)?;
    Ok(json!({
        "id": cfg.id, "name": cfg.name, "kind": cfg.kind.label(),
        "host": cfg.host, "port": cfg.port, "user": cfg.user,
        "database": cfg.database, "file": cfg.file, "implemented": cfg.kind.implemented(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    #[test]
    fn uri_parsing_reports_the_pieces() {
        let mut session = Session::new();
        let params =
            Params::from_json(json!({"uri": "mysql://root@localhost:3307/shop"})).expect("params");
        let out = parse_uri(&mut session, &params).expect("parse");
        assert_eq!(out["port"], 3307);
        assert_eq!(out["kind"], "MySQL");
        let params = Params::from_json(json!({"uri": "oracle://x"})).expect("params");
        assert!(parse_uri(&mut session, &params).is_err());
    }

    #[test]
    fn unopened_connections_behave() {
        let mut session = Session::new();
        let out = list(&mut session, &Params::none()).expect("list");
        assert_eq!(out["connections"].as_array().map(Vec::len), Some(0));
        assert!(refresh(&mut session, &Params::none()).is_err());
        assert!(open(&mut session, &Params::none()).is_err());
    }
}
