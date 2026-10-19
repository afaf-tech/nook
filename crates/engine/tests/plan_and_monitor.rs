//! `sql.explainVisual`, `sql.profiler`, `monitor.locks`, `monitor.replication`, `monitor.alerts`.
//!
//! The lock and replication queries are engine-specific SQL that only a live server can prove, so
//! what is asserted here is the part that holds on any engine: the shape of the answer, and the
//! refusal on an engine that has no such concept. The live Postgres assertions are behind
//! `NOOK_PG_URI` and are skipped, not faked, when it is not set.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-plan");
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
        let path = dir.join(format!("{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", path.display()))
            .expect("a sqlite URI parses");
        let conn = cfg.id.clone();
        let mut session = Session::new();
        session.conns.push(Conn::new(cfg));
        let index = session.conn_index(&conn).expect("registered");
        open_now(&mut session, index).expect("sqlite opens");
        let mut f = Fixture { session, conn };
        f.run(
            "sql.run",
            json!({"sql": "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT); \
                           CREATE INDEX users_email ON users (email); \
                           INSERT INTO users (email) VALUES ('a@b.c'), ('d@e.f');"}),
        );
        f.run("sql.newQuery", json!({ "sql": "SELECT * FROM users WHERE email = 'a@b.c'" }));
        f
    }

    fn run(&mut self, id: &str, mut params: serde_json::Value) -> serde_json::Value {
        params["conn"] = json!(self.conn);
        self.session
            .run(id, params)
            .unwrap_or_else(|e| panic!("{id} failed: {e}"))
    }

    fn refuse(&mut self, id: &str, mut params: serde_json::Value) -> String {
        params["conn"] = json!(self.conn);
        match self.session.run(id, params) {
            Ok(v) => panic!("{id} should have refused, returned {v}"),
            Err(e) => e.to_string(),
        }
    }
}

#[test]
fn a_visual_explain_returns_a_tree_with_a_root() {
    let mut f = Fixture::new("visual");
    let out = f.run("sql.explainVisual", json!({ "sql": "SELECT * FROM users WHERE email = 'a@b.c'" }));
    assert_eq!(out["root"], json!(0), "{out}");
    assert!(!out["nodes"].as_array().expect("nodes").is_empty(), "{out}");
    assert_eq!(out["nodes_total"], json!(1));
    let node = &out["nodes"][0];
    assert_eq!(node["label"], json!("SEARCH"), "SQLite says SEARCH when it uses the index: {out}");
    assert!(
        node["detail"].as_str().unwrap_or_default().contains("users_email"),
        "{out}"
    );
}

#[test]
fn a_visual_explain_puts_the_plan_on_the_tab() {
    let mut f = Fixture::new("tabplan");
    f.run("sql.explainVisual", json!({ "sql": "SELECT * FROM users" }));
    let tab = f.session.active_tab().expect("a tab");
    let plan = tab.sql.plan.as_ref().expect("the plan is on the tab");
    assert!(plan["nodes"].as_array().is_some_and(|n| !n.is_empty()));
}

#[test]
fn an_analyzed_write_is_refused_before_it_changes_the_data() {
    // `EXPLAIN ANALYZE` executes. On a write that is a data incident, not an explain.
    let mut f = Fixture::new("analyze-write");
    let message = f.refuse(
        "sql.explainVisual",
        json!({ "sql": "UPDATE users SET email = 'x'", "analyze": true }),
    );
    assert!(message.contains("would change the data"), "{message}");
}

#[test]
fn a_profiler_runs_a_read_and_reports_the_spread() {
    let mut f = Fixture::new("profile");
    let out = f.run(
        "sql.profiler",
        json!({ "sql": "SELECT * FROM users", "runs": 5 }),
    );
    assert_eq!(out["count"], json!(5), "{out}");
    assert_eq!(out["rows"], json!(2));
    assert_eq!(out["runs"].as_array().expect("timings").len(), 5);
    let min = out["min_ms"].as_f64().expect("min");
    let max = out["max_ms"].as_f64().expect("max");
    let median = out["median_ms"].as_f64().expect("median");
    assert!(min <= median && median <= max, "{out}");
    assert!(out["varied"].is_boolean());
    assert!(out["note"].as_str().is_some_and(|n| !n.is_empty()));
}

#[test]
fn a_profiler_refuses_a_write() {
    let mut f = Fixture::new("profile-write");
    let message = f.refuse(
        "sql.profiler",
        json!({ "sql": "DELETE FROM users", "runs": 5 }),
    );
    assert!(message.contains("not read-only"), "{message}");
    // And the rows are still there: a refusal that ran the statement first is not a refusal.
    let out = f.run("sql.run", json!({ "sql": "SELECT COUNT(*) FROM users" }));
    assert_eq!(out["data"][0][0].as_i64(), Some(2));
}

#[test]
fn a_profiler_caps_the_run_count() {
    let mut f = Fixture::new("profile-cap");
    let out = f.run(
        "sql.profiler",
        json!({ "sql": "SELECT 1", "runs": 500 }),
    );
    assert_eq!(out["count"], json!(50), "the cap is 50: {out}");
}

#[test]
fn locks_on_sqlite_refuse_instead_of_returning_an_empty_table() {
    // An empty lock table reads as "nothing is blocked", which is a lie a file database cannot tell.
    let mut f = Fixture::new("locks");
    let message = f.refuse("monitor.locks", json!({}));
    assert!(message.contains("no server-level locks"), "{message}");
}

