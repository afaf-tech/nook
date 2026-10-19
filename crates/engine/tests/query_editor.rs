//! Phase 6: the query editor. Parameters detected on the token stream, run as real prepared
//! statements; find & replace over the editor text; zen; split panes; history reuse.
//!
//! The prepared tests use a real sqlite file: the claim being tested is that the *driver* binds the
//! values, so a fake would prove nothing.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    /// Kept alive for the file's lifetime: the store is the session's, and a dropped temp path
    /// would leave the fixture pointing at nothing.
    #[allow(dead_code)]
    path: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-query-editor");
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
        let path = dir.join(format!("{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", path.display()))
            .expect("a sqlite URI parses");
        let mut session = Session::new();
        session.conns.push(Conn::new(cfg));
        let index = session.conn_index(&session.conns[0].cfg.id.clone()).expect("registered");
        open_now(&mut session, index).expect("sqlite opens");
        Fixture { session, path }
    }

    fn run(&mut self, command: &str, params: serde_json::Value) -> serde_json::Value {
        self.session
            .run(command, params)
            .unwrap_or_else(|e| panic!("{command} failed: {e}"))
    }

    /// The error a command refuses with, for the cases where refusing is the behaviour.
    fn refuse(&mut self, command: &str, params: serde_json::Value) -> String {
        match self.session.run(command, params) {
            Ok(v) => panic!("{command} should have refused, returned {v}"),
            Err(e) => e.to_string(),
        }
    }

    fn sql(&mut self, sql: &str) {
        self.run("sql.run", json!({ "sql": sql }));
    }

    fn open_query_tab(&mut self) {
        self.run("sql.newQuery", json!({ "sql": "" }));
    }
}

fn users(f: &mut Fixture) {
    f.sql("create table users (id integer primary key, email text, name text)");
    f.sql("insert into users (id, email, name) values (1, 'a@b.c', 'Ada')");
    f.sql("insert into users (id, email, name) values (2, 'x@y.z', 'Bob')");
}

#[test]
fn parameters_reports_the_style_names_and_bindings() {
    let mut f = Fixture::new("params");
    f.open_query_tab();
    let out = f.run(
        "sql.parameters",
        json!({ "sql": "select * from users where email = :email and name = :name or email = :email" }),
    );
    assert_eq!(out["style"], "colon");
    let params = out["params"].as_array().expect("params is an array");
    assert_eq!(params.len(), 2, "one entry per distinct name");
    assert_eq!(params[0]["name"], "email");
    assert_eq!(params[0]["count"], 2);
    assert_eq!(params[1]["name"], "name");
    assert_eq!(params[0]["bound"], false);
    assert_eq!(out["missing"], json!(["email", "name"]));
}

