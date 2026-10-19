//! `schema.partitions`, `schema.createPartition`, `schema.dropPartition`.
//!
//! The Postgres cases run against a live server (`NOOK_PG_URI`) and skip without one: a partition
//! bound is the server's own expression, so a mock would prove nothing about the catalog read.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

fn live_pg() -> Option<Session> {
    let uri = std::env::var("NOOK_PG_URI")
        .ok()
        .filter(|s| !s.is_empty())?;
    let cfg = nook_core::ConnectionConfig::parse(&uri).ok()?;
    let mut session = Session::new();
    session.conns.push(Conn::new(cfg));
    let index = session.conn_index(&session.conns[0].cfg.id.clone()).ok()?;
    open_now(&mut session, index).ok()?;
    Some(session)
}

fn run(session: &mut Session, id: &str, mut params: serde_json::Value) -> serde_json::Value {
    params["conn"] = json!(session.conns[0].cfg.id);
    session
        .run(id, params)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn refuse(session: &mut Session, id: &str, mut params: serde_json::Value) -> String {
    params["conn"] = json!(session.conns[0].cfg.id);
    match session.run(id, params) {
        Ok(v) => panic!("{id} should have refused, returned {v}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_partitioned_table_lists_its_partitions_with_bounds() {
    let Some(mut s) = live_pg() else {
        eprintln!("skipped: NOOK_PG_URI is not set");
        return;
    };
    run(
        &mut s,
        "sql.run",
        json!({"sql": "DROP TABLE IF EXISTS p7_events CASCADE; \
                       CREATE TABLE p7_events (at date NOT NULL, kind text) PARTITION BY RANGE (at);"}),
    );
    run(
        &mut s,
        "schema.createPartition",
        json!({"parent": "p7_events", "name": "p7_events_2026_11",
               "from": "2026-11-01", "to": "2026-12-01"}),
    );
    let out = run(&mut s, "schema.partitions", json!({"table": "p7_events"}));
    assert_eq!(out["partitioned"], true);
    let items = out["partitions"].as_array().expect("partitions");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "p7_events_2026_11");
    // The bound comes from `pg_get_expr`, so it is the server's own spelling, not a reconstruction.
    let bound = items[0]["bound"].as_str().unwrap_or_default();
    assert!(bound.contains("2026-11-01"), "{bound}");
    assert!(bound.contains("2026-12-01"), "{bound}");

    // And dropping it leaves the parent alone.
    run(
        &mut s,
        "schema.dropPartition",
        json!({"parent": "p7_events", "name": "p7_events_2026_11", "confirm": true}),
    );
    let after = run(&mut s, "schema.partitions", json!({"table": "p7_events"}));
    assert_eq!(after["partitions"].as_array().expect("partitions").len(), 0);
    run(
        &mut s,
        "sql.run",
        json!({"sql": "DROP TABLE IF EXISTS p7_events CASCADE;"}),
    );
}

#[test]
fn an_unpartitioned_table_answers_an_empty_list_with_a_reason() {
    let Some(mut s) = live_pg() else {
        eprintln!("skipped: NOOK_PG_URI is not set");
        return;
    };
    run(
        &mut s,
        "sql.run",
        json!({"sql": "DROP TABLE IF EXISTS p7_plain; CREATE TABLE p7_plain (id int);"}),
    );
    let out = run(&mut s, "schema.partitions", json!({"table": "p7_plain"}));
    assert_eq!(out["partitioned"], false);
    assert_eq!(out["partitions"], json!([]));
    assert!(
        out["note"]
            .as_str()
            .unwrap_or_default()
            .contains("not partitioned"),
        "{}",
        out["note"]
    );
    run(
        &mut s,
        "sql.run",
        json!({"sql": "DROP TABLE IF EXISTS p7_plain;"}),
    );
}

#[test]
fn a_bound_that_is_not_a_date_refuses_before_any_sql_runs() {
    let Some(mut s) = live_pg() else {
        eprintln!("skipped: NOOK_PG_URI is not set");
        return;
    };
    for (from, to, expected) in [
        ("2026-11-01", "2026-13-01", "is not a date"),
        ("2026-12-01", "2026-11-01", "the range is empty"),
        ("now()", "2026-11-01", "is not a date"),
    ] {
        let message = refuse(
            &mut s,
            "schema.createPartition",
            json!({"parent": "p7_never", "name": "p7_never_a", "from": from, "to": to}),
        );
        assert!(message.contains(expected), "{from}..{to} said: {message}");
    }
}

#[test]
fn dropping_a_partition_needs_confirm() {
    let Some(mut s) = live_pg() else {
        eprintln!("skipped: NOOK_PG_URI is not set");
        return;
    };
    run(
        &mut s,
        "sql.run",
        json!({"sql": "DROP TABLE IF EXISTS p7_gate CASCADE; \
                       CREATE TABLE p7_gate (at date NOT NULL) PARTITION BY RANGE (at);"}),
    );
    run(
        &mut s,
        "schema.createPartition",
        json!({"parent": "p7_gate", "name": "p7_gate_a", "from": "2026-01-01", "to": "2027-01-01"}),
    );
    let message = refuse(
        &mut s,
        "schema.dropPartition",
        json!({"parent": "p7_gate", "name": "p7_gate_a"}),
    );
    assert!(message.contains("confirm"), "{message}");
    let still = run(&mut s, "schema.partitions", json!({"table": "p7_gate"}));
    assert_eq!(still["partitions"].as_array().expect("partitions").len(), 1);
    run(
        &mut s,
        "sql.run",
        json!({"sql": "DROP TABLE IF EXISTS p7_gate CASCADE;"}),
    );
}