#[test]
fn replication_on_sqlite_refuses_instead_of_returning_an_empty_list() {
    let mut f = Fixture::new("replication");
    let message = f.refuse("monitor.replication", json!({}));
    assert!(message.contains("no replication"), "{message}");
}

#[test]
fn an_alert_rule_over_a_real_metric_reports_whether_it_tripped() {
    let mut f = Fixture::new("alerts");
    // `monitor.status` on SQLite reports `page_count` and friends; a rule over a metric that exists
    // must evaluate, and one over a metric that does not must refuse rather than never fire.
    let status = f.run("monitor.status", json!({}));
    let first = status["status"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| e["name"].as_str())
        .expect("at least one metric")
        .to_string();
    let out = f.run(
        "monitor.alerts",
        json!({ "rules": [{ "metric": first, "op": ">=", "value": 0 }] }),
    );
    assert_eq!(out["rules"].as_array().expect("rules").len(), 1, "{out}");
    assert_eq!(out["tripped"], json!(1), "{out}");
}

#[test]
fn an_alert_rule_naming_a_metric_that_does_not_exist_refuses() {
    let mut f = Fixture::new("alerts-typo");
    let message = f.refuse(
        "monitor.alerts",
        json!({ "rules": [{ "metric": "nonesuch", "op": ">", "value": 1 }] }),
    );
    assert!(message.contains("no metric named"), "{message}");
}

#[test]
fn an_alert_rule_with_an_unknown_operator_refuses() {
    let mut f = Fixture::new("alerts-op");
    let status = f.run("monitor.status", json!({}));
    let first = status["status"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| e["name"].as_str())
        .expect("at least one metric")
        .to_string();
    let message = f.refuse(
        "monitor.alerts",
        json!({ "rules": [{ "metric": first, "op": "~~", "value": 1 }] }),
    );
    assert!(message.contains("unknown operator"), "{message}");
}

#[test]
fn alerts_without_rules_refuse_and_name_the_shape() {
    let mut f = Fixture::new("alerts-empty");
    let message = f.refuse("monitor.alerts", json!({}));
    assert!(message.contains("rules"), "{message}");
}

/// A tripped alert reaches the endpoint a failed job reaches, in that job's shape. The fake curl
/// records the body it was handed, so the assertion is on what was actually posted.
#[test]
fn a_tripped_alert_posts_the_same_shape_a_failed_job_does() {
    let mut f = Fixture::new("alert-webhook");
    let dir = std::env::temp_dir().join("nook-plan");
    let _ = std::fs::create_dir_all(&dir);
    let capture = dir.join(format!("alert-body-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&capture);
    // The fake curl: read the body from stdin and write it where the test can see it.
    let fake = dir.join(format!("fake-curl-{}.sh", std::process::id()));
    std::fs::write(
        &fake,
        format!("#!/bin/sh\ncat > '{}'\n", capture.display()),
    )
    .expect("the fake curl is written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("the fake curl is executable");
    }
    std::env::set_var("NOOK_CURL_BIN", &fake);

    let status = f.run("monitor.status", json!({}));
    let first = status["status"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| e["name"].as_str())
        .expect("at least one metric")
        .to_string();
    let out = f.run(
        "monitor.alerts",
        json!({
            "rules": [{ "metric": first, "op": ">=", "value": 0 }],
            "webhook": "https://hooks.example.com/nook",
        }),
    );
    assert_eq!(out["tripped"], json!(1), "{out}");

    let body: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&capture).unwrap_or_else(|e| {
            panic!("the webhook was not called: {e}");
        }),
    )
    .expect("the posted body is JSON");
    // The same keys a failed job's notification carries.
    for key in ["job", "name", "ok", "error", "elapsed_ms", "at"] {
        assert!(body.get(key).is_some(), "missing {key}: {body}");
    }
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["alerts"].as_array().map(|a| a.len()), Some(1));
    std::env::remove_var("NOOK_CURL_BIN");
}

/// A rule that does not trip posts nothing: an endpoint that hears from the monitor on a quiet day
/// is an endpoint nobody trusts.
#[test]
fn an_alert_that_does_not_trip_posts_nothing() {
    let mut f = Fixture::new("alert-quiet");
    let dir = std::env::temp_dir().join("nook-plan");
    let capture = dir.join(format!("quiet-body-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&capture);
    let fake = dir.join(format!("fake-curl-quiet-{}.sh", std::process::id()));
    std::fs::write(
        &fake,
        format!("#!/bin/sh\ncat > '{}'\n", capture.display()),
    )
    .expect("the fake curl is written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("the fake curl is executable");
    }
    std::env::set_var("NOOK_CURL_BIN", &fake);

    let status = f.run("monitor.status", json!({}));
    let first = status["status"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| e["name"].as_str())
        .expect("at least one metric")
        .to_string();
    let out = f.run(
        "monitor.alerts",
        json!({
            "rules": [{ "metric": first, "op": "<", "value": -1.0 }],
            "webhook": "https://hooks.example.com/nook",
        }),
    );
    assert_eq!(out["tripped"], json!(0), "{out}");
    assert!(!capture.exists(), "a quiet alert posted anyway");
    std::env::remove_var("NOOK_CURL_BIN");
}
