//! Phase 2: Open Anything. `quickOpen.search` ranks objects, saved queries and open tabs;
//! `quickOpen.open` opens the hit; `tools.serverSearch` is the same index, objects only, under the
//! Navicat id. Every test opens a real sqlite catalog so the index is exercised end to end.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
    path: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-quick-open");
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
        let path = dir.join(format!("{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", path.display()))
            .expect("a sqlite URI parses");
        let conn = cfg.id.clone();
        let mut session = Session::new();
        session.conns.push(Conn::new(cfg));
        let index = session
            .conn_index(&conn)
            .expect("the connection is registered");
        open_now(&mut session, index).expect("sqlite opens");
        Fixture {
            session,
            conn,
            path,
        }
    }

    fn sql(&mut self, sql: &str) -> serde_json::Value {
        self.session
            .run("sql.run", json!({"sql": sql, "conn": self.conn}))
            .expect("sql runs")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn hits(out: &serde_json::Value) -> Vec<(String, String, String)> {
    out["hits"]
        .as_array()
        .map(|hs| {
            hs.iter()
                .map(|h| {
                    (
                        h["kind"].as_str().unwrap_or_default().to_string(),
                        h["conn"].as_str().unwrap_or_default().to_string(),
                        h["name"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn search_finds_a_table_by_name_over_the_live_catalog() {
    let mut f = Fixture::new("by-name");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY)");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let out = f
        .session
        .run("quickOpen.search", json!({"query":"cust"}))
        .expect("search");
    let names: Vec<_> = hits(&out).into_iter().map(|(_, _, n)| n).collect();
    assert!(names.contains(&"customers".to_string()), "hits: {names:?}");
    assert!(!names.iter().any(|n| n == "orders"), "hits: {names:?}");
}

#[test]
fn search_finds_a_table_by_column_name() {
    let mut f = Fixture::new("by-column");
    f.sql("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)");
    let out = f
        .session
        .run("quickOpen.search", json!({"query":"email"}))
        .expect("search");
    let names: Vec<_> = hits(&out).into_iter().map(|(_, _, n)| n).collect();
    assert_eq!(names, vec!["users".to_string()]);
}

#[test]
fn an_empty_query_returns_nothing() {
    let mut f = Fixture::new("empty");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY)");
    let out = f
        .session
        .run("quickOpen.search", json!({"query":""}))
        .expect("search");
    assert_eq!(out["total"], json!(0), "empty must not browse: {out}");
}

#[test]
fn closed_connections_contribute_nothing_and_open_ones_carry_their_id() {
    let mut f = Fixture::new("closed");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY)");
    // A second connection that is registered but never opened: no catalog, no hits.
    let cfg = nook_core::ConnectionConfig::parse("sqlite::memory:unopened-never-opened")
        .expect("a sqlite URI parses");
    f.session.conns.push(Conn::new(cfg));
    let out = f
        .session
        .run("quickOpen.search", json!({"query":"customers"}))
        .expect("search");
    let found = hits(&out);
    assert!(found.iter().all(|(_, c, _)| c == &f.conn), "{found:?}");
    assert_eq!(found.len(), 1, "one open catalog, one hit: {found:?}");
}

#[test]
fn server_search_returns_objects_and_closes_the_catalog_id() {
    let mut f = Fixture::new("server");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let out = f
        .session
        .run("tools.serverSearch", json!({"pattern":"order"}))
        .expect("server search");
    let found = hits(&out);
    assert_eq!(
        found,
        vec![("table".to_string(), f.conn.clone(), "orders".to_string())]
    );
    // The Navicat id is closed: parity no longer lists it as missing.
    let missing: Vec<&str> = nook_engine::parity::measure().missing;
    assert!(
        !missing.contains(&"tools.serverSearch"),
        "the Navicat id must be covered: {missing:?}"
    );
}

#[test]
fn opening_a_table_creates_a_data_tab_and_a_missing_connection_is_a_named_error() {
    let mut f = Fixture::new("open-table");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY)");
    f.session
        .run("quickOpen.open", json!({"kind":"table","conn":f.conn,"name":"customers"}))
        .expect("open");
    let tabs = &f.session.tabs;
    let data = tabs
        .iter()
        .find(|t| t.kind == nook_engine::TabKind::Data)
        .expect("the Data tab");
    assert_eq!(data.title, "customers");
    assert!(data.table.is_some(), "the grid is bound: {tabs:?}");
    assert_eq!(
        data.table.as_ref().map(|t| t.name.clone()),
        Some("customers".into())
    );
    let err = f
        .session
        .run("quickOpen.open", json!({"kind":"table","conn":"ghost","name":"customers"}))
        .unwrap_err();
    assert!(err.message().contains("ghost"), "must name the id: {err:?}");
}

#[test]
fn opening_a_saved_query_creates_a_query_tab_with_its_sql() {
    let mut f = Fixture::new("open-saved");
    f.session
        .run("snippet.save", json!({"name":"Big spenders","sql":"select * from orders"}))
        .expect("save snippet");
    let out = f
        .session
        .run("quickOpen.search", json!({"query":"spend"}))
        .expect("search");
    assert!(
        hits(&out).iter().any(|(k, _, n)| k == "saved" && n == "Big spenders"),
        "{out:?}"
    );
    f.session
        .run("quickOpen.open", json!({"kind":"saved","name":"Big spenders"}))
        .expect("open");
    let tab = f.session.tabs.last().cloned().expect("the tab exists");
    assert_eq!(tab.kind, nook_engine::TabKind::Query);
    assert_eq!(tab.sql.text, "select * from orders");
}

#[test]
fn opening_a_tab_hit_makes_it_active() {
    let mut f = Fixture::new("open-tab");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    f.session
        .run("quickOpen.open", json!({"kind":"table","conn":f.conn,"name":"orders"}))
        .expect("first tab");
    f.session
        .run("tab.new", json!({"kind":"Query","title":"scratch"}))
        .expect("second tab");
    f.session
        .run("quickOpen.open", json!({"kind":"tab","name":"orders"}))
        .expect("activate");
    assert_eq!(f.session.active_tab, Some(1), "{:?}", f.session.tabs.iter().map(|t| &t.title).collect::<Vec<_>>());
}

#[test]
fn junk_parameters_are_rejected_not_ignored() {
    let mut f = Fixture::new("junk");
    let err = f
        .session
        .run("quickOpen.search", json!({"query":[1,2]}))
        .unwrap_err();
    assert!(!err.message().is_empty());
    let err = f
        .session
        .run("quickOpen.open", json!({"kind":"purple","name":"x"}))
        .unwrap_err();
    assert!(err.message().contains("purple"), "{err:?}");
    let err = f
        .session
        .run("tools.serverSearch", json!({"pattern":{"x":1}}))
        .unwrap_err();
    assert!(!err.message().is_empty());
}
