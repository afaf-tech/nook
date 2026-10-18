//! Phase 3, steps 1–2: TSV copy and rectangle selection. `grid.copyTsv` must return the same
//! bytes a spreadsheet paste expects; `grid.selection` must read and set the rectangle so the
//! pointer's work is scriptable without synthesising mouse events.

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
        let dir = std::env::temp_dir().join("nook-grid-reading");
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

    /// Open the grid for a table and make it the active tab.
    fn open_grid(&mut self, table: &str) {
        self.session
            .run("grid.open", json!({"conn": self.conn, "table": table}))
            .expect("grid opens");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn copy_tsv_returns_a_spreadsheet_table() {
    let mut f = Fixture::new("copy-tsv");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY, note TEXT)");
    f.sql("INSERT INTO t (id, note) VALUES (1, 'line1\nline2'), (2, NULL)");
    f.open_grid("t");
    let out = f
        .session
        .run("grid.copyTsv", json!({"header": true}))
        .expect("copy");
    let tsv = out["tsv"].as_str().expect("the text");
    // Header, escaped newline in a wrapper, NULL as an empty cell.
    assert_eq!(tsv, "id\tnote\n1\t\"line1\\nline2\"\n2\t\n", "{tsv:?}");
    // Without the header.
    let out = f
        .session
        .run("grid.copyTsv", json!({"header": false}))
        .expect("copy");
    assert_eq!(out["tsv"].as_str().unwrap(), "1\t\"line1\\nline2\"\n2\t\n");
}

#[test]
fn copy_tsv_slices_the_selection_rectangle() {
    let mut f = Fixture::new("copy-sel");
    f.sql("CREATE TABLE t (a INTEGER, b INTEGER, c INTEGER)");
    f.sql("INSERT INTO t VALUES (1,2,3),(4,5,6),(7,8,9)");
    f.open_grid("t");
    // The rectangle rows 0..=1, columns 0..=1 (a and b of the first two rows).
    let out = f
        .session
        .run(
            "grid.copyTsv",
            json!({"header": true, "anchor": [0, 0], "cursor": [1, 1]}),
        )
        .expect("copy");
    assert_eq!(out["tsv"].as_str().unwrap(), "a\tb\n1\t2\n4\t5\n");
}

#[test]
fn selection_reads_and_sets_the_rectangle() {
    let mut f = Fixture::new("selection");
    f.sql("CREATE TABLE t (a INTEGER, b INTEGER)");
    f.sql("INSERT INTO t VALUES (1,2),(3,4)");
    f.open_grid("t");
    // Setting: anchor and cursor define the rectangle; the corners come back normalised.
    let out = f
        .session
        .run(
            "grid.selection",
            json!({"anchor": [1, 1], "cursor": [0, 0]}),
        )
        .expect("set");
    assert_eq!(out["anchor"], json!([0, 0]), "{out:?}");
    assert_eq!(out["cursor"], json!([1, 1]), "{out:?}");
    assert_eq!(out["rows"], json!(2));
    assert_eq!(out["columns"], json!(2));
    // Reading back with no params returns the same rectangle.
    let out = f.session.run("grid.selection", json!({})).expect("read");
    assert_eq!(out["cursor"], json!([1, 1]));
    // A degenerate rectangle is one cell: rows 1, columns 1.
    let out = f
        .session
        .run(
            "grid.selection",
            json!({"anchor": [1, 1], "cursor": [1, 1]}),
        )
        .expect("collapse");
    assert_eq!(out["rows"], json!(1));
    assert_eq!(out["columns"], json!(1));
}

#[test]
fn selection_out_of_range_is_a_named_error() {
    let mut f = Fixture::new("selection-oob");
    f.sql("CREATE TABLE t (a INTEGER)");
    f.sql("INSERT INTO t VALUES (1)");
    f.open_grid("t");
    let err = f
        .session
        .run(
            "grid.selection",
            json!({"anchor": [0, 0], "cursor": [9, 0]}),
        )
        .unwrap_err();
    assert!(err.message().contains("9"), "must name the row: {err:?}");
    let err = f
        .session
        .run(
            "grid.selection",
            json!({"anchor": [0, 0], "cursor": [0, 5]}),
        )
        .unwrap_err();
    assert!(err.message().contains("5"), "must name the column: {err:?}");
}

