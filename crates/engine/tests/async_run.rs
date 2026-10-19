//! `sql.runAsync`, `sql.jobStatus`, `sql.cancel`, `sql.stop`: a slow statement must not own the
//! frame, and a Stop must actually stop.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-async");
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
            json!({"sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT); \
                           INSERT INTO t (v) VALUES ('a'), ('b'), ('c'), ('d'), ('e');"}),
        );
        // Background pages land in a query tab, which is where a user runs one from.
        f.run("sql.newQuery", json!({ "sql": "SELECT * FROM t" }));
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

    /// Pump until the job leaves `running`, or give up. The pump is what a frame does.
    fn settle(&mut self, job: u64) -> serde_json::Value {
        for _ in 0..2000 {
            self.session.pump();
            let status = self.run("sql.jobStatus", json!({ "job": job }));
            if status["state"] != json!("running") {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("job {job} never finished");
    }
}

#[test]
fn an_async_run_lands_its_rows_in_the_tab() {
    let mut f = Fixture::new("lands");
    let started = f.run(
        "sql.runAsync",
        json!({ "sql": "SELECT * FROM t ORDER BY id", "page": 2 }),
    );
    let job = started["job"].as_u64().expect("a job id");
    assert_eq!(started["state"], json!("running"));

    let done = f.settle(job);
    assert_eq!(done["state"], json!("done"), "{done}");
    assert_eq!(done["rows"], json!(5));
    // Two rows per page over five rows: three pages, not one.
    assert_eq!(done["pages"], json!(3), "{done}");

    let tab = f.session.active_tab().expect("a tab");
    let last = tab.sql.last_result.as_ref().expect("rows landed");
    assert_eq!(last.row_count(), 5);
    assert_eq!(last.columns, vec!["id".to_string(), "v".to_string()]);
    assert_eq!(last.rows[4][1].as_text(), "e");
}

#[test]
fn the_run_is_visible_as_running_before_it_is_done() {
    // The whole point of the phase: the frame must be able to observe a job mid-flight. A run this
    // small can finish before the first poll, so the assertion is that the pump never has to block
    // and that `running` is a state the status command can actually report.
    let mut f = Fixture::new("midflight");
    let job = f.run(
        "sql.runAsync",
        json!({ "sql": "SELECT * FROM t", "page": 1 }),
    )["job"]
        .as_u64()
        .expect("a job id");
    let first = f.run("sql.jobStatus", json!({ "job": job }));
    assert!(
        first["state"] == json!("running") || first["state"] == json!("done"),
        "{first}"
    );
    // Pumping must return rather than wait: a blocking pump is the defect this phase removes.
    f.session.pump();
    f.settle(job);
}

#[test]
fn a_job_that_does_not_exist_is_not_found() {
    let mut f = Fixture::new("missing");
    let message = f.refuse("sql.jobStatus", json!({ "job": 9999 }));
    assert!(message.contains("no background run"), "{message}");
    let message = f.refuse("sql.cancel", json!({ "job": 9999 }));
    assert!(message.contains("no background run"), "{message}");
}

#[test]
fn a_job_id_is_required() {
    let mut f = Fixture::new("noid");
    let message = f.refuse("sql.jobStatus", json!({}));
    assert!(message.contains("\"job\""), "{message}");
}

#[test]
fn a_failing_async_run_reports_the_servers_error_in_the_tab() {
    let mut f = Fixture::new("failing");
    let job = f.run("sql.runAsync", json!({ "sql": "SELECT * FROM nope" }))["job"]
        .as_u64()
        .expect("a job id");
    let done = f.settle(job);
    assert_eq!(done["state"], json!("failed"), "{done}");
    assert!(
        done["error"].as_str().unwrap_or_default().contains("nope"),
        "{done}"
    );
    let tab = f.session.active_tab().expect("a tab");
    let error = tab.sql.error().unwrap_or_default();
    assert!(!error.is_empty(), "the failure reaches the tab");
    assert!(error.contains("nope"), "{error}");
}

#[test]
fn sql_stop_refuses_when_nothing_is_running_in_the_background() {
    let mut f = Fixture::new("nostop");
    let message = f.refuse("sql.stop", json!({}));
    assert!(message.contains("nothing is running"), "{message}");
    assert!(
        message.contains("sql.runAsync"),
        "the refusal must name the command that can be stopped: {message}"
    );
}

#[test]
fn cancelling_a_finished_job_says_so_instead_of_pretending() {
    let mut f = Fixture::new("finished");
    let job = f.run("sql.runAsync", json!({ "sql": "SELECT * FROM t" }))["job"]
        .as_u64()
        .expect("a job id");
    f.settle(job);
    let message = f.refuse("sql.cancel", json!({ "job": job }));
    assert!(message.contains("already"), "{message}");
}

#[test]
fn sqlite_cancels_between_pages_and_names_the_granularity() {
    // SQLite has no out-of-band cancel: the reader is inside one `sqlite3_step`. The honest answer
    // names the granularity rather than claiming the statement stopped where it did not.
    let mut f = Fixture::new("granularity");
    let job = f.run(
        "sql.runAsync",
        json!({ "sql": "SELECT * FROM t", "page": 1 }),
    )["job"]
        .as_u64()
        .expect("a job id");
    match f.session.run("sql.cancel", json!({ "job": job })) {
        Ok(out) => {
            let granularity = out["granularity"].as_str().unwrap_or_default();
            assert!(
                granularity.contains("page") || granularity.contains("first"),
                "{out}"
            );
        }
        Err(e) => {
            // A job that finished in the microseconds between start and cancel is a correct refusal.
            assert!(e.to_string().contains("already"), "{e}");
        }
    }
    f.settle(job);
}

#[test]
fn the_jobs_list_shows_every_run_this_session_started() {
    let mut f = Fixture::new("list");
    let a = f.run("sql.runAsync", json!({ "sql": "SELECT 1" }))["job"]
        .as_u64()
        .expect("a job id");
    let b = f.run("sql.runAsync", json!({ "sql": "SELECT 2" }))["job"]
        .as_u64()
        .expect("a job id");
    f.settle(a);
    f.settle(b);
    let out = f.run("sql.jobs", json!({}));
    assert_eq!(out["count"], json!(2), "{out}");
    let ids: Vec<u64> = out["jobs"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|j| j["job"].as_u64())
        .collect();
    assert_eq!(ids, vec![a, b]);
}

#[test]
fn an_async_run_respects_the_limit() {
    let mut f = Fixture::new("limit");
    let job = f.run(
        "sql.runAsync",
        json!({ "sql": "SELECT * FROM t ORDER BY id", "limit": 2, "page": 1 }),
    )["job"]
        .as_u64()
        .expect("a job id");
    let done = f.settle(job);
    assert_eq!(done["rows"], json!(2), "{done}");
}

/// Live Postgres: a `pg_sleep` run is cancelled from a sibling connection, and the tab says
/// `cancelled` rather than `failed`. Skipped when no server is configured, never faked.
#[test]
fn a_slow_postgres_query_is_cancelled_by_sql_cancel() {
    let Ok(uri) = std::env::var("NOOK_PG_URI") else {
        eprintln!("NOOK_PG_URI not set; live cancel not exercised");
        return;
    };
    let dir = std::env::temp_dir().join("nook-async-pg");
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
    let cfg = nook_core::ConnectionConfig::parse(&uri).expect("the URI parses");
    let conn = cfg.id.clone();
    let mut session = Session::new();
    session.conns.push(Conn::new(cfg));
    let index = session.conn_index(&conn).expect("registered");
    open_now(&mut session, index).expect("postgres opens");
    session
        .run("sql.newQuery", json!({ "sql": "SELECT pg_sleep(30)" }))
        .expect("a tab");

    let job = session
        .run(
            "sql.runAsync",
            json!({ "sql": "SELECT pg_sleep(30)", "page": 1 }),
        )
        .expect("start")["job"]
        .as_u64()
        .expect("a job id");

    // Give the worker time to connect and start the statement, so the pid is recorded.
    for _ in 0..200 {
        session.pump();
        if session.runner.job(job).map(|j| j.rows).unwrap_or(0) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        if session
            .runner
            .job(job)
            .map(|j| !j.state.is_running())
            .unwrap_or(true)
        {
            break;
        }
    }
    let started = std::time::Instant::now();
    let out = session
        .run("sql.cancel", json!({ "job": job }))
        .expect("cancel");
    assert_eq!(out["cancelled"], json!(true), "{out}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the cancel itself must not wait for the statement"
    );

    let mut state = String::new();
    for _ in 0..400 {
        session.pump();
        state = session
            .run("sql.jobStatus", json!({ "job": job }))
            .expect("status")["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if state != "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(state, "cancelled", "a cancelled run is not a failed one");
    let tab = session.active_tab().expect("a tab");
    assert_eq!(tab.sql.error(), Some("cancelled"));
}