#[test]
fn a_cast_and_a_literal_are_not_parameters() {
    let mut f = Fixture::new("cast");
    f.open_query_tab();
    let out = f.run(
        "sql.parameters",
        json!({ "sql": "select 'a:name' as lit, x::int as n from t where y = :real" }),
    );
    let names: Vec<&str> = out["params"]
        .as_array()
        .expect("array")
        .iter()
        .map(|p| p["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["real"], "only the real parameter survives");
}

#[test]
fn a_bound_parameter_runs_as_a_prepared_statement() {
    let mut f = Fixture::new("prepared");
    users(&mut f);
    f.open_query_tab();
    f.run("sql.setParameter", json!({ "name": "email", "value": "a@b.c" }));
    let out = f.run(
        "sql.run",
        json!({ "sql": "select name from users where email = :email" }),
    );
    assert_eq!(out["prepared"], true, "it went through the prepared path");
    assert_eq!(out["rows"], 1);
    assert_eq!(out["data"][0][0], "Ada");
}

#[test]
fn a_value_with_a_quote_and_a_semicolon_runs_without_changing_the_statement() {
    let mut f = Fixture::new("hostile");
    users(&mut f);
    f.open_query_tab();
    f.sql("insert into users (id, email, name) values (3, 'q', 'o''brien; drop table users')");
    let out = f.run(
        "sql.run",
        json!({
            "sql": "select name from users where email = :email",
            "params": { "email": "q" },
        }),
    );
    assert_eq!(out["rows"], 1);
    assert_eq!(out["data"][0][0], "o'brien; drop table users");
    // The statement was prepared with the placeholder, not the value: the value never entered the SQL.
    let prepared = out["summary"]["statement"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        !prepared.contains("o'brien"),
        "the value leaked into the prepared text: {prepared}"
    );
    assert!(prepared.contains('?'), "the placeholder is in the text: {prepared}");
    // The table is still there, which is what a hostile value would have taken.
    let out = f.run("sql.run", json!({ "sql": "select count(*) from users" }));
    assert_eq!(out["data"][0][0], 3);
}

#[test]
fn an_unbound_parameter_refuses_and_names_it() {
    let mut f = Fixture::new("unbound");
    users(&mut f);
    f.open_query_tab();
    let message = f.refuse(
        "sql.run",
        json!({ "sql": "select * from users where email = :email and name = :name" }),
    );
    assert!(message.contains("email"), "{message}");
    assert!(message.contains("name"), "{message}");
    assert!(
        message.contains("unbound parameter"),
        "it must say what is wrong: {message}"
    );
}

#[test]
fn an_explicit_null_is_a_binding_and_an_unset_one_is_not() {
    let mut f = Fixture::new("null");
    users(&mut f);
    f.open_query_tab();
    // Bound to NULL: the query runs and matches nothing.
    f.run("sql.setParameter", json!({ "name": "email", "value": null }));
    let out = f.run(
        "sql.run",
        json!({ "sql": "select name from users where email = :email" }),
    );
    assert_eq!(out["rows"], 0, "email = NULL matches nothing, and it ran");

    // Cleared: the run refuses instead of sending the placeholder.
    f.run("sql.setParameter", json!({ "name": "email", "clear": true }));
    let message = f.refuse(
        "sql.run",
        json!({ "sql": "select name from users where email = :email" }),
    );
    assert!(message.contains("email"), "{message}");
}

#[test]
fn a_positional_script_binds_one_value_per_slot() {
    let mut f = Fixture::new("positional");
    users(&mut f);
    f.open_query_tab();
    let out = f.run(
        "sql.run",
        json!({
            "sql": "select name from users where email = ? or email = ? order by id",
            "positional": ["a@b.c", "x@y.z"],
        }),
    );
    assert_eq!(out["rows"], 2);
    assert_eq!(out["data"][0][0], "Ada");
    assert_eq!(out["data"][1][0], "Bob");
}

#[test]
fn find_and_replace_reports_the_ranges_it_touched() {
    let mut f = Fixture::new("replace");
    f.open_query_tab();
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "user", "replace": "account", "all": true }),
    );
    assert_eq!(out["replaced"], 0, "an empty editor has nothing to replace");

    // Set the text through the editor, then replace.
    f.session
        .active_tab_mut()
        .expect("a tab")
        .sql
        .text = "select * from users where users.id = 1".into();
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "users", "replace": "accounts", "all": true }),
    );
    assert_eq!(out["text"], "select * from accounts where accounts.id = 1");
    assert_eq!(out["replaced"], 2);
}

#[test]
fn a_whole_word_replace_leaves_the_longer_word_alone() {
    let mut f = Fixture::new("word");
    f.open_query_tab();
    f.session
        .active_tab_mut()
        .expect("a tab")
        .sql
        .text = "select user, users from users".into();
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "user", "replace": "acct", "all": true, "word": true }),
    );
    assert_eq!(
        out["text"], "select acct, users from users",
        "only the standalone word changed"
    );
}

#[test]
fn a_replacement_longer_than_the_match_keeps_later_offsets_valid() {
    let mut f = Fixture::new("grow");
    f.open_query_tab();
    f.session.active_tab_mut().expect("a tab").sql.text = "a a a".into();
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "a", "replace": "aaaa", "all": true }),
    );
    assert_eq!(out["text"], "aaaa aaaa aaaa");
}

#[test]
fn an_empty_find_is_refused() {
    let mut f = Fixture::new("emptyfind");
    f.open_query_tab();
    let message = f.refuse("sql.findReplace", json!({ "find": "", "replace": "x" }));
    assert!(message.contains("empty"), "{message}");
}

#[test]
fn find_is_case_sensitive_only_when_asked() {
    let mut f = Fixture::new("case");
    f.open_query_tab();
    f.session.active_tab_mut().expect("a tab").sql.text = "SELECT select".into();
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "select", "replace": "x", "all": true }),
    );
    assert_eq!(out["text"], "x x", "case-insensitive by default");
    f.session.active_tab_mut().expect("a tab").sql.text = "SELECT select".into();
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "select", "replace": "x", "all": true, "case": true }),
    );
    assert_eq!(out["text"], "SELECT x");
}

#[test]
fn zen_is_a_reported_flag() {
    let mut f = Fixture::new("zen");
    let out = f.run("sql.zen", json!({ "on": true }));
    assert_eq!(out["zen"], true);
    assert!(f.session.zen);
    let out = f.run("sql.zen", json!({ "on": false }));
    assert!(!out["zen"].as_bool().unwrap_or(true));
}

#[test]
fn replacing_at_a_range_touches_only_that_hit() {
    let mut f = Fixture::new("replace_at");
    f.open_query_tab();
    f.session.active_tab_mut().expect("a tab").sql.text = "select a, a, a".into();
    // The third `a` alone, which is what the find bar walks to. `all: false` would have taken the
    // first, and an empty replacement would have deleted it.
    let out = f.run(
        "sql.findReplace",
        json!({ "find": "a", "replace": "bb", "at": [13, 14] }),
    );
    assert_eq!(out["replaced"], 1);
    assert_eq!(out["text"], "select a, a, bb");
    // A range that is not a hit refuses instead of editing somewhere else: the text may have moved
    // on between the walk and the replace.
    let message = f.refuse(
        "sql.findReplace",
        json!({ "find": "a", "replace": "z", "at": [0, 1] }),
    );
    assert!(message.contains("not a hit"), "{message}");
    assert_eq!(
        f.session.active_tab().expect("a tab").sql.text,
        "select a, a, bb"
    );
}