#[test]
fn copy_tsv_junk_is_rejected() {
    let mut f = Fixture::new("copy-junk");
    f.sql("CREATE TABLE t (a INTEGER)");
    f.sql("INSERT INTO t VALUES (1)");
    f.open_grid("t");
    // A header that is neither boolean, number nor the accepted word forms is a named error.
    let err = f
        .session
        .run("grid.copyTsv", json!({"header": [1, 2]}))
        .unwrap_err();
    assert!(err.message().contains("header"), "{err:?}");
    let err = f
        .session
        .run("grid.selection", json!({"anchor": "x", "cursor": [0, 0]}))
        .unwrap_err();
    assert!(err.message().contains("anchor"), "{err:?}");
    // A three-item corner is rejected with the count.
    let err = f
        .session
        .run(
            "grid.selection",
            json!({"anchor": [0, 0, 0], "cursor": [0, 0]}),
        )
        .unwrap_err();
    assert!(err.message().contains("3"), "{err:?}");
}

#[test]
fn copy_tsv_without_params_uses_the_stored_selection() {
    let mut f = Fixture::new("copy-stored");
    f.sql("CREATE TABLE t (a INTEGER, b INTEGER, c INTEGER)");
    f.sql("INSERT INTO t VALUES (1,2,3),(4,5,6)");
    f.open_grid("t");
    // A selection made earlier (a drag, or grid.selection) is what a bare copy copies.
    f.session
        .run(
            "grid.selection",
            json!({"anchor": [0, 0], "cursor": [0, 1]}),
        )
        .expect("set selection");
    let out = f
        .session
        .run("grid.copyTsv", json!({"header": true}))
        .expect("copy");
    assert_eq!(out["tsv"].as_str().unwrap(), "a\tb\n1\t2\n");
    // Explicit corners still override the stored rectangle.
    let out = f
        .session
        .run(
            "grid.copyTsv",
            json!({"header": false, "anchor": [1, 2], "cursor": [1, 2]}),
        )
        .expect("copy");
    assert_eq!(out["tsv"].as_str().unwrap(), "6\n");
}

#[test]
fn hide_columns_persists_and_does_not_change_the_query() {
    let mut f = Fixture::new("hide-columns");
    f.sql("CREATE TABLE t (a INTEGER, b INTEGER, c INTEGER)");
    f.sql("INSERT INTO t VALUES (1,2,3)");
    f.open_grid("t");
    let out = f
        .session
        .run("grid.hideColumns", json!({"hidden": ["b"]}))
        .expect("hide");
    assert_eq!(out["hidden"], json!(["b"]), "{out:?}");
    // Hiding is a view concern: the page still holds every column.
    let page = f.session.run("grid.fetchPage", json!({})).expect("page");
    assert_eq!(
        page["columns"],
        json!(["a", "b", "c"]),
        "hiding must not change the query"
    );
    // Hidden columns survive a reopen: a fresh `grid.open` binds the table and reads the stored
    // list, which is the only way a preference outlives the tab that set it.
    f.open_grid("t");
    let reopened = f
        .session
        .run("grid.fetchPage", json!({}))
        .expect("page after reopen");
    assert_eq!(
        reopened["hidden"],
        json!(["b"]),
        "a reopened table must come back with the same columns hidden"
    );
    let out = f
        .session
        .run("grid.hideColumns", json!({}))
        .expect("read back");
    assert_eq!(out["hidden"], json!(["b"]), "{out:?}");
    // Unhiding by omission: an explicit empty list clears it.
    f.session
        .run("grid.hideColumns", json!({"hidden": []}))
        .expect("clear");
    let out = f
        .session
        .run("grid.hideColumns", json!({}))
        .expect("read back");
    assert_eq!(out["hidden"], json!([]), "{out:?}");
}

