//! Phase 4, steps 1–3: the structured filter, its preview, and bulk edit. Every case asserts the
//! rows the server actually holds — a filter that "applied" while matching nothing, or a bulk edit
//! that staged the wrong count, is exactly the failure this phase exists to prevent.

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
        let dir = std::env::temp_dir().join("nook-phase4");
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

    /// A query tab holding `sql`, run once, and left active: the state a person is in when they
    /// ask for a result to be editable.
    fn query(&mut self, sql: &str) -> serde_json::Value {
        self.session
            .run("sql.newQuery", json!({"conn": self.conn, "sql": sql}))
            .expect("query tab opens");
        self.session
            .run("sql.run", json!({"sql": sql, "conn": self.conn}))
            .expect("sql runs")
    }

    fn open_grid(&mut self, table: &str) {
        self.session
            .run("grid.open", json!({"conn": self.conn, "table": table}))
            .expect("grid opens");
    }

    fn run(&mut self, id: &str, params: serde_json::Value) -> serde_json::Value {
        self.session
            .run(id, params)
            .unwrap_or_else(|e| panic!("{id} failed: {e}"))
    }

    fn try_run(
        &mut self,
        id: &str,
        params: serde_json::Value,
    ) -> nook_core::Result<serde_json::Value> {
        self.session.run(id, params)
    }

    /// The status column of every row, straight from the server, in id order.
    fn statuses(&mut self) -> Vec<String> {
        let out = self.sql("SELECT status FROM orders ORDER BY id");
        out["objects"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|row| row["status"].as_str().unwrap_or("<null>").to_string())
            .collect()
    }

    /// The emails the grid is showing, in page order.
    fn page_emails(&self) -> Vec<String> {
        let index = self.session.active_tab.expect("a tab is active");
        self.session.tabs[index]
            .grid
            .result
            .rows
            .iter()
            .filter_map(|row| row.first().map(|v| v.display()))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn orders(f: &mut Fixture) {
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT, total REAL)");
    f.sql(
        "INSERT INTO orders (id, status, total) VALUES \
         (1, 'live', 50), (2, 'live', 150), (3, 'draft', 500), (4, 'live', 900)",
    );
}

// ------------------------------------------------------------------- filters

