//! Phase 1: state that survives a restart. Snippets, history and the tab snapshot are written
//! through `store.rs`, so a fresh `Session` reads back what an earlier one saved. Every test uses
//! the `NOOK_CONFIG_DIR` seam; each writes with one session and reads with a second, which is the
//! process-shaped boundary.

use nook_engine::{HistoryEntry, Session};
use serde_json::json;

struct Scratch {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join("nook-state-persistence");
        let _ = std::fs::create_dir_all(&dir);
        let guard = nook_engine::test_lock();
        for file in ["jobs.json", "profiles.json", "nook.db"] {
            let _ = std::fs::remove_file(dir.join(file));
        }
        std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
        Scratch { _guard: guard }
    }
}

#[test]
fn snippet_save_survives_a_fresh_session() {
    let _s = Scratch::new();
    let mut a = Session::new();
    a.run(
        "snippet.save",
        json!({"name":"mine","sql":"select 42","tags":["t"]}),
    )
    .expect("save");
    let mut b = Session::new();
    let out = b.run("snippet.list", json!({})).expect("list");
    let list = out["snippets"].as_array().expect("snippets array");
    let mine = list
        .iter()
        .find(|s| s["name"] == "mine")
        .expect("saved snippet present");
    assert_eq!(mine["sql"], "select 42");
    assert_eq!(mine["tags"][0], "t");

    a.run("snippet.delete", json!({"name":"mine"}))
        .expect("delete");
    let mut c = Session::new();
    let out = c.run("snippet.list", json!({})).expect("list");
    assert!(out["snippets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["name"] != "mine"));
}

#[test]
fn remember_persists_history_and_history_reads_it_back() {
    let _s = Scratch::new();
    let mut a = Session::new();
    a.remember(HistoryEntry {
        conn: "local".into(),
        sql: "select 1".into(),
        ok: true,
        elapsed_ms: 2.5,
        rows: 1,
        at: 100,
    });
    let mut b = Session::new();
    let out = b.run("sql.history", json!({"limit": 10})).expect("history");
    let entries = out["history"].as_array().expect("history array");
    let mine = entries
        .iter()
        .find(|h| h["sql"] == "select 1")
        .expect("entry present");
    assert_eq!(mine["conn"], "local");
    assert_eq!(mine["ok"], true);
    b.run("sql.clearHistory", json!({})).expect("clear");
    let mut c = Session::new();
    let out = c.run("sql.history", json!({})).expect("history");
    assert_eq!(out["total"], 0);
}

#[test]
fn state_persist_then_restore_rebuilds_tabs() {
    let _s = Scratch::new();
    let mut a = Session::new();
    a.run("tab.new", json!({})).expect("open");
    a.run("tab.rename", json!({"title":"Scratch"}))
        .expect("rename");
    a.run("tab.setEditorText", json!({"sql":"select 7"}))
        .expect("set text");
    a.run("state.persist", json!({})).expect("persist");
    let mut b = Session::new();
    let out = b.run("state.restore", json!({})).expect("restore");
    let tabs = out["tabs"].as_array().expect("tabs array");
    assert!(
        tabs.len() >= 2,
        "welcome plus the restored query tab: {tabs:?}"
    );
    let scratch = tabs
        .iter()
        .find(|t| t["title"] == "Scratch")
        .expect("restored tab");
    assert_eq!(scratch["sql"], "select 7");
}

#[test]
fn a_tab_whose_connection_vanished_restores_without_a_connection() {
    let _s = Scratch::new();
    let mut a = Session::new();
    a.run(
        "state.persist",
        json!({"tabs":[
        {"kind":"Query","title":"Orphan","conn":"ghost","sql":"select 1"}
    ],"active":0}),
    )
    .expect("persist");
    let mut b = Session::new();
    let out = b
        .run("state.restore", json!({}))
        .expect("restore must not fail");
    let tabs = out["tabs"].as_array().expect("tabs");
    let orphan = tabs
        .iter()
        .find(|t| t["title"] == "Orphan")
        .expect("kept, not dropped");
    assert_eq!(orphan["conn"], serde_json::Value::Null);
    // The rule is a named warning, never a silent drop.
    assert!(
        b.messages.iter().any(|m| m.text.contains("ghost")),
        "expected a warning naming the missing connection: {:?}",
        b.messages
    );
}

#[test]
fn conn_pin_and_reorder_round_trip_through_the_store() {
    let _s = Scratch::new();
    let mut a = Session::new();
    a.run(
        "conn.new",
        json!({"name":"local","kind":"sqlite","file":"/tmp/nook-pin-test.db"}),
    )
    .expect("create profile");
    a.run("conn.pin", json!({"conn":"local","pinned":true}))
        .expect("pin");
    let b = Session::new();
    assert_eq!(b.pinned(), &["local".to_string()]);
    a.run("conn.pin", json!({"conn":"local","pinned":false}))
        .expect("unpin");
    let c = Session::new();
    assert!(c.pinned().is_empty());
}