#[test]
fn hide_columns_rejects_unknown_names() {
    let mut f = Fixture::new("hide-unknown");
    f.sql("CREATE TABLE t (a INTEGER)");
    f.sql("INSERT INTO t VALUES (1)");
    f.open_grid("t");
    let err = f
        .session
        .run("grid.hideColumns", json!({"hidden": ["nope"]}))
        .unwrap_err();
    assert!(err.message().contains("nope"), "{err:?}");
    let err = f
        .session
        .run("grid.hideColumns", json!({"hidden": 7}))
        .unwrap_err();
    assert!(err.message().contains("hidden"), "{err:?}");
}

#[test]
fn quick_look_formats_json_hex_and_text() {
    let mut f = Fixture::new("quick-look");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY, doc TEXT, data BLOB, note TEXT, extra TEXT)");
    f.sql(
        "INSERT INTO t VALUES (1, '{\"a\": 1, \"b\": [2, 3]}', x'00ff10deadbeef', 'plain', NULL)",
    );
    f.open_grid("t");

    // Valid JSON is re-indented and says so.
    let out = f
        .session
        .run("grid.quickLook", json!({"row": 0, "column": "doc"}))
        .expect("json");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("{\n  \"a\": 1"), "{out:?}");
    assert_eq!(out["kind"], json!("json"));

    // A BLOB is hex, 16 bytes per line, with an offset column and an ASCII gutter.
    let out = f
        .session
        .run("grid.quickLook", json!({"row": 0, "column": "data"}))
        .expect("hex");
    let text = out["text"].as_str().unwrap();
    assert_eq!(out["kind"], json!("bytes"));
    assert!(text.contains("00000000  00 ff 10 de ad be ef"), "{text:?}");
    assert!(text.ends_with(".......\n"), "the ASCII gutter: {text:?}");

    // Plain text and a NULL both come back readable, never an error.
    let out = f
        .session
        .run("grid.quickLook", json!({"row": 0, "column": "note"}))
        .expect("text");
    assert_eq!(out["text"], json!("plain"));
    let out = f
        .session
        .run("grid.quickLook", json!({"row": 0, "column": "extra"}))
        .expect("null column reads fine");
    assert_eq!(out["kind"], json!("null"));
}

#[test]
fn quick_look_invalid_json_stays_verbatim_with_a_note() {
    let mut f = Fixture::new("quick-look-bad");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY, doc TEXT)");
    f.sql("INSERT INTO t VALUES (1, '{not json')");
    f.open_grid("t");
    let out = f
        .session
        .run("grid.quickLook", json!({"row": 0, "column": "doc"}))
        .expect("bad json is still a look, not an error");
    // Verbatim text with the note, because a json column holding bad text is
    // exactly when you need to look at it.
    assert_eq!(out["text"], json!("{not json"));
    assert!(
        out["note"].as_str().unwrap().contains("invalid JSON"),
        "{out:?}"
    );
    // Out-of-range coordinates are named errors.
    let err = f
        .session
        .run("grid.quickLook", json!({"row": 9, "column": "doc"}))
        .unwrap_err();
    assert!(err.message().contains("9"), "{err:?}");
}

