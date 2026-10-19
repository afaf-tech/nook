//! Saved reports and boards: `report.save|list|delete|designer|biDashboard|board|export`,
//! `monitor.charts`, `metrics.input`.
//!
//! The store is the user's real `nook.db`, so every test points `NOOK_CONFIG_DIR` at a scratch
//! directory — a test that wrote a report into the developer's own store would be a bug.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-reports");
        let _ = std::fs::create_dir_all(&dir);
        // `NOOK_CONFIG_DIR` is process-global, so every test in this binary shares one store; names
        // are unique per test, which is what keeps them from reading each other's reports.
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
            json!({"sql": "CREATE TABLE sales (day TEXT, total REAL); \
                           INSERT INTO sales VALUES ('mon', 10.0), ('tue', 20.0), ('wed', 30.0);"}),
        );
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
fn a_saved_report_reads_back_with_its_chart_kind() {
    let mut f = Fixture::new("save");
    let out = f.run(
        "report.save",
        json!({"name": "a-saved-report-reads-back-with-its-chart-kind-daily", "sql": "select day, total from sales", "chart": "line", "label": "day"}),
    );
    assert_eq!(out["saved"], json!(true), "{out}");
    assert_eq!(out["report"]["chart"], json!("line"));
    assert_eq!(out["report"]["label"], json!("day"));

    let listed = f.run("report.list", json!({}));
    let names: Vec<&str> = listed["reports"]
        .as_array()
        .expect("reports")
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(
        names.contains(&"a-saved-report-reads-back-with-its-chart-kind-daily"),
        "{listed}"
    );
}

#[test]
fn saving_over_a_name_replaces_it_rather_than_adding_a_second() {
    let mut f = Fixture::new("replace");
    f.run("report.save", json!({"name": "saving-over-a-name-replaces-it-rather-than-adding-a-second-daily", "sql": "select 1", "chart": "table"}));
    f.run("report.save", json!({"name": "saving-over-a-name-replaces-it-rather-than-adding-a-second-daily", "sql": "select day, total from sales", "chart": "line"}));
    let listed = f.run("report.list", json!({}));
    let daily: Vec<&serde_json::Value> = listed["reports"]
        .as_array()
        .expect("reports")
        .iter()
        .filter(|r| {
            r["name"] == json!("saving-over-a-name-replaces-it-rather-than-adding-a-second-daily")
        })
        .collect();
    assert_eq!(daily.len(), 1, "one row per name: {listed}");
    assert_eq!(daily[0]["chart"], json!("line"));
}

#[test]
fn a_chart_kind_that_does_not_exist_is_refused_before_it_is_stored() {
    let mut f = Fixture::new("bad-chart");
    let message = f.refuse(
        "report.save",
        json!({"name": "bad", "sql": "select 1", "chart": "sankey"}),
    );
    assert!(message.contains("unknown chart"), "{message}");
    let listed = f.run("report.list", json!({}));
    assert!(
        !listed["reports"]
            .as_array()
            .expect("reports")
            .iter()
            .any(|r| r["name"] == json!("bad")),
        "a refused report was stored anyway: {listed}"
    );
}

#[test]
fn deleting_a_report_that_is_not_there_is_a_no_op() {
    let mut f = Fixture::new("delete");
    let out = f.run("report.delete", json!({"name": "nonesuch"}));
    assert_eq!(out["deleted"], json!(true), "{out}");
}

#[test]
fn a_board_refuses_a_tile_that_names_no_saved_report() {
    let mut f = Fixture::new("board-missing");
    let message = f.refuse(
        "report.biDashboard",
        json!({"name": "a-board-refuses-a-tile-that-names-no-saved-report-ops", "tiles": [{"report": "ghost", "w": 6, "h": 4}]}),
    );
    assert!(message.contains("no saved report named"), "{message}");
}

#[test]
fn a_board_refuses_a_tile_without_a_report_key() {
    let mut f = Fixture::new("board-shape");
    let message = f.refuse(
        "report.biDashboard",
        json!({"name": "a-board-refuses-a-tile-without-a-report-key-ops", "tiles": [{"w": 6, "h": 4}]}),
    );
    assert!(message.contains("every tile needs"), "{message}");
}