#[test]
fn filters_compile_to_a_where_the_grid_runs() {
    let mut f = Fixture::new("filters");
    orders(&mut f);
    f.open_grid("orders");
    let out = f.run(
        "grid.filters",
        json!({"filters": [
            {"column": "status", "op": "=", "value": "live"},
            {"column": "total", "op": ">", "value": 100}
        ]}),
    );
    assert_eq!(
        out["where"], "\"status\" = 'live' AND \"total\" > 100",
        "{out:?}"
    );
    assert_eq!(out["conditions"], 2);
    // The page is the filtered one, not just a stored string.
    let ids = f.run("grid.copyTsv", json!({"header": false}))["tsv"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(ids, "2\tlive\t150.0\n4\tlive\t900.0\n", "{ids:?}");
}

#[test]
fn an_empty_filter_list_clears_the_where() {
    let mut f = Fixture::new("filters-clear");
    orders(&mut f);
    f.open_grid("orders");
    f.run(
        "grid.filters",
        json!({"filters": [{"column": "status", "op": "=", "value": "draft"}]}),
    );
    let out = f.run("grid.filters", json!({"filters": []}));
    assert_eq!(out["where"], "", "{out:?}");
    assert_eq!(out["rows"], json!(4), "every row is back");
}

#[test]
fn filter_preview_counts_without_changing_the_grid() {
    let mut f = Fixture::new("preview");
    orders(&mut f);
    f.open_grid("orders");
    let out = f.run(
        "grid.filterPreview",
        json!({"filters": [{"column": "status", "op": "=", "value": "live"}]}),
    );
    assert_eq!(out["rows"], json!(3), "{out:?}");
    assert_eq!(out["applied"], json!(false));
    assert_eq!(
        out["count_sql"],
        "SELECT count(*) FROM \"orders\" WHERE \"status\" = 'live'"
    );
    // The grid still shows all four rows: a preview is not an apply.
    assert_eq!(f.page_emails().len(), 4);
}

#[test]
fn a_hostile_filter_value_is_data_and_a_junk_operator_is_refused() {
    let mut f = Fixture::new("filters-hostile");
    orders(&mut f);
    f.open_grid("orders");
    // A quote in the value is escaped, so this is one string and matches nothing rather than
    // becoming `status = 'live' OR 1=1`.
    let out = f.run(
        "grid.filters",
        json!({"filters": [{"column": "status", "op": "=", "value": "live' OR '1'='1"}]}),
    );
    assert_eq!(
        out["rows"],
        json!(0),
        "an injected tail is not SQL: {out:?}"
    );
    let err = f
        .try_run(
            "grid.filters",
            json!({"filters": [{"column": "status", "op": "; drop table orders", "value": 1}]}),
        )
        .expect_err("a junk operator is refused");
    assert!(err.message().contains("not a filter operator"), "{err}");
    // And the table is still there.
    assert_eq!(f.statuses().len(), 4);
}

#[test]
fn the_is_null_condition_matches_null_rows_and_not_equals_null() {
    let mut f = Fixture::new("filters-null");
    f.sql("CREATE TABLE people (id INTEGER PRIMARY KEY, email TEXT)");
    f.sql("INSERT INTO people (id, email) VALUES (1, 'a@b.c'), (2, NULL)");
    f.open_grid("people");
    let out = f.run(
        "grid.filters",
        json!({"filters": [{"column": "email", "op": "is null"}]}),
    );
    assert_eq!(out["where"], "\"email\" IS NULL");
    assert_eq!(out["rows"], json!(1), "{out:?}");
}

// ---------------------------------------------------------------- bulk edit

#[test]
fn a_filter_naming_a_column_the_result_lacks_is_refused_not_read_as_a_literal() {
    let mut f = Fixture::new("filters-typo");
    orders(&mut f);
    f.open_grid("orders");
    // SQLite reads `"ctiy"` as the string `'ctiy'`, so an unchecked typo compiles to a filter that
    // matches nothing and reports success. It has to be an error that names the column.
    let err = f
        .try_run(
            "grid.filters",
            json!({"filters": [{"column": "ctiy", "op": "=", "value": "London"}]}),
        )
        .expect_err("an unknown column is refused");
    assert!(err.message().contains("ctiy"), "{err}");
    assert!(
        err.message().contains("status"),
        "{err}: the message lists the real columns"
    );
}

#[test]
fn a_hand_typed_where_clears_the_chips_it_replaces() {
    let mut f = Fixture::new("filters-raw");
    orders(&mut f);
    f.open_grid("orders");
    f.run(
        "grid.filters",
        json!({"filters": [{"column": "status", "op": "=", "value": "draft"}]}),
    );
    f.run("grid.filter", json!({"where": "status = 'live'"}));
    let index = f.session.active_tab.expect("a tab is active");
    assert!(
        f.session.tabs[index].grid.filters.is_empty(),
        "the chips still describe the filter the raw WHERE replaced"
    );
    // And the same when the chips are cleared outright.
    f.run(
        "grid.filters",
        json!({"filters": [{"column": "status", "op": "=", "value": "draft"}]}),
    );
    f.run("grid.clearFilter", json!({}));
    assert!(
        f.session.tabs[index].grid.filters.is_empty(),
        "Clear all left a chip behind"
    );
}

#[test]
fn bulk_edit_stages_one_edit_per_row_and_the_apply_lands_them() {
    let mut f = Fixture::new("bulk");
    orders(&mut f);
    f.open_grid("orders");
    let out = f.run(
        "grid.bulkEdit",
        json!({"column": "status", "value": "archived", "rows": [0, 1, 3]}),
    );
    assert_eq!(out["staged"], json!(3), "{out:?}");
    assert_eq!(out["pending"], json!(3));
    // Nothing is written until the apply.
    assert_eq!(f.statuses(), vec!["live", "live", "draft", "live"]);
    f.run("grid.apply", json!({}));
    assert_eq!(
        f.statuses(),
        vec!["archived", "archived", "draft", "archived"],
        "the rows the edit named changed and the one it did not is untouched"
    );
}

#[test]
fn a_bulk_edit_value_of_null_is_a_null_not_the_word() {
    let mut f = Fixture::new("bulk-null");
    orders(&mut f);
    f.open_grid("orders");
    f.run(
        "grid.bulkEdit",
        json!({"column": "status", "value": null, "rows": [0, 1]}),
    );
    f.run("grid.apply", json!({}));
    let out = f.sql("SELECT count(*) AS n FROM orders WHERE status IS NULL");
    assert_eq!(out["objects"][0]["n"], json!(2), "{out:?}");
}

#[test]
fn bulk_edit_refuses_a_column_that_is_not_there_and_a_primary_key_across_rows() {
    let mut f = Fixture::new("bulk-refuse");
    orders(&mut f);
    f.open_grid("orders");
    let err = f
        .try_run(
            "grid.bulkEdit",
            json!({"column": "nope", "value": 1, "rows": [0]}),
        )
        .expect_err("a column that does not exist is refused");
    assert!(err.message().contains("nope"), "{err}");
    // Setting the key of three rows to one value is a collision the server would reject; refusing
    // it here names the reason instead of surfacing a constraint error three statements in.
    let err = f
        .try_run(
            "grid.bulkEdit",
            json!({"column": "id", "value": 7, "rows": [0, 1, 2]}),
        )
        .expect_err("a key set across rows is refused");
    assert!(err.message().contains("primary key"), "{err}");
    assert!(
        f.try_run("grid.pending", json!({})).unwrap()["pending"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a refused bulk edit stages nothing"
    );
}

#[test]
fn a_refused_row_leaves_a_bulk_edit_exactly_as_it_was() {
    let mut f = Fixture::new("bulk-atomic");
    orders(&mut f);
    f.open_grid("orders");
    // Row 3 exists; row 9 does not. The batch must stage nothing rather than the two good rows.
    let err = f
        .try_run(
            "grid.bulkEdit",
            json!({"column": "status", "value": "x", "rows": [0, 1, 9]}),
        )
        .expect_err("the last row is not on the page");
    assert!(err.message().contains('9'), "{err}");
    assert!(
        f.try_run("grid.pending", json!({})).unwrap()["pending"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a batch refused partway staged rows before the refusal"
    );
}

#[test]
fn bulk_edit_caps_the_row_count_and_says_so() {
    let mut f = Fixture::new("bulk-cap");
    f.sql("CREATE TABLE wide (id INTEGER PRIMARY KEY, status TEXT)");
    let values: Vec<String> = (1..=250).map(|i| format!("({i}, 'live')")).collect();
    f.sql(&format!(
        "INSERT INTO wide (id, status) VALUES {}",
        values.join(", ")
    ));
    f.open_grid("wide");
    let rows: Vec<usize> = (0..250).collect();
    let out = f.run(
        "grid.bulkEdit",
        json!({"column": "status", "value": "x", "rows": rows}),
    );
    assert_eq!(out["staged"], json!(200), "capped at max_rows-ish: {out:?}");
    assert_eq!(out["skipped"], json!(50), "{out:?}");
}

#[test]
fn bulk_edit_refuses_a_row_the_page_does_not_hold() {
    let mut f = Fixture::new("bulk-oob");
    orders(&mut f);
    f.open_grid("orders");
    let err = f
        .try_run(
            "grid.bulkEdit",
            json!({"column": "status", "value": "x", "rows": [99]}),
        )
        .expect_err("a row outside the page is refused");
    assert!(err.message().contains("99"), "{err}");
}

#[test]
fn a_query_result_becomes_editable_and_its_apply_writes_the_row() {
    let mut f = Fixture::new("query-editable");
    f.sql("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("INSERT INTO people (id, name) VALUES (1, 'Ada'), (2, 'Bob')");
    f.query("select id, name from people order by id");
    let out = f
        .session
        .run("grid.editQueryResult", json!({"statement": 0}))
        .expect("the projection carries the key");
    assert_eq!(out["table"], json!("people"), "{out:?}");
    assert_eq!(out["key"], json!(["id"]), "{out:?}");
    // The statement's own rows are the page, so row 0 is Ada, not whoever the table lists first.
    let page = &f.session.tabs[f.session.active_tab.unwrap()].grid.result;
    assert_eq!(page.rows[0][1].display(), "Ada");

    f.session
        .run(
            "grid.editCell",
            json!({"row": 0, "column": "name", "value": "Ada L."}),
        )
        .expect("stages");
    let out = f.session.run("grid.apply", json!({})).expect("apply");
    assert_eq!(out["applied"], json!(1), "{out:?}");

    let rows = f.sql("select id, name from people order by id");
    assert_eq!(rows["objects"][0]["name"], json!("Ada L."), "{rows:?}");
    assert_eq!(rows["objects"][1]["name"], json!("Bob"), "{rows:?}");
}

#[test]
fn a_page_query_command_refuses_an_editable_result_instead_of_lying() {
    let mut f = Fixture::new("query-sort");
    f.sql("CREATE TABLE items (id INTEGER PRIMARY KEY, kind TEXT)");
    f.sql("INSERT INTO items (id, kind) VALUES (1, 'b'), (2, 'a')");
    f.query("select id, kind from items order by id");
    f.session
        .run("grid.editQueryResult", json!({"statement": 0}))
        .expect("editable");
    // Sorting would be stored and reported back by `grid_summary` while the reload re-ran the
    // statement, which is the silent lie: the grid would claim an order it never applied.
    for (id, params) in [
        ("grid.sort", json!({"column": "kind"})),
        ("grid.filter", json!({"where": "kind = 'a'"})),
        ("grid.clearFilter", json!({})),
        ("grid.fetchPage", json!({"page": 1})),
        (
            "grid.filters",
            json!({"filters": [{"column": "kind", "op": "=", "value": "a"}]}),
        ),
    ] {
        let err = f.try_run(id, params.clone()).expect_err(id);
        assert!(err.message().contains("statement"), "{id}: {err}");
    }
    // And the rows are still the statement's, in the statement's order.
    let index = f.session.active_tab.expect("a tab is active");
    let kinds: Vec<String> = f.session.tabs[index]
        .grid
        .result
        .rows
        .iter()
        .map(|r| r[1].display())
        .collect();
    assert_eq!(kinds, vec!["b", "a"], "a refused command moved the page");
}

#[test]
fn a_result_whose_rows_are_not_one_per_table_row_is_refused_with_its_reason() {
    let mut f = Fixture::new("query-refuse");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT)");
    f.sql("INSERT INTO orders (id, status) VALUES (1, 'new'), (2, 'new')");
    f.sql("CREATE TABLE log (at TEXT, note TEXT)");
    f.sql("INSERT INTO log (at, note) VALUES ('now', 'x')");

    for (sql, needle) in [
        (
            "select status, count(*) from orders group by status",
            "GROUP BY",
        ),
        ("select distinct status from orders", "DISTINCT"),
        ("select * from log", "primary key"),
        ("select status from orders", "primary key column"),
    ] {
        f.query(sql);
        let err = f
            .session
            .run("grid.editQueryResult", json!({"statement": 0}))
            .expect_err(sql);
        assert!(
            err.message().contains(needle),
            "{sql:?} should be refused with {needle:?}, got {}",
            err.message()
        );
    }
}

#[test]
fn an_editable_query_reloads_by_running_its_statement_not_the_whole_table() {
    let mut f = Fixture::new("query-reload");
    f.sql("CREATE TABLE items (id INTEGER PRIMARY KEY, kind TEXT)");
    f.sql("INSERT INTO items (id, kind) VALUES (1, 'a'), (2, 'b'), (3, 'a')");
    f.query("select id, kind from items where kind = 'a' order by id");
    f.session
        .run("grid.editQueryResult", json!({"statement": 0}))
        .expect("editable");
    f.session
        .run(
            "grid.editCell",
            json!({"row": 1, "column": "kind", "value": "b"}),
        )
        .expect("stages");
    f.session.run("grid.apply", json!({})).expect("apply");
    // The reloaded page is still the filtered statement: one row left, not the three in the table.
    let page = &f.session.tabs[f.session.active_tab.unwrap()].grid.result;
    // Rows 1 and 3 matched; row 1 of the page (id 3) became 'b', so id 1 is the only one left.
    assert_eq!(page.row_count(), 1, "{:?}", page.columns);
    assert_eq!(page.rows[0][0].display(), "1");
}

#[test]
fn a_re_run_makes_the_result_read_only_again() {
    let mut f = Fixture::new("query-stale");
    f.sql("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)");
    f.sql("INSERT INTO users (id, email) VALUES (1, 'a@x'), (2, 'b@x')");
    f.query("select id, email from users order by id");
    f.session
        .run("grid.editQueryResult", json!({"statement": 0}))
        .expect("editable");
    // A second run replaces the rows the resolution was made against.
    f.session
        .run("sql.run", json!({"sql": "select 1", "conn": f.conn}))
        .expect("runs");
    let err = f
        .session
        .run(
            "grid.editCell",
            json!({"row": 0, "column": "email", "value": "z@x"}),
        )
        .expect_err("a re-run result is not editable");
    // The binding went with the marker, so the tab is back to being a plain query.
    assert!(err.message().contains("no data grid"), "{err}");
    // And nothing was staged anywhere.
    assert_eq!(
        f.session.tabs[f.session.active_tab.unwrap()]
            .grid
            .pending
            .len(),
        0
    );
}

#[test]
fn raw_mode_lists_each_statement_and_replaces_one_cells_value() {
    let mut f = Fixture::new("raw-mode");
    orders(&mut f);
    f.open_grid("orders");
    f.session
        .run(
            "grid.editCell",
            json!({"row": 0, "column": "status", "value": "live"}),
        )
        .expect("stages");
    let out = f.session.run("grid.rawMode", json!({})).expect("lists");
    let edits = out["edits"].as_array().expect("edits");
    assert_eq!(edits.len(), 1, "{out:?}");
    assert!(
        edits[0]["sql"].as_str().unwrap().starts_with("UPDATE"),
        "{out:?}"
    );

    let planned = edits[0]["sql"].as_str().expect("sql");
    // Rewrite the value in the statement the review shows, keeping the WHERE it was planned with.
    let where_at = planned
        .rfind(" WHERE ")
        .expect("the planner writes a WHERE");
    let sql = format!(
        "UPDATE orders SET status = 'Archived'{}",
        &planned[where_at..]
    );
    let out = f
        .session
        .run("grid.rawMode", json!({"sql": sql}))
        .expect("maps back");
    assert_eq!(out["index"], json!(0), "{out:?}");
    // The value round-trips with its case, and the statement the apply will run is re-planned.
    let pending = f.session.run("grid.pending", json!({})).expect("pending");
    assert!(
        pending["pending"][0]["sql"]
            .as_str()
            .unwrap()
            .contains("'Archived'"),
        "{pending:?}"
    );
}

#[test]
fn raw_mode_refuses_a_statement_it_cannot_map_and_leaves_the_list_alone() {
    let mut f = Fixture::new("raw-refuse");
    orders(&mut f);
    f.open_grid("orders");
    f.session
        .run(
            "grid.editCell",
            json!({"row": 0, "column": "status", "value": "live"}),
        )
        .expect("stages");
    let before = f.session.run("grid.pending", json!({})).expect("pending");

    for sql in [
        // A WHERE that matches no pending edit.
        "UPDATE orders SET status = 'x' WHERE id = 999;",
        // A column this edit does not change.
        "UPDATE orders SET total = 1 WHERE id = 1;",
        // A statement that is not an UPDATE.
        "DELETE FROM orders WHERE id = 1;",
    ] {
        f.session
            .run("grid.rawMode", json!({"sql": sql}))
            .expect_err(sql);
    }
    let after = f.session.run("grid.pending", json!({})).expect("pending");
    assert_eq!(before, after, "a refused statement must change nothing");
}

/// A fake `security` on `NOOK_SECURITY_BIN` and a fake `secret-tool` on `NOOK_SECRET_TOOL_BIN`: the
/// two backends answer lookups with `secret` and succeed on a store. One test owns it, because the
/// overrides are process-wide and two tests setting them would race.
fn fake_security(secret: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("nook-phase4-security");
    let _ = std::fs::create_dir_all(&dir);
    let script = dir.join("security");
    let body = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *find-generic-password*) printf '%s\\n' '{secret}' ;;\nesac\nexit 0\n"
    );
    let _ = std::fs::write(&script, body);
    // Linux reaches its keychain through `secret-tool`, and without a fake here the real one runs and
    // fails on the missing session bus — the test is about safe mode, not about which CLI is present.
    let linux = dir.join("secret-tool");
    let linux_body = format!(
        "#!/bin/sh\ncase \"$1\" in\n  lookup) printf '%s\\n' '{secret}' ;;\n  store) cat > /dev/null ;;\nesac\nexit 0\n"
    );
    let _ = std::fs::write(&linux, linux_body);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [&script, &linux] {
            let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755));
        }
    }
    std::env::set_var("NOOK_SECURITY_BIN", &script);
    std::env::set_var("NOOK_SECRET_TOOL_BIN", &linux);
    script
}