#[test]
fn row_json_expands_one_foreign_key_level_and_stops() {
    let mut f = Fixture::new("row-json");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)");
    // A self-referencing FK: the expansion must stop after one level, or it recurses forever.
    f.sql("CREATE TABLE nodes (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES nodes(id), label TEXT)");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), total REAL)");
    f.sql("INSERT INTO customers (id, name) VALUES (1, 'Ada'), (2, 'Grace')");
    f.sql("INSERT INTO nodes (id, parent_id, label) VALUES (1, NULL, 'root'), (2, 1, 'child')");
    f.sql("INSERT INTO orders (id, customer_id, total) VALUES (10, 1, 9.5), (11, NULL, 1.0)");
    f.open_grid("orders");

    let out = f
        .session
        .run("grid.rowJson", json!({"row": 0, "expand": ["customer_id"]}))
        .expect("row as json");
    let row = &out["row"];
    assert_eq!(row["id"], json!(10), "{out:?}");
    assert_eq!(row["total"], json!(9.5));
    // The FK column becomes a reference document, not the bare scalar.
    let reference = &row["customer_id"];
    assert_eq!(reference["__ref__"], json!("customers"), "{out:?}");
    assert_eq!(reference["row"]["name"], json!("Ada"));
    // One level: the nested row carries its own columns and no expansions of its own.
    assert!(
        reference["row"].get("__ref__").is_none(),
        "the nested row must not expand further: {out:?}"
    );

    // A NULL FK expands to nothing at all: there is no row to point at.
    let out = f
        .session
        .run("grid.rowJson", json!({"row": 1, "expand": ["customer_id"]}))
        .expect("row as json");
    assert_eq!(out["row"]["customer_id"], json!(null), "{out:?}");

    // No `expand` list means the row's own keys: the UI has no catalog to name them from.
    let out = f
        .session
        .run("grid.rowJson", json!({"row": 0}))
        .expect("row as json");
    assert_eq!(
        out["row"]["customer_id"]["__ref__"],
        json!("customers"),
        "{out:?}"
    );

    // Self-reference: one level, and the nested row's own parent_id stays the bare scalar.
    f.open_grid("nodes");
    let out = f
        .session
        .run("grid.rowJson", json!({"row": 1, "expand": ["parent_id"]}))
        .expect("row as json");
    let parent = &out["row"]["parent_id"];
    assert_eq!(parent["__ref__"], json!("nodes"), "{out:?}");
    assert_eq!(parent["row"]["label"], json!("root"));
    // The nested row is root, whose own parent is NULL: it stays a scalar, never a second
    // expansion. That is what "one level" means.
    assert!(
        !parent["row"]["parent_id"].is_object(),
        "the nested row must not expand again: {out:?}"
    );
}

#[test]
fn row_json_reports_a_dangling_foreign_key_as_data() {
    let mut f = Fixture::new("row-json-dangling");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id))");
    // A row whose FK points at a customer that is not there.
    f.sql("PRAGMA foreign_keys = OFF");
    f.sql("INSERT INTO orders (id, customer_id) VALUES (10, 99)");
    f.open_grid("orders");

    let out = f
        .session
        .run("grid.rowJson", json!({"row": 0, "expand": ["customer_id"]}))
        .expect("a dangling key must not fail the command");
    let reference = &out["row"]["customer_id"];
    assert!(
        reference["__error__"].is_string(),
        "a dangling key is data, not an error: {out:?}"
    );
    assert!(
        reference["__error__"].as_str().unwrap().contains("99"),
        "the sentence names the value that matched nothing: {out:?}"
    );
}

#[test]
fn follow_foreign_key_resolves_the_target_row_and_says_where() {
    let mut f = Fixture::new("follow-fk");
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id))");
    f.sql("INSERT INTO customers (id, name) VALUES (7, 'Ada')");
    f.sql("INSERT INTO orders (id, customer_id) VALUES (10, 7)");
    f.open_grid("orders");

    let out = f
        .session
        .run(
            "grid.followForeignKey",
            json!({"row": 0, "column": "customer_id"}),
        )
        .expect("a keyed target resolves");
    assert_eq!(out["table"], json!("customers"), "{out:?}");
    // Quoted, because a column name is an identifier and the filter is raw SQL.
    assert_eq!(out["where"], json!("\"id\" = 7"), "{out:?}");
    assert_eq!(out["conn"], json!(f.conn));
    assert_eq!(out["schema"], json!(null));

    // A column that is not a foreign key is a named error, not a silent no-op.
    let err = f
        .session
        .run("grid.followForeignKey", json!({"row": 0, "column": "id"}))
        .expect_err("id is not a foreign key");
    assert!(
        err.to_string().contains("foreign key"),
        "the refusal must name the reason: {err}"
    );

    // A NULL key has nowhere to go, and says so rather than opening an empty filter.
    f.sql("INSERT INTO orders (id, customer_id) VALUES (11, NULL)");
    f.open_grid("orders");
    let err = f
        .session
        .run(
            "grid.followForeignKey",
            json!({"row": 1, "column": "customer_id"}),
        )
        .expect_err("a NULL key points nowhere");
    assert!(err.to_string().contains("NULL"), "{err}");
}

