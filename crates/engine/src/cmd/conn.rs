//! Connection commands: profiles, opening, closing, testing, SSH tunnels, saved passwords.

use nook_core::{ConnectionConfig, DriverKind, Error, Result, SslMode};
use serde_json::json;

use crate::store;
use crate::tunnel::{tunnel_facts, SshSecret};
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
            .params(r#"{"conn":"local","password":"…"}"#)
            .help("Connect, read the version and disconnect without touching the open connection. A \"password\" overrides the stored one for this probe, which is how editor.safeMode proves the credential a user typed."),
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
            .help("Stored in the OS keychain, never in the profile file. Where no keychain answers, the profile keeps it, mode 0600 on unix."),
        CommandSpec::new("conn.forgetPassword", "Remove the saved password", "Connection", forget_password),
        CommandSpec::new("conn.sshPassword", "SSH password in the keychain", "Connection", ssh_password)
            .params(r#"{"password":"…"}"#)
            .help("The tunnel password is kept in the OS keychain under its own account, so ssh reads it back through --ssh-askpass and never from an argument; {\"remove\":true} forgets it. Where no keychain answers, ssh asks on connect."),
        CommandSpec::new("conn.setReadOnly", "Read-only mode", "Connection", set_read_only)
            .params(r#"{"read_only":true}"#),
        CommandSpec::new("conn.setSshTunnel", "SSH tunnel settings", "Connection", set_ssh)
            .params(r#"{"host":"jump.example.com","port":22,"user":"ops","key_file":"~/.ssh/id_ed25519","enabled":true}"#),
        CommandSpec::new("conn.setSsl", "SSL settings", "Connection", set_ssl)
            .params(r#"{"ssl":"require"}"#),
        CommandSpec::new("conn.groups", "Connection groups", "Connection", groups),
        CommandSpec::new("conn.parseUri", "Parse a connection string", "Connection", parse_uri)
            .params(r#"{"uri":"postgres://user:pass@host:5432/db"}"#),
        CommandSpec::new("conn.pin", "Pin connection in the tree", "Connection", pin)
            .params(r#"{"conn":"local","pinned":true}"#),
        CommandSpec::new("conn.reorder", "Set the tree order", "Connection", reorder)
            .params(r#"{"order":["local","stage"]}"#),
        CommandSpec::new("conn.setColor", "Set the tree colour", "Connection", set_color)
            .params(r##"{"conn":"local","color":"#7aa2f7"}"##),
    ]
}

/// Pin or unpin a connection. The store holds the pin set; the tree paints pins first.
fn pin(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let id = p.str_arg("conn")?;
    // Resolve against the profiles so a typo is a named error, not a silent no-op.
    session
        .profiles
        .get(&id)
        .ok_or_else(|| Error::not_found(format!("no connection named '{id}'")))?;
    let pinned = p.bool_opt("pinned", true);
    let mut pins: Vec<String> = session.pins.iter().filter(|x| *x != &id).cloned().collect();
    if pinned {
        pins.push(id.clone());
    }
    store::save_pins(&pins)?;
    session.pins = pins;
    Ok(json!({ "conn": id, "pinned": pinned }))
}

/// Replace the whole tree order. Every named id must exist; nothing named keeps its relative
/// position at the end. This is the data behind drag-and-drop later.
fn reorder(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let order = p.list("order");
    if order.is_empty() {
        return Err(Error::config("order names at least one connection"));
    }
    for id in &order {
        session
            .profiles
            .get(id)
            .ok_or_else(|| Error::not_found(format!("no connection named '{id}'")))?;
    }
    store::save_pins(&order)?;
    session.pins = order.clone();
    Ok(json!({ "order": order }))
}

/// Set the palette colour painted beside a connection in the tree. Empty clears it.
fn set_color(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let id = p.str_arg("conn")?;
    let color = p.str_arg("color")?;
    if !color.is_empty() && !color.starts_with('#') {
        return Err(Error::config(format!("'{color}' is not a hex colour")));
    }
    let cfg = session
        .profiles
        .get(&id)
        .ok_or_else(|| Error::not_found(format!("no connection named '{id}'")))?
        .clone();
    let new_cfg = ConnectionConfig {
        color: color.clone(),
        ..cfg
    };
    session.profiles.upsert(new_cfg.clone())?;
    // The tree paints the live entry's cfg, so mirror the colour into the open connection too —
    // the change must be visible now, not after a restart.
    if let Some(conn) = session.conns.iter_mut().find(|c| c.cfg.id == id) {
        conn.cfg.color = color.clone();
    }
    Ok(json!({ "conn": id, "color": color }))
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
    // Saving over an existing profile must also replace the in-memory config: otherwise a changed
    // host or port is written to disk and the open/close path keeps dialling the old one, which
    // reports a connect failure to a server the profile no longer names.
    match session.conn_index(&id) {
        Ok(index) => {
            let conn = session.conn_mut(index)?;
            if !conn.is_open() {
                conn.cfg = cfg.clone();
            }
        }
        Err(_) => session.conns.push(Conn::new(cfg.clone())),
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
    let mut cfg = session.conns[index].cfg.clone();
    if let Some(name) = p.str_opt("name") {
        // A rename is a real edit; an empty one leaves the name alone.
        if !name.trim().is_empty() {
            cfg.name = name;
        }
    }
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
    if p.has("timeout") {
        cfg.connect_timeout_s = p.int_opt("timeout", 15).clamp(1, 300) as u64;
    }
    session.profiles.upsert(cfg.clone())?;
    session.conns[index].cfg = cfg;
    session.log(Level::Info, "connection updated");
    list(session, p)
}

fn delete(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    super::schema::require_confirm(session, p, "delete a connection")?;
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
        // Both of the connection's accounts, or a deleted profile would leave its SSH password behind.
        let _ = crate::profiles::keychain_delete(&cfg.id);
        let _ = crate::profiles::keychain_delete(&crate::profiles::ssh_account(&cfg.id));
    }
    Ok(json!({ "removed": cfg.name }))
}

/// The id a duplicated connection takes: `base`, or `base-2`, `base-3`, … until it names no connection
/// the session already holds. Ids are what the UI, the catalogue and MCP use to name a connection, so
/// a duplicate one is not a valid key — and `jobs::now()` is whole seconds, so duplicating the same
/// connection twice inside one second is reachable. Same guard `new` applies.
fn unique_id(session: &Session, base: &str) -> String {
    let mut id = base.to_owned();
    let mut suffix = 2;
    while session.conns.iter().any(|conn| conn.cfg.id == id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}

fn duplicate(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let mut cfg = session.conns[index].cfg.clone();
    let new_name = p
        .str_opt("name")
        .unwrap_or_else(|| format!("{} copy", cfg.name));
    let base = format!("{}-{}", cfg.id, crate::jobs::now());
    cfg.id = unique_id(session, &base);
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
    let mut effective = super::dial_config(&cfg)?;
    // A caller-supplied password overrides the stored one: `editor.safeMode` uses this to prove the
    // credential the *user* typed rather than one already in the keychain.
    if let Some(password) = p.str_opt("password") {
        effective.password = Some(password);
    }
    let mut tunnel = None;
    if cfg.ssh.enabled {
        let t = crate::Tunnel::start(&cfg, SshSecret::for_connection(&cfg.id))?;
        effective.ssh.local_port = t.local_port;
        tunnel = Some(t);
    }
    let facts = tunnel.as_ref().map(tunnel_facts);
    let mut driver = nook_drivers::open(&effective)?;
    let server = driver.server_info()?;
    let tables = driver.catalog().map(|c| c.table_count()).unwrap_or(0);
    driver.close();
    probe.state = ConnState::Connected;
    let ms = started.elapsed().as_millis() as u64;
    drop(tunnel);
    Ok(json!({
        "ok": true, "version": server.version, "driver": server.driver,
        "tables": tables, "elapsed_ms": ms,
        "tunnel": facts,
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

/// One id for both directions, because the catalog names one SSH-password feature: a `password`
/// stores it under the connection's SSH account, `{"remove": true}` deletes that account. The
/// password never reaches the profile file — `ConnectionConfig` has no SSH password field at all.
fn ssh_password(session: &mut Session, p: &crate::Params) -> Result<serde_json::Value> {
    if let Some(reason) = crate::profiles::keychain_refusal(crate::profiles::keychain_available()) {
        return Err(Error::unsupported(reason));
    }
    let index = session.resolve_conn(p)?;
    let id = session.conns[index].cfg.id.clone();
    let account = crate::profiles::ssh_account(&id);
    if p.bool_opt("remove", false) {
        crate::profiles::keychain_delete(&account)?;
        return Ok(json!({ "removed": id, "account": account }));
    }
    let password = p
        .str_opt("password")
        .ok_or_else(|| Error::config("pass \"password\", or \"remove\": true"))?;
    crate::profiles::keychain_store(&account, &password)?;
    Ok(json!({ "stored": true, "keychain": "login", "account": account }))
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
    use crate::ProfileStore;
    use nook_core::SshTunnel;

    /// A saved profile with nothing but an id and a name.
    fn profile(id: &str) -> ConnectionConfig {
        let mut cfg = ConnectionConfig::new(id, DriverKind::Postgres);
        cfg.id = id.to_string();
        cfg
    }

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
    fn a_duplicated_connection_gets_an_id_no_other_one_holds() {
        let mut session = Session::new();
        let mut cfg = ConnectionConfig::new("shop-100", DriverKind::Sqlite);
        cfg.id = "shop-100".into();
        session.conns.push(Conn::new(cfg));
        // The second duplicate inside the same second asks for the id the first one already took.
        assert_eq!(unique_id(&session, "shop-100"), "shop-100-2");
        let mut cfg = ConnectionConfig::new("shop-100-2", DriverKind::Sqlite);
        cfg.id = "shop-100-2".into();
        session.conns.push(Conn::new(cfg));
        assert_eq!(unique_id(&session, "shop-100"), "shop-100-3");
        // A free base is taken as it is.
        assert_eq!(unique_id(&session, "shop-101"), "shop-101");
    }

    #[test]
    fn saving_over_a_profile_replaces_the_config_the_connection_holds() {
        // Saving a profile twice with different hosts is how a user fixes a typo or moves a server.
        // The second save wrote the file but left `Conn.cfg` at the first host, so `conn.open` then
        // dialled an address the profile no longer named.
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        let first = Params::from_json(json!({
            "name": "moving", "kind": "mysql", "host": "127.0.0.1", "port": 3306,
            "user": "root", "database": "shop"
        }))
        .expect("params");
        new_conn(&mut session, &first).expect("first save");
        let second = Params::from_json(json!({
            "name": "moving", "kind": "mysql", "host": "127.0.0.1", "port": 3311,
            "user": "root", "database": "shop"
        }))
        .expect("params");
        new_conn(&mut session, &second).expect("second save");

        let index = session.conn_index("moving").expect("one connection");
        assert_eq!(
            session.conns[index].cfg.port, 3311,
            "the in-memory config still dials the old port"
        );
        assert_eq!(
            session.conns.len(),
            1,
            "the second save added a second connection"
        );
    }

    #[test]
    fn unopened_connections_behave() {
        let mut session = Session::new();
        let out = list(&mut session, &Params::none()).expect("list");
        assert_eq!(out["connections"].as_array().map(Vec::len), Some(0));
        assert!(refresh(&mut session, &Params::none()).is_err());
        assert!(open(&mut session, &Params::none()).is_err());
    }

    #[test]
    fn the_tunnel_settings_reach_the_connection_they_name_and_survive_a_reload() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        for id in ["first", "second"] {
            let cfg = profile(id);
            session.profiles.upsert(cfg.clone()).ok();
            session.conns.push(Conn::new(cfg));
        }
        session.active_conn = Some(0);

        let out = session
            .run(
                "conn.setSshTunnel",
                json!({"conn": "second", "enabled": true, "host": "jump.example.com", "port": 2222,
                       "user": "ops", "key_file": "/keys/id", "local_port": 15432}),
            )
            .expect("set the tunnel");
        assert_eq!(out["local_port"], 15432);

        // All six fields, on the connection that was named and not on the active one.
        let second = session.conns[1].cfg.ssh.clone();
        assert_eq!(
            second,
            SshTunnel {
                enabled: true,
                host: "jump.example.com".into(),
                port: 2222,
                user: "ops".into(),
                key_file: "/keys/id".into(),
                local_port: 15432,
            }
        );
        assert_eq!(session.conns[0].cfg.ssh, SshTunnel::default());

        // And the same after a reload: a typed tunnel is a saved tunnel.
        let reloaded = ProfileStore::load().expect("reload");
        assert_eq!(reloaded.get("second").map(|c| c.ssh.clone()), Some(second));
        assert_eq!(
            reloaded.get("first").map(|c| c.ssh.clone()),
            Some(SshTunnel::default())
        );
    }

    #[test]
    fn deleting_a_connection_needs_a_confirmation_that_removes_nothing_else() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        let cfg = profile("doomed");
        session.profiles.upsert(cfg.clone()).ok();
        session.conns.push(Conn::new(cfg));

        let refused = session
            .run("conn.delete", json!({"conn": "doomed"}))
            .expect_err("the gate must refuse");
        assert!(refused.to_string().contains("confirm"), "{refused}");
        assert!(
            session.profiles.get("doomed").is_some(),
            "the refusal removed the profile"
        );
        assert_eq!(session.conns.len(), 1);

        // A connection that is not there is refused too, and removes nothing either.
        assert!(session
            .run("conn.delete", json!({"conn": "ghost", "confirm": true}))
            .is_err());
        assert!(session.profiles.get("doomed").is_some());
        assert_eq!(session.conns.len(), 1);

        let removed = session
            .run("conn.delete", json!({"conn": "doomed", "confirm": true}))
            .expect("delete");
        assert_eq!(removed["removed"], "doomed");
        assert!(session.profiles.get("doomed").is_none());
        assert!(session.conns.is_empty());
    }

    #[test]
    fn deleting_a_connection_takes_both_of_its_keychain_accounts() {
        let (_guard, log) = crate::test_support::fake_security("doomed#ssh", "hunter2");
        let mut session = Session::new();
        let cfg = profile("doomed");
        session.profiles.upsert(cfg.clone()).ok();
        session.conns.push(Conn::new(cfg));
        session
            .run("conn.delete", json!({"conn": "doomed", "confirm": true}))
            .expect("delete");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        if !crate::profiles::keychain_available() {
            // No keychain, so nothing to delete — and no call to the fake either.
            assert!(!calls.contains("delete-generic-password"), "{calls}");
            return;
        }
        // The database password's account and the SSH one, or a deleted profile leaks a password.
        let account_call = |name: &str| {
            calls.lines().any(|line| {
                line.split_whitespace()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|w| (w[0] == "-a" || w[0] == "account") && w[1] == name)
            })
        };
        assert!(account_call("doomed"), "{calls}");
        assert!(account_call("doomed#ssh"), "{calls}");
    }

    #[test]
    fn the_ssh_password_is_stored_and_forgotten_under_its_own_account() {
        let (_guard, log) = crate::test_support::fake_security("doomed#ssh", "hunter2");
        let mut session = Session::new();
        let cfg = profile("doomed");
        session.profiles.upsert(cfg.clone()).ok();
        session.conns.push(Conn::new(cfg));

        if !crate::profiles::keychain_available() {
            let refused = session
                .run(
                    "conn.sshPassword",
                    json!({"conn": "doomed", "password": "hunter2"}),
                )
                .expect_err("no keychain to store into");
            let sentence = refused.to_string();
            assert!(sentence.contains("keychain"), "{sentence}");
            assert!(sentence.contains("ssh asks"), "{sentence}");
            return;
        }

        let stored = session
            .run(
                "conn.sshPassword",
                json!({"conn": "doomed", "password": "hunter2"}),
            )
            .expect("store");
        assert_eq!(stored["account"], "doomed#ssh");
        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        // The account is a token pair in both CLIs — `-a NAME` or `account NAME` — and macOS appends
        // `-w`'s value after it, so the pair is matched wherever it sits on the line.
        let account_call = |name: &str| {
            calls.lines().any(|line| {
                line.split_whitespace()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|w| (w[0] == "-a" || w[0] == "account") && w[1] == name)
            })
        };
        assert!(account_call("doomed#ssh"), "{calls}");
        // The secret must reach the child, not the command line the app was started with. macOS
        // carries it as `security`'s `-w` value; `secret-tool` reads it from stdin on `store`, which
        // the fake records by copying stdin, so on Linux it shows up as its own line.
        let with_secret: Vec<&str> = calls
            .lines()
            .filter(|line| line.contains("hunter2"))
            .collect();
        assert_eq!(with_secret.len(), 1, "{calls}");
        if cfg!(target_os = "macos") {
            assert!(
                with_secret[0].ends_with("-a doomed#ssh -w hunter2"),
                "the secret reached the child by something other than `-w`'s value: {calls}"
            );
        } else {
            assert_eq!(with_secret[0], "hunter2", "{calls}");
        }
        assert_eq!(
            crate::profiles::keychain_lookup("doomed#ssh")
                .expect("lookup")
                .as_deref(),
            Some("hunter2")
        );
        // Nothing reached the profile file: `ConnectionConfig` has no SSH password field at all.
        let saved = std::fs::read_to_string(crate::profiles::profiles_path()).unwrap_or_default();
        assert!(!saved.contains("hunter2"), "{saved}");

        let gone = session
            .run(
                "conn.sshPassword",
                json!({"conn": "doomed", "remove": true}),
            )
            .expect("forget");
        assert_eq!(gone["removed"], "doomed");
        // Neither direction takes a password from the command line arguments of the app.
        assert!(session
            .run("conn.sshPassword", json!({"conn": "doomed"}))
            .is_err());
    }

    #[test]
    fn editing_a_connection_changes_the_one_it_names_with_its_name_and_timeout() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        for id in ["first", "second"] {
            let mut cfg = profile(id);
            cfg.name = id.to_string();
            session.profiles.upsert(cfg.clone()).ok();
            session.conns.push(Conn::new(cfg));
        }
        session.active_conn = Some(0);
        let first_before = session.conns[0].cfg.clone();

        session
            .run(
                "conn.edit",
                json!({"conn": "second", "host": "db.example.com", "name": "Second shop",
                       "timeout": 42}),
            )
            .expect("edit");

        // The connection that was named moved; the active one — which is a different connection —
        // did not.
        assert_eq!(session.conns[1].cfg.host, "db.example.com");
        assert_eq!(session.conns[1].cfg.name, "Second shop");
        assert_eq!(session.conns[1].cfg.connect_timeout_s, 42);
        assert_eq!(session.conns[0].cfg.host, first_before.host);
        assert_eq!(session.conns[0].cfg.name, first_before.name);
        assert_eq!(
            session.conns[0].cfg.connect_timeout_s,
            first_before.connect_timeout_s
        );

        // An edit is a save: both fields come back from the store.
        let reloaded = ProfileStore::load().expect("reload");
        assert_eq!(
            reloaded
                .get("second")
                .map(|cfg| (cfg.name.clone(), cfg.connect_timeout_s)),
            Some(("Second shop".to_string(), 42))
        );
        assert_eq!(
            reloaded.get("first").map(|cfg| cfg.name.clone()),
            Some("first".to_string())
        );

        // An empty name is not a rename, and the timeout stays inside the engine's own range.
        session
            .run("conn.edit", json!({"conn": "second", "name": "   "}))
            .expect("edit");
        assert_eq!(session.conns[1].cfg.name, "Second shop");
        session
            .run("conn.edit", json!({"conn": "second", "timeout": 9000}))
            .expect("edit");
        assert_eq!(session.conns[1].cfg.connect_timeout_s, 300);
    }

    #[test]
    fn testing_a_connection_says_so_when_no_tunnel_ran() {
        let _guard = crate::test_support::scratch_config();
        let mut session = Session::new();
        let mut cfg = ConnectionConfig::sqlite(
            std::env::temp_dir()
                .join("nook-test-conn.db")
                .to_string_lossy()
                .to_string(),
        );
        cfg.id = "local".into();
        cfg.name = "local".into();
        session.profiles.upsert(cfg.clone()).ok();
        session.conns.push(Conn::new(cfg));

        let out = session
            .run("conn.test", json!({"conn": "local"}))
            .expect("test");
        assert_eq!(out["ok"], true);
        // Additive: the key is always there, and null exactly when no hop was made.
        assert!(
            out.as_object().is_some_and(|o| o.contains_key("tunnel")),
            "{out}"
        );
        assert!(out["tunnel"].is_null(), "{out}");
    }
}