#[test]
fn a_board_paints_every_tile_and_survives_one_broken_report() {
    let mut f = Fixture::new("board-paint");
    f.run(
        "report.save",
        json!({"name": "a-board-paints-every-tile-and-survives-one-broken-report-good", "sql": "select day, total from sales", "chart": "line", "label": "day"}),
    );
    // Saved, then broken: the report is stored but its statement is not valid SQL any more.
    f.run("report.save", json!({"name": "a-board-paints-every-tile-and-survives-one-broken-report-broken", "sql": "select * from no_such_table", "chart": "line"}));
    let saved = f.run(
        "report.biDashboard",
        json!({"name": "a-board-paints-every-tile-and-survives-one-broken-report-ops", "tiles": [
            {"report": "a-board-paints-every-tile-and-survives-one-broken-report-good", "w": 6, "h": 4, "x": 0, "y": 0},
            {"report": "a-board-paints-every-tile-and-survives-one-broken-report-broken", "w": 6, "h": 4, "x": 6, "y": 0}
        ]}),
    );
    assert_eq!(saved["tiles"], json!(2), "{saved}");

    let painted = f.run(
        "report.board",
        json!({"name": "a-board-paints-every-tile-and-survives-one-broken-report-ops"}),
    );
    assert_eq!(
        painted["tile_count"],
        json!(1),
        "one tile painted: {painted}"
    );
    assert_eq!(painted["error_count"], json!(1), "{painted}");
    assert_eq!(
        painted["tiles"][0]["report"],
        json!("a-board-paints-every-tile-and-survives-one-broken-report-good")
    );
    assert_eq!(painted["tiles"][0]["chart"], json!("line"));
    assert_eq!(painted["tiles"][0]["labels"], json!(["mon", "tue", "wed"]));
    assert_eq!(
        painted["errors"][0]["report"],
        json!("a-board-paints-every-tile-and-survives-one-broken-report-broken")
    );
}

#[test]
fn a_board_keeps_the_layout_it_was_saved_with() {
    let mut f = Fixture::new("board-layout");
    f.run("report.save", json!({"name": "a-board-keeps-the-layout-it-was-saved-with-good", "sql": "select 1 as one", "chart": "table"}));
    f.run(
        "report.biDashboard",
        json!({"name": "a-board-keeps-the-layout-it-was-saved-with-ops", "tiles": [{"report": "a-board-keeps-the-layout-it-was-saved-with-good", "w": 4, "h": 3, "x": 8, "y": 2}]}),
    );
    let painted = f.run(
        "report.board",
        json!({"name": "a-board-keeps-the-layout-it-was-saved-with-ops"}),
    );
    assert_eq!(painted["tiles"][0]["layout"]["w"], json!(4));
    assert_eq!(painted["tiles"][0]["layout"]["x"], json!(8));
}

#[test]
fn a_board_input_binds_a_parameter_in_every_tile() {
    let mut f = Fixture::new("board-input");
    f.run(
        "report.save",
        json!({"name": "since", "sql": "select count(*) as n from sales where day = :since", "chart": "table"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "a-board-input-binds-a-parameter-in-every-tile-ops", "tiles": [{"report": "since", "w": 12, "h": 2}]}),
    );
    let painted = f.run(
        "report.board",
        json!({"name": "a-board-input-binds-a-parameter-in-every-tile-ops", "input.since": "tue"}),
    );
    assert_eq!(painted["error_count"], json!(0), "{painted}");
    let rows = &painted["tiles"][0]["result"]["rows"];
    assert_eq!(rows[0]["n"], json!(1), "only tue matches: {painted}");
}

#[test]
fn an_input_value_cannot_end_the_literal_it_is_bound_into() {
    // The value is a bound parameter, so a quote in it is data rather than the end of a literal.
    let mut f = Fixture::new("board-inject");
    f.run(
        "report.save",
        json!({"name": "inject-report", "sql": "select count(*) as n from sales where day = :since", "chart": "table"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "inject-board", "tiles": [{"report": "inject-report", "w": 12, "h": 2}]}),
    );
    let painted = f.run(
        "report.board",
        json!({"name": "inject-board", "input.since": "x' OR '1'='1"}),
    );
    assert_eq!(painted["error_count"], json!(0), "{painted}");
    let rows = &painted["tiles"][0]["result"]["rows"];
    assert_eq!(
        rows[0]["n"],
        json!(0),
        "the value is data, not SQL: {painted}"
    );
}