#[test]
fn a_foreign_key_that_names_no_column_resolves_against_the_parent_key() {
    let mut f = Fixture::new("follow-fk-implicit-pk");
    // `REFERENCES customers` names no column: SQLite reports an empty `to` and means the parent's
    // primary key. Quoting the empty identifier would produce `"" = 1` and match nothing.
    f.sql("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, customer INTEGER REFERENCES customers)");
    f.sql("INSERT INTO customers (id, name) VALUES (7, 'Ada')");
    f.sql("INSERT INTO orders (id, customer) VALUES (1, 7)");
    f.open_grid("orders");

    let out = f
        .session
        .run(
            "grid.followForeignKey",
            json!({"row": 0, "column": "customer"}),
        )
        .expect("an implicit primary key still resolves");
    assert_eq!(out["table"], json!("customers"), "{out:?}");
    assert_eq!(out["where"], json!("\"id\" = 7"), "{out:?}");
}

#[test]
fn follow_foreign_key_falls_back_to_every_column_without_a_primary_key() {
    let mut f = Fixture::new("follow-fk-nokey");
    // A keyless table: the command still resolves the row, but its WHERE names every column.
    f.sql("CREATE TABLE tags (name TEXT, color TEXT)");
    f.sql("CREATE TABLE notes (id INTEGER PRIMARY KEY, tag TEXT REFERENCES tags(name))");
    f.sql("INSERT INTO tags (name, color) VALUES ('urgent', 'red')");
    f.sql("INSERT INTO notes (id, tag) VALUES (1, 'urgent')");
    f.open_grid("notes");

    let out = f
        .session
        .run("grid.followForeignKey", json!({"row": 0, "column": "tag"}))
        .expect("a keyless target still resolves");
    assert_eq!(out["table"], json!("tags"), "{out:?}");
    let where_clause = out["where"].as_str().expect("a WHERE clause");
    assert!(
        where_clause.contains("name") && where_clause.contains("color"),
        "the fallback names every column: {where_clause}"
    );
    assert_eq!(
        out["note"],
        json!("tags has no primary key — matching on every column"),
        "the fallback must say so: {out:?}"
    );

    // A NULL column is `IS NULL`, not `= NULL`: the latter matches no row in any dialect, so a
    // keyless target holding one NULL would filter the grid down to nothing.
    f.sql("INSERT INTO tags (name, color) VALUES ('later', NULL)");
    f.sql("INSERT INTO notes (id, tag) VALUES (2, 'later')");
    f.open_grid("notes");
    let out = f
        .session
        .run("grid.followForeignKey", json!({"row": 1, "column": "tag"}))
        .expect("a keyless target with a NULL column still resolves");
    let where_clause = out["where"].as_str().expect("a WHERE clause");
    assert!(
        where_clause.contains("IS NULL") && !where_clause.contains("= NULL"),
        "a NULL must be matched with IS NULL: {where_clause}"
    );
}

#[test]
fn blob_editor_refuses_non_ascii_hex_instead_of_panicking() {
    let mut f = Fixture::new("blob-editor-nonascii");
    f.sql("CREATE TABLE files (id INTEGER PRIMARY KEY, body BLOB)");
    f.sql("INSERT INTO files (id, body) VALUES (1, x'00ff')");
    f.open_grid("files");

    // `é` is two bytes, so slicing the string into pairs would split it and abort the process.
    let error = f
        .session
        .run(
            "grid.blobEditor",
            json!({"row": 0, "column": "body", "hex": "é0"}),
        )
        .expect_err("non-ASCII hex is a named error");
    assert!(error.to_string().contains("ASCII"), "{error}");
}