#[test]
fn safe_mode_hands_out_no_token_without_a_password_to_prove() {
    let _script = fake_security("safe-token-value");
    let mut f = Fixture::new("safe-password");
    orders(&mut f);
    f.open_grid("orders");
    // The proof has to cost the caller the credential. Reading the stored one would make the gate
    // two commands deep for any script that can reach the control channel.
    let err = f
        .try_run(
            "editor.safeMode",
            json!({"enabled": true, "confirm": true, "conn": f.conn}),
        )
        .expect_err("no password is refused");
    assert!(err.message().contains("password"), "{err}");
}

#[test]
fn safe_mode_gates_the_commit_on_a_token_only_a_connection_test_hands_out() {
    let _script = fake_security("safe-token-value");
    let mut f = Fixture::new("safe-mode");
    orders(&mut f);
    f.open_grid("orders");

    // Off: exactly today's behaviour.
    f.session
        .run("editor.safeMode", json!({"enabled": false}))
        .expect("off");
    f.session
        .run(
            "grid.editCell",
            json!({"row": 0, "column": "status", "value": "live"}),
        )
        .expect("stages");
    f.session
        .run("grid.apply", json!({}))
        .expect("applies with safe mode off");

    // On, and no token presented: refused before anything is planned, buffer untouched.
    let out = f
        .session
        .run("editor.safeMode", json!({"enabled": true}))
        .expect("on");
    assert_eq!(out["safe_mode"], json!(true), "{out:?}");
    assert_eq!(out["token"], json!(null), "{out:?}");
    f.session
        .run(
            "grid.editCell",
            json!({"row": 0, "column": "status", "value": "live"}),
        )
        .expect("stages");
    let err = f
        .session
        .run("grid.apply", json!({}))
        .expect_err("no token");
    assert!(err.message().contains("safe mode"), "{err}");
    assert_eq!(
        f.session.tabs[f.session.active_tab.unwrap()]
            .grid
            .pending
            .len(),
        1,
        "a refused gate must not clear the buffer"
    );

    // A wrong token is refused, including on a confirm-gated destructive command.
    let err = f
        .session
        .run("grid.apply", json!({"safe_token": "not-the-token"}))
        .expect_err("wrong token");
    assert!(err.message().contains("safe mode"), "{err}");
    let err = f
        .session
        .run(
            "schema.dropTable",
            json!({"table": "orders", "confirm": true, "safe_token": "nope"}),
        )
        .expect_err("wrong token on a drop");
    assert!(err.message().contains("safe mode"), "{err}");

    // The right token applies it.
    let token = nook_engine::profiles::safe_token(&f.conn).expect("a token");
    let out = f
        .session
        .run("grid.apply", json!({"safe_token": token}))
        .expect("applies with the token");
    assert_eq!(out["applied"], json!(1), "{out:?}");
}