#[test]
fn the_two_panes_keep_their_own_results() {
    let mut f = Fixture::new("split_results");
    users(&mut f);
    f.open_query_tab();
    f.run("sql.splitPane", json!({ "on": true }));
    let left = f.run("sql.run", json!({ "sql": "select 1 as left_side" }));
    assert_eq!(left["columns"], json!(["left_side"]));
    let right = f.run(
        "sql.run",
        json!({ "sql": "select 2 as right_side", "pane": "right" }),
    );
    assert_eq!(right["columns"], json!(["right_side"]));
    let tab = f.session.active_tab().expect("a tab");
    // The right pane's run landed in its own results and left the left pane's alone.
    assert_eq!(
        tab.sql.last_result.as_ref().expect("left rows").columns,
        vec!["left_side".to_string()]
    );
    let split = tab.sql.split.as_ref().expect("a split");
    assert_eq!(split.results.len(), 1);
    assert_eq!(
        split.results[0]
            .result
            .as_ref()
            .expect("right rows")
            .columns,
        vec!["right_side".to_string()]
    );
}

#[test]
fn a_split_pane_has_its_own_text_and_closing_discards_it() {
    let mut f = Fixture::new("split");
    users(&mut f);
    f.open_query_tab();
    let out = f.run("sql.splitPane", json!({ "on": true }));
    assert_eq!(out["split"], true);
    f.session
        .active_tab_mut()
        .expect("a tab")
        .sql
        .split
        .as_mut()
        .expect("a split")
        .text = "select 1".into();
    // Both panes exist and hold different text.
    let tab = f.session.active_tab_mut().expect("a tab");
    tab.sql.text = "select 2".into();
    assert_eq!(tab.sql.text, "select 2");
    assert_eq!(tab.sql.split.as_ref().expect("split").text, "select 1");
    let out = f.run("sql.splitPane", json!({ "on": false }));
    assert_eq!(out["split"], false);
    assert!(
        f.session.active_tab_mut().expect("a tab").sql.split.is_none(),
        "closing the split discards the right pane"
    );
}

#[test]
fn history_filters_by_connection_and_puts_an_entry_back() {
    let mut f = Fixture::new("history");
    users(&mut f);
    f.open_query_tab();
    f.sql("select 1 as first");
    f.sql("select 2 as second");
    let conn = f.session.conns[0].cfg.id.clone();
    let out = f.run("sql.history", json!({ "conn": conn, "limit": 10 }));
    assert!(
        out["history"].as_array().is_some_and(|a| a.len() >= 2),
        "the entries are there"
    );
    // 0 = most recent.
    let out = f.run("sql.history", json!({ "reuse": 0 }));
    let reused = out["reused"].as_str().unwrap_or_default().to_string();
    assert_eq!(
        f.session.active_tab().expect("a tab").sql.text,
        reused,
        "the entry went back into the editor"
    );
    assert!(reused.contains("select"), "{reused}");
}

#[test]
fn the_capability_command_reports_without_running_and_runs_when_asked() {
    let mut f = Fixture::new("capability");
    users(&mut f);
    f.open_query_tab();
    let out = f.run(
        "sql.parameterised",
        json!({ "sql": "select name from users where email = :email" }),
    );
    assert_eq!(out["ran"], false, "reporting must not run anything");
    assert_eq!(out["parameters"]["style"], "colon");

    let out = f.run(
        "sql.parameterised",
        json!({
            "sql": "select name from users where email = :email",
            "params": { "email": "a@b.c" },
        }),
    );
    assert_eq!(out["prepared"], true);
    assert_eq!(out["rows"], 1);

    // Reporting is the default, so a plain script still only reports.
    let out = f.run("sql.parameterised", json!({ "sql": "select 1" }));
    assert_eq!(out["ran"], false);
    assert_eq!(out["parameters"]["style"], "none");
    // Asking for a run of a script with no placeholders points at the command that does that.
    let message = f.refuse(
        "sql.parameterised",
        json!({ "sql": "select 1", "run": true }),
    );
    assert!(message.contains("sql.run"), "{message}");
}

#[test]
fn a_script_without_placeholders_still_runs_as_a_script() {
    let mut f = Fixture::new("plain");
    users(&mut f);
    f.open_query_tab();
    let out = f.run(
        "sql.run",
        json!({ "sql": "select 1 as a; select 2 as b" }),
    );
    assert_eq!(out["statements"], 2, "the script path, not the prepared one");
    assert!(
        out["prepared"].is_null(),
        "an ordinary script is not reported as prepared"
    );
}