#[test]
fn blob_editor_decodes_hex_into_a_pending_edit() {
    let mut f = Fixture::new("blob-editor");
    f.sql("CREATE TABLE files (id INTEGER PRIMARY KEY, body BLOB)");
    f.sql("INSERT INTO files (id, body) VALUES (1, x'00ff10')");
    f.open_grid("files");

    let out = f
        .session
        .run(
            "grid.blobEditor",
            json!({"row": 0, "column": "body", "hex": "deadbeef"}),
        )
        .expect("valid hex stages an edit");
    assert_eq!(out["ok"], json!(true), "{out:?}");
    assert_eq!(out["bytes"], json!(4), "four bytes decoded: {out:?}");

    // The edit is staged like any other, so nothing is written until `grid.apply`.
    let pending = f
        .session
        .run("grid.pending", json!({}))
        .expect("the pending list reads");
    assert_eq!(
        pending["pending"].as_array().map(Vec::len),
        Some(1),
        "the blob edit must be one pending statement: {pending:?}"
    );
    let applied = f.session.run("grid.apply", json!({})).expect("apply runs");
    assert_eq!(applied["applied"], json!(1), "{applied:?}");
    let read = f.sql("SELECT hex(body) AS body FROM files WHERE id = 1");
    assert_eq!(
        read["objects"][0]["body"],
        json!("DEADBEEF"),
        "the applied bytes round-trip: {read:?}"
    );
}

#[test]
fn blob_editor_rejects_junk_hex_and_oversized_blobs() {
    let mut f = Fixture::new("blob-editor-junk");
    f.sql("CREATE TABLE files (id INTEGER PRIMARY KEY, body BLOB)");
    f.sql("INSERT INTO files (id, body) VALUES (1, x'00')");
    f.open_grid("files");

    // Odd length and a non-hex pair are both named errors, never a silent truncation. Whitespace
    // is not junk: it is stripped first, so a dump pasted with spaces still decodes.
    for bad in ["abc", "zz", "de gg", "\u{e9}0"] {
        let err = f
            .session
            .run(
                "grid.blobEditor",
                json!({"row": 0, "column": "body", "hex": bad}),
            )
            .expect_err("junk hex is refused");
        assert!(
            err.to_string().to_lowercase().contains("hex"),
            "the refusal must name hex, got {err} for {bad:?}"
        );
    }

    // The cap is a setting, and the refusal names the size it would have loaded.
    f.session
        .run(
            "app.setSetting",
            json!({"key": "max_blob_bytes", "value": "2"}),
        )
        .expect("the cap is a preference");
    let err = f
        .session
        .run(
            "grid.blobEditor",
            json!({"row": 0, "column": "body", "hex": "00112233"}),
        )
        .expect_err("an oversized blob is refused");
    assert!(
        err.to_string().contains("4") && err.to_string().contains("2"),
        "the refusal names both sizes, got {err}"
    );
}

#[test]
fn display_hints_read_the_format_marker_in_the_column_name() {
    let mut f = Fixture::new("hints");
    f.sql("CREATE TABLE shop (id INTEGER PRIMARY KEY, price__format__money REAL, active__format__check INTEGER, site__format__link TEXT, plain TEXT)");
    f.sql("INSERT INTO shop (id, price__format__money, active__format__check, site__format__link, plain) VALUES (1, 12.5, 1, 'https://example.com', 'x'), (2, 7.0, 0, NULL, 'y')");
    f.open_grid("shop");

    let out = f
        .session
        .run("grid.hints", json!({}))
        .expect("hints read from the page's columns");
    let hints = out["hints"].as_array().expect("a hint list");
    assert_eq!(
        hints.len(),
        3,
        "only the marked columns carry a hint: {out:?}"
    );
    let find = |name: &str| {
        hints
            .iter()
            .find(|h| h["column"] == json!(name))
            .cloned()
            .unwrap_or_else(|| panic!("no hint for {name}: {out:?}"))
    };
    assert_eq!(find("price__format__money")["kind"], json!("money"));
    assert_eq!(find("active__format__check")["kind"], json!("check"));
    assert_eq!(find("site__format__link")["kind"], json!("link"));
    // The marker is stripped for display: the header shows the real name.
    assert_eq!(find("price__format__money")["label"], json!("price"));
    // A hint is a view concern: the query still returns every column.
    let page = f
        .session
        .run("grid.fetchPage", json!({}))
        .expect("the page reads");
    assert_eq!(
        page["columns"].as_array().map(Vec::len),
        Some(5),
        "hints must not change the query: {page:?}"
    );
}