#[test]
fn an_input_binds_as_a_value_so_a_limit_takes_a_number() {
    // A text substitution would send `limit '2'`, which SQLite rejects. A bound parameter keeps the
    // slot's own type, so the tile runs.
    let mut f = Fixture::new("board-limit");
    f.run(
        "sql.run",
        json!({"sql": "CREATE TABLE people (name TEXT); INSERT INTO people VALUES ('ann'), ('bob'), ('cy');"}),
    );
    f.run("conn.refresh", json!({}));
    f.run(
        "report.save",
        json!({"name": "limited", "sql": "select name from people order by name limit :n", "chart": "table"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "limit-board", "tiles": [{"report": "limited", "w": 12, "h": 2}]}),
    );
    let painted = f.run(
        "report.board",
        json!({"name": "limit-board", "input.n": "2"}),
    );
    assert_eq!(painted["error_count"], json!(0), "{painted}");
    assert_eq!(
        painted["tiles"][0]["result"]["rows"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "{painted}"
    );
}

#[test]
fn a_tile_that_names_an_input_the_board_did_not_supply_says_which_one() {
    let mut f = Fixture::new("board-missing-input");
    f.run(
        "report.save",
        json!({"name": "wants-input", "sql": "select count(*) as n from people where name = :who", "chart": "table"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "missing-input-board", "tiles": [{"report": "wants-input", "w": 12, "h": 2}]}),
    );
    let painted = f.run("report.board", json!({"name": "missing-input-board"}));
    assert_eq!(painted["error_count"], json!(1), "{painted}");
    let message = painted["errors"][0]["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("who"),
        "the error names the input: {painted}"
    );
}

#[test]
fn a_metrics_board_reports_the_interval_it_should_be_asked_again() {
    let mut f = Fixture::new("metrics");
    f.run("report.save", json!({"name": "a-metrics-board-reports-the-interval-it-should-be-asked-again-good", "sql": "select 1 as one", "chart": "table"}));
    f.run(
        "report.biDashboard",
        json!({"name": "a-metrics-board-reports-the-interval-it-should-be-asked-again-ops", "tiles": [{"report": "a-metrics-board-reports-the-interval-it-should-be-asked-again-good", "w": 12, "h": 2}]}),
    );
    let out = f.run("monitor.charts", json!({"board": "a-metrics-board-reports-the-interval-it-should-be-asked-again-ops", "every_s": 15}));
    assert_eq!(out["every_s"], json!(15), "{out}");
    assert!(out["refresh_at"].as_u64().unwrap_or(0) > 0, "{out}");
    assert_eq!(out["tile_count"], json!(1), "{out}");
}

#[test]
fn a_board_that_was_never_saved_is_not_found() {
    let mut f = Fixture::new("board-absent");
    let message = f.refuse("report.board", json!({"name": "ghost"}));
    assert!(message.contains("no board named"), "{message}");
}

#[test]
fn exporting_a_board_writes_self_contained_html() {
    let mut f = Fixture::new("export-html");
    f.run(
        "report.save",
        json!({"name": "exporting-a-board-writes-self-contained-html-daily", "sql": "select day, total from sales", "chart": "bar", "label": "day"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "exporting-a-board-writes-self-contained-html-ops", "tiles": [{"report": "exporting-a-board-writes-self-contained-html-daily", "w": 12, "h": 4}]}),
    );
    let path = std::env::temp_dir().join(format!("nook-export-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let out = f.run(
        "report.export",
        json!({"name": "exporting-a-board-writes-self-contained-html-ops", "format": "html", "path": path.to_string_lossy()}),
    );
    assert_eq!(out["format"], json!("html"), "{out}");
    let html = std::fs::read_to_string(&path).expect("the file was written");
    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(html.contains("<td>mon</td>"), "the rows are in it: {html}");
    // A bar report exports as a drawing, not only as its numbers: the file has to show what the
    // screen showed.
    assert!(html.contains("<svg"), "the chart is drawn: {html}");
    assert!(html.contains("<rect"), "the bars are in it: {html}");
    assert!(
        !html.contains("href=") && !html.contains("src="),
        "self-contained means it fetches nothing: {html}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_export_names_the_tiles_it_left_out() {
    let mut f = Fixture::new("export-skipped");
    f.run(
        "report.save",
        json!({"name": "an-export-names-the-tiles-it-left-out-bound", "sql": "select sum(total) as n from sales where day = :day", "chart": "table"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "an-export-names-the-tiles-it-left-out-ops", "tiles": [{"report": "an-export-names-the-tiles-it-left-out-bound", "w": 12, "h": 4}]}),
    );
    let path = std::env::temp_dir().join(format!("nook-export-skip-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let out = f.run(
        "report.export",
        json!({"name": "an-export-names-the-tiles-it-left-out-ops", "format": "html", "path": path.to_string_lossy()}),
    );
    // No input was supplied, so the tile's query cannot run. Dropping it without a word would make
    // the file claim the board holds nothing else.
    assert_eq!(
        out["sections"].as_array().map(|s| s.len()),
        Some(0),
        "{out}"
    );
    let skipped = out["skipped"].as_array().cloned().unwrap_or_default();
    assert_eq!(skipped.len(), 1, "{out}");
    assert_eq!(
        skipped[0]["title"],
        json!("an-export-names-the-tiles-it-left-out-bound")
    );
    assert!(
        skipped[0]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("day"),
        "{out}"
    );
    assert!(
        out["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("left out"),
        "{out}"
    );
    // The same input the board takes fills the same parameter here, so an export can be the board.
    let out = f.run(
        "report.export",
        json!({"name": "an-export-names-the-tiles-it-left-out-ops", "format": "html", "path": path.to_string_lossy(), "input.day": "tue"}),
    );
    assert_eq!(out["skipped"].as_array().map(|s| s.len()), Some(0), "{out}");
    let html = std::fs::read_to_string(&path).expect("the file was written");
    assert!(
        html.contains("<td>20.0</td>"),
        "the bound value reached the file: {html}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn exporting_as_csv_writes_the_rows() {
    let mut f = Fixture::new("export-csv");
    f.run("report.save", json!({"name": "exporting-as-csv-writes-the-rows-daily", "sql": "select day, total from sales", "chart": "table"}));
    let path = std::env::temp_dir().join(format!("nook-export-{}.csv", std::process::id()));
    let _ = std::fs::remove_file(&path);
    f.run(
        "report.export",
        json!({"name": "exporting-as-csv-writes-the-rows-daily", "format": "csv", "path": path.to_string_lossy()}),
    );
    let text = std::fs::read_to_string(&path).expect("the file was written");
    assert!(text.contains("day"), "{text}");
    assert!(text.contains("mon"), "{text}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unknown_export_format_is_refused_and_names_the_two() {
    let mut f = Fixture::new("export-bad");
    f.run("report.save", json!({"name": "an-unknown-export-format-is-refused-and-names-the-two-daily", "sql": "select 1", "chart": "table"}));
    let message = f.refuse(
        "report.export",
        json!({"name": "an-unknown-export-format-is-refused-and-names-the-two-daily", "format": "pdf", "path": "/tmp/x.pdf"}),
    );
    assert!(message.contains("html or csv"), "{message}");
}

#[test]
fn a_metrics_input_names_the_binding_it_produces() {
    let mut f = Fixture::new("input");
    let out = f.run(
        "metrics.input",
        json!({"name": "since", "value": "2026-10-01"}),
    );
    assert_eq!(out["binding"], json!("input.since"), "{out}");
    assert_eq!(out["value"], json!("2026-10-01"));
}

#[test]
fn a_metrics_input_needs_a_value() {
    let mut f = Fixture::new("input-missing");
    let message = f.refuse("metrics.input", json!({"name": "since"}));
    assert!(message.contains("value"), "{message}");
}

#[test]
fn a_column_profile_reports_nulls_distincts_and_the_commonest_values() {
    let mut f = Fixture::new("profile");
    f.run(
        "sql.run",
        json!({"sql": "CREATE TABLE people (id INTEGER PRIMARY KEY, email TEXT, status TEXT); \
                       INSERT INTO people (email, status) VALUES \
                         ('a@b.co', 'active'), ('c@d.co', 'active'), (NULL, 'idle');"}),
    );
    f.run("conn.refresh", json!({}));
    let out = f.run(
        "tools.dataProfiling",
        json!({"table": "people", "columns": ["email", "status"]}),
    );
    assert_eq!(out["rows"], json!(3), "{out}");
    assert_eq!(out["count"], json!(2), "{out}");
    let email = &out["columns"][0];
    assert_eq!(email["column"], json!("email"));
    let null_share = email["null_share"].as_f64().expect("a share");
    assert!(
        (null_share - 1.0 / 3.0).abs() < 0.01,
        "one of three is NULL: {out}"
    );
    assert_eq!(email["distinct"], json!(2));
    assert_eq!(email["shape"], json!("email"));
    let status = &out["columns"][1];
    assert_eq!(status["column"], json!("status"));
    assert_eq!(status["distinct"], json!(2));
    assert_eq!(status["commonest"][0]["value"], json!("active"));
    assert_eq!(status["commonest"][0]["count"], json!(2));
}

#[test]
fn profiling_a_column_the_table_does_not_have_is_refused_by_name() {
    let mut f = Fixture::new("profile-typo");
    let message = f.refuse(
        "tools.dataProfiling",
        json!({"table": "sales", "columns": ["dayy"]}),
    );
    assert!(message.contains("no column"), "{message}");
    assert!(
        message.contains("dayy"),
        "the refusal names the typo: {message}"
    );
}

#[test]
fn profiling_with_no_columns_asked_for_profiles_every_column() {
    let mut f = Fixture::new("profile-all");
    let out = f.run("tools.dataProfiling", json!({"table": "sales"}));
    assert_eq!(out["count"], json!(2), "sales has day and total: {out}");
}

#[test]
fn a_uuid_column_is_recognised_as_one() {
    let mut f = Fixture::new("profile-uuid");
    f.run(
        "sql.run",
        json!({"sql": "CREATE TABLE tokens (id TEXT); \
                       INSERT INTO tokens VALUES \
                         ('3f2504e0-4f89-11d3-9a0c-0305e82c3301'), \
                         ('6ba7b810-9dad-11d1-80b4-00c04fd430c8');"}),
    );
    f.run("conn.refresh", json!({}));
    let out = f.run("tools.dataProfiling", json!({"table": "tokens"}));
    assert_eq!(out["columns"][0]["shape"], json!("uuid"), "{out}");
}

#[test]
fn a_job_bundle_round_trips_and_never_writes_a_secret() {
    let mut f = Fixture::new("jobs");
    // The job store lives in the shared scratch config, so the id is unique per process: a rerun
    // in the same directory must not collide with the previous run's job.
    let id = format!("bundle-a-{}", std::process::id());
    f.run(
        "job.create",
        json!({"id": id, "name": "Nightly", "kind": "query", "conn": f.conn.clone(),
               "params": {"sql": "select 1", "password": "hunter2"}, "schedule": "03:00"}),
    );
    let dir = std::env::temp_dir().join("nook-reports");
    let bundle = dir.join(format!("jobs-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&bundle);
    let out = f.run(
        "job.export",
        json!({"path": bundle.to_string_lossy(), "ids": [id]}),
    );
    assert_eq!(out["jobs"], json!(1), "{out}");
    assert_eq!(out["stripped"], json!(["password"]), "{out}");

    let text = std::fs::read_to_string(&bundle).expect("the bundle was written");
    assert!(
        !text.contains("hunter2"),
        "a password reached the bundle: {text}"
    );
    assert!(text.contains("\"secrets\": \"omitted\""), "{text}");

    // Importing it back adds the job rather than failing on the existing one.
    let imported = f.run("job.import", json!({"path": bundle.to_string_lossy()}));
    assert_eq!(imported["imported"], json!(1), "{imported}");
    let _ = std::fs::remove_file(&bundle);
}

#[test]
fn a_bundle_whose_chain_points_outside_it_is_refused_before_anything_is_written() {
    let mut f = Fixture::new("jobs-chain");
    let dir = std::env::temp_dir().join("nook-reports");
    let bundle = dir.join(format!("bad-chain-{}.json", std::process::id()));
    std::fs::write(
        &bundle,
        r#"{"version":1,"secrets":"omitted","jobs":[
            {"id":"a","name":"A","kind":"query","conn":"","params":{},"schedule":{"kind":"manual"},"then":["ghost"]}
        ]}"#,
    )
    .expect("the bundle is written");
    let message = f.refuse("job.import", json!({"path": bundle.to_string_lossy()}));
    assert!(message.contains("ghost"), "{message}");
    let listed = f.run("job.list", json!({}));
    assert!(
        !listed["jobs"]
            .as_array()
            .expect("jobs")
            .iter()
            .any(|j| j["id"] == json!("a")),
        "a refused bundle was partly written: {listed}"
    );
    let _ = std::fs::remove_file(&bundle);
}

#[test]
fn a_bundle_whose_chain_loops_is_refused() {
    let mut f = Fixture::new("jobs-loop");
    let dir = std::env::temp_dir().join("nook-reports");
    let bundle = dir.join(format!("loop-{}.json", std::process::id()));
    std::fs::write(
        &bundle,
        r#"{"version":1,"secrets":"omitted","jobs":[
            {"id":"loop-a","name":"A","kind":"query","conn":"","params":{},"schedule":{"kind":"manual"},"then":["loop-b"]},
            {"id":"loop-b","name":"B","kind":"query","conn":"","params":{},"schedule":{"kind":"manual"},"then":["loop-a"]}
        ]}"#,
    )
    .expect("the bundle is written");
    let message = f.refuse("job.import", json!({"path": bundle.to_string_lossy()}));
    assert!(message.contains("loops back"), "{message}");
    let _ = std::fs::remove_file(&bundle);
}

#[test]
fn a_file_that_is_not_a_bundle_is_refused_by_name() {
    let mut f = Fixture::new("jobs-shape");
    let dir = std::env::temp_dir().join("nook-reports");
    let bundle = dir.join(format!("not-a-bundle-{}.json", std::process::id()));
    std::fs::write(&bundle, r#"{"jobs": "nope"}"#).expect("the file is written");
    let message = f.refuse("job.import", json!({"path": bundle.to_string_lossy()}));
    assert!(message.contains("jobs"), "{message}");
    let _ = std::fs::remove_file(&bundle);
}

#[test]
fn exporting_with_no_jobs_is_refused_rather_than_writing_an_empty_bundle() {
    let mut f = Fixture::new("jobs-empty");
    let dir = std::env::temp_dir().join("nook-reports");
    let bundle = dir.join(format!("empty-{}.json", std::process::id()));
    let message = f.refuse(
        "job.export",
        json!({"path": bundle.to_string_lossy(), "ids": ["nonesuch"]}),
    );
    assert!(message.contains("no job named"), "{message}");
}

#[test]
fn the_language_is_reported_set_and_refused_by_name() {
    let mut f = Fixture::new("i18n");
    // The locale is a persisted preference, so the starting value belongs to whichever test ran
    // first. Set it explicitly rather than asserting a default the scratch config may have changed.
    f.run("platform.i18n", json!({"locale": "en"}));
    let before = f.run("platform.i18n", json!({}));
    assert_eq!(before["locale"], json!("en"), "{before}");
    assert!(
        before["languages"].as_array().map(Vec::len).unwrap_or(0) >= 2,
        "{before}"
    );

    let set = f.run("platform.i18n", json!({"locale": "id"}));
    assert_eq!(set["locale"], json!("id"), "{set}");
    assert_eq!(set["saved"], json!(true), "{set}");
    assert_eq!(f.run("app.settings", json!({}))["locale"], json!("id"));

    let message = f.refuse("platform.i18n", json!({"locale": "kl"}));
    assert!(
        message.contains("en, id"),
        "the refusal names the two: {message}"
    );
}

#[test]
fn the_control_channel_reports_its_port_and_methods_without_inventing_either() {
    let mut f = Fixture::new("control-info");
    let out = f.run("platform.control", json!({}));
    // A headless session has no channel: the answer says so rather than naming a port nothing bound.
    assert_eq!(out["running"], json!(false), "{out}");
    assert!(out["port"].is_null(), "{out}");
    assert!(
        out["methods"].as_array().map(Vec::len).unwrap_or(0) >= 15,
        "{out}"
    );
}

#[test]
fn the_cli_command_reports_the_same_registry_it_runs() {
    let mut f = Fixture::new("cli-info");
    let out = f.run("platform.cli", json!({}));
    assert_eq!(out["binary"], json!("nook-cli"), "{out}");
    assert_eq!(
        out["commands"].as_u64().unwrap_or(0) as usize,
        nook_engine::registry().len(),
        "{out}"
    );
    assert!(out["on_path"].is_boolean(), "{out}");
}

#[test]
fn a_board_reports_the_inputs_its_tiles_read_so_the_view_can_paint_a_field_for_each() {
    let mut f = Fixture::new("board-inputs");
    f.run(
        "sql.run",
        json!({"sql": "CREATE TABLE days (d TEXT); INSERT INTO days VALUES ('mon'), ('tue');"}),
    );
    f.run("conn.refresh", json!({}));
    f.run(
        "report.save",
        json!({"name": "by-day", "sql": "select count(*) as n from days where d = :since", "chart": "table"}),
    );
    f.run(
        "report.save",
        json!({"name": "all-days", "sql": "select count(*) as n from days", "chart": "table"}),
    );
    f.run(
        "report.biDashboard",
        json!({"name": "inputs-board", "tiles": [{"report": "by-day", "w": 6, "h": 2}, {"report": "all-days", "w": 6, "h": 2}]}),
    );
    let painted = f.run(
        "report.board",
        json!({"name": "inputs-board", "input.since": "tue"}),
    );
    let inputs = painted["inputs"].as_array().cloned().unwrap_or_default();
    // Only the tile that names `:since` contributes a field, and it carries the value this call sent.
    assert_eq!(inputs.len(), 1, "{painted}");
    assert_eq!(inputs[0]["name"], json!("since"), "{painted}");
    assert_eq!(inputs[0]["value"], json!("tue"), "{painted}");
}

#[test]
fn job_profiles_names_the_store_and_the_format_that_moves_it() {
    let mut f = Fixture::new("job-profiles");
    let out = f.run("job.profiles", json!({}));
    assert!(
        out["path"].as_str().unwrap_or_default().ends_with(".json"),
        "{out}"
    );
    assert_eq!(out["bundle"]["secrets"], json!("omitted"), "{out}");
    assert_eq!(out["bundle"]["write"], json!("job.export"), "{out}");
    assert!(out["jobs"].as_u64().is_some(), "{out}");
}

#[test]
fn the_designer_stores_what_it_was_given_and_reads_it_back() {
    let mut f = Fixture::new("designer");
    let name = "the-designer-stores-what-it-was-given-and-reads-it-back";
    // The store outlives the test process, so a previous run's row would make the "blank draft" half
    // pass or fail on history rather than on behaviour.
    let _ = f.run("report.delete", json!({"name": name}));
    // A name alone is a read: a report that does not exist opens as a blank draft.
    let draft = f.run("report.designer", json!({"name": name}));
    assert_eq!(draft["exists"], json!(false), "{draft}");
    assert_eq!(draft["saved"], json!(false), "{draft}");
    // Any write parameter stores it — the help says the designer is the save surface.
    let saved = f.run(
        "report.designer",
        json!({"name": name, "sql": "select day, total from sales", "chart": "line", "label": "day", "layout": {"w": 6, "h": 4}}),
    );
    assert_eq!(saved["saved"], json!(true), "{saved}");
    let back = f.run("report.list", json!({}));
    let listed = back["reports"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|r| r["name"] == json!(name))
        .unwrap_or_else(|| panic!("the designer wrote nothing: {back}"));
    assert_eq!(listed["chart"], json!("line"), "{listed}");
    assert_eq!(
        listed["sql"],
        json!("select day, total from sales"),
        "{listed}"
    );
    assert_eq!(listed["options"]["layout"]["w"], json!(6), "{listed}");
}

#[test]
fn the_designer_refuses_a_chart_kind_it_cannot_paint() {
    let mut f = Fixture::new("designer-chart");
    let why = f.refuse(
        "report.designer",
        json!({"name": "the-designer-refuses-a-chart-kind-it-cannot-paint", "sql": "select 1", "chart": "sunburst"}),
    );
    assert!(why.contains("sunburst"), "{why}");
    assert!(
        why.contains("line"),
        "the refusal names what it accepts: {why}"
    );
}
