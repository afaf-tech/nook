//! Phase 5, steps 1–9: import and export at scale.
//!
//! Every case asserts what the database actually holds afterwards, because the failure this phase
//! exists to prevent is an import that reports success while having truncated, duplicated or
//! silently skipped rows.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
    dir: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("nook-phase5-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
        let path = dir.join("main.db");
        let cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", path.display()))
            .expect("a sqlite URI parses");
        let conn = cfg.id.clone();
        let mut session = Session::new();
        session.conns.push(Conn::new(cfg));
        let index = session.conn_index(&conn).expect("registered");
        open_now(&mut session, index).expect("sqlite opens");
        Fixture { session, conn, dir }
    }

    fn file(&self, name: &str, contents: &str) -> String {
        let path = self.dir.join(name);
        std::fs::write(&path, contents).expect("write fixture");
        path.to_string_lossy().into_owned()
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

    fn sql(&mut self, sql: &str) -> serde_json::Value {
        self.run("sql.run", json!({"sql": sql, "conn": self.conn}))
    }

    /// A second, independent connection: the sync commands refuse one connection used twice, and
    /// rightly so — a "sync" from a database to itself proves nothing.
    fn second(&mut self, name: &str) -> String {
        let path = self.dir.join(format!("{name}.db"));
        let _ = std::fs::remove_file(&path);
        let cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", path.display()))
            .expect("a sqlite URI parses");
        let id = cfg.id.clone();
        self.session.conns.push(Conn::new(cfg));
        let index = self.session.conn_index(&id).expect("registered");
        open_now(&mut self.session, index).expect("sqlite opens");
        id
    }

    fn count(&mut self, table: &str) -> i64 {
        let out = self.sql(&format!("SELECT COUNT(*) AS n FROM {table}"));
        out["objects"][0]["n"].as_i64().unwrap_or(-1)
    }
}

// ---------------------------------------------------------------- step 1: the reader

#[test]
fn a_quoted_csv_with_embedded_newlines_imports_its_cells_intact() {
    let mut f = Fixture::new("quotes");
    let path = f.file(
        "in.csv",
        "id,note\n1,\"a, b\"\n2,\"line\nbreak\"\n3,\"he said \"\"hi\"\"\"\n",
    );
    f.run(
        "tools.importCsv",
        json!({"conn": f.conn.clone(), "path": path, "table": "notes"}),
    );
    let out = f.sql("SELECT id, note FROM notes ORDER BY id");
    assert_eq!(out["objects"][0]["note"], json!("a, b"));
    assert_eq!(out["objects"][1]["note"], json!("line\nbreak"));
    assert_eq!(out["objects"][2]["note"], json!("he said \"hi\""));
}

#[test]
fn a_bom_does_not_become_part_of_the_first_column_name() {
    let mut f = Fixture::new("bom");
    let path = f.dir.join("bom.csv");
    std::fs::write(&path, b"\xEF\xBB\xBFid,name\n1,Ada\n").expect("write");
    f.run(
        "tools.importCsv",
        json!({"conn": f.conn.clone(), "path": path.to_string_lossy(), "table": "people"}),
    );
    let out = f.sql("SELECT id FROM people");
    assert_eq!(
        out["columns"],
        json!(["id"]),
        "the BOM leaked into the header: {out}"
    );
}

// ---------------------------------------------------------------- step 2: chunking

#[test]
fn a_failure_in_the_second_batch_leaves_the_first_committed_and_says_so() {
    let mut f = Fixture::new("batch");
    // The table exists first, with a real primary key: `create_table` from a CSV makes every
    // column TEXT, and a duplicate key is exactly what the failure path is about.
    f.sql("CREATE TABLE big (id INTEGER PRIMARY KEY, name TEXT)");
    // 3000 good rows, then a duplicate primary key. Batch 1000 means three committed batches
    // before the failure; the report must not read like nothing landed.
    let mut text = String::from("id,name\n");
    for i in 1..=3000 {
        text.push_str(&format!("{i},row{i}\n"));
    }
    text.push_str("1,duplicate\n");
    let path = f.file("big.csv", &text);
    let out = f.run(
        "tools.importCsv",
        json!({
            "conn": f.conn.clone(), "path": path, "table": "big",
            "batch": 1000, "create_table": false,
        }),
    );
    assert_eq!(out["inserted"], json!(3000), "{out}");
    assert_eq!(out["batches"], json!(3), "{out}");
    assert_eq!(out["failed_at_row"], json!(3001), "{out}");
    assert!(
        out["notes"][0]
            .as_str()
            .unwrap_or("")
            .contains("stay committed"),
        "the report must say the committed rows stay: {out}"
    );
    assert_eq!(
        f.count("big"),
        3000,
        "the committed batches were rolled back"
    );
}

#[test]
fn batches_are_committed_separately_not_in_one_transaction() {
    let mut f = Fixture::new("batches");
    let mut text = String::from("id\n");
    for i in 1..=250 {
        text.push_str(&format!("{i}\n"));
    }
    let path = f.file("rows.csv", &text);
    let out = f.run(
        "tools.importCsv",
        json!({"conn": f.conn.clone(), "path": path, "table": "rows", "batch": 100}),
    );
    assert_eq!(
        out["batches"],
        json!(3),
        "250 rows in batches of 100: {out}"
    );
    assert_eq!(f.count("rows"), 250);
}

// ---------------------------------------------------------------- step 3: modes

#[test]
fn a_truncate_that_cannot_read_the_header_does_not_empty_the_table() {
    let mut f = Fixture::new("truncate-guard");
    f.sql("CREATE TABLE keepme (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("INSERT INTO keepme (id, name) VALUES (1, 'still here')");
    let path = f.file("empty.csv", "");
    let err = f
        .try_run(
            "tools.importCsv",
            json!({
                "conn": f.conn.clone(), "path": path, "table": "keepme", "mode": "truncate",
            }),
        )
        .expect_err("an empty file has no header");
    assert!(err.message().contains("no rows"), "{err}");
    assert_eq!(
        f.count("keepme"),
        1,
        "a truncate ran before the file was validated"
    );
}

#[test]
fn truncate_empties_the_table_then_inserts() {
    let mut f = Fixture::new("truncate");
    f.sql("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("INSERT INTO items (id, name) VALUES (9, 'old')");
    let path = f.file("items.csv", "id,name\n1,fresh\n2,also\n");
    f.run(
        "tools.importCsv",
        json!({
            "conn": f.conn.clone(), "path": path, "table": "items", "mode": "truncate",
        }),
    );
    let out = f.sql("SELECT id FROM items ORDER BY id");
    assert_eq!(
        out["objects"],
        json!([{"id": 1}, {"id": 2}]),
        "the old row survived"
    );
}

#[test]
fn upsert_without_a_key_is_refused_rather_than_degraded_to_insert() {
    let mut f = Fixture::new("upsert-nokey");
    f.sql("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE)");
    let path = f.file("u.csv", "id,email\n1,a@b.c\n");
    let err = f
        .try_run(
            "tools.importCsv",
            json!({
                "conn": f.conn.clone(), "path": path, "table": "users", "mode": "upsert",
            }),
        )
        .expect_err("upsert needs a key");
    assert!(err.message().contains("key"), "{err}");
    assert_eq!(f.count("users"), 0);
}

#[test]
fn upsert_on_a_column_that_is_not_unique_is_refused_naming_it() {
    let mut f = Fixture::new("upsert-nonunique");
    f.sql("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)");
    let path = f.file("u.csv", "id,email\n1,a@b.c\n");
    let err = f
        .try_run(
            "tools.importCsv",
            json!({
                "conn": f.conn.clone(), "path": path, "table": "users",
                "mode": "upsert", "key": ["email"],
            }),
        )
        .expect_err("email is not unique");
    assert!(err.message().contains("email"), "{err}");
    assert!(err.message().contains("not unique"), "{err}");
}

#[test]
fn a_re_import_with_upsert_updates_instead_of_duplicating() {
    let mut f = Fixture::new("upsert");
    f.sql("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE, name TEXT)");
    let first = f.file("one.csv", "id,email,name\n1,a@b.c,Ada\n2,g@b.c,Grace\n");
    f.run(
        "tools.importCsv",
        json!({
            "conn": f.conn.clone(), "path": first, "table": "users",
            "mode": "upsert", "key": ["id"],
        }),
    );
    assert_eq!(f.count("users"), 2);
    let second = f.file("two.csv", "id,email,name\n1,a@b.c,Ada Lovelace\n");
    let out = f.run(
        "tools.importCsv",
        json!({
            "conn": f.conn.clone(), "path": second, "table": "users",
            "mode": "upsert", "key": ["id"],
        }),
    );
    // The file's single row already existed, so the table did not grow: reporting "1 inserted"
    // here was the counter measuring statements instead of rows.
    assert_eq!(out["inserted"], json!(0), "{out}");
    assert_eq!(out["updated"], json!(1), "{out}");
    assert_eq!(f.count("users"), 2, "the upsert inserted a duplicate");
    let row = f.sql("SELECT name FROM users WHERE id = 1");
    assert_eq!(row["objects"][0]["name"], json!("Ada Lovelace"));
}

// ---------------------------------------------------------------- step 4: export.stream

#[test]
fn export_stream_writes_more_rows_than_the_grid_cap() {
    let mut f = Fixture::new("stream");
    f.sql("CREATE TABLE wide (id INTEGER PRIMARY KEY)");
    // Past the 50,000-row grid cap on purpose: the export must not inherit it.
    f.sql(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 60000) \
         INSERT INTO wide (id) SELECT i FROM n",
    );
    let path = f.dir.join("wide.csv");
    let out = f.run(
        "export.stream",
        json!({
            "conn": f.conn.clone(), "table": "wide", "path": path.to_string_lossy(),
            "format": "csv", "page": 5000,
        }),
    );
    assert_eq!(out["rows"], json!(60_000), "{out}");
    assert_eq!(out["pages"], json!(12), "{out}");
    let text = std::fs::read_to_string(&path).expect("the file exists");
    // The header plus 60,000 rows.
    assert_eq!(text.lines().count(), 60_001);
}

#[test]
fn export_stream_refuses_json_and_points_at_jsonl() {
    let mut f = Fixture::new("stream-json");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY)");
    let err = f
        .try_run(
            "export.stream",
            json!({"conn": f.conn.clone(), "table": "t", "format": "json"}),
        )
        .expect_err("json cannot stream");
    assert!(err.message().contains("jsonl"), "{err}");
}

#[test]
fn export_stream_writes_one_json_object_per_line() {
    let mut f = Fixture::new("stream-jsonl");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')");
    let path = f.dir.join("t.jsonl");
    f.run(
        "export.stream",
        json!({
            "conn": f.conn.clone(), "table": "t", "path": path.to_string_lossy(),
            "format": "jsonl",
        }),
    );
    let text = std::fs::read_to_string(&path).expect("file");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).expect("one object per line");
        assert!(value.get("id").is_some(), "{line}");
    }
}

#[test]
fn export_query_with_stream_pages_the_statement() {
    let mut f = Fixture::new("stream-query");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY)");
    f.sql(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 12000) \
         INSERT INTO t (id) SELECT i FROM n",
    );
    let path = f.dir.join("q.csv");
    let out = f.run(
        "tools.exportQuery",
        json!({
            "conn": f.conn.clone(), "sql": "select id from t order by id",
            "path": path.to_string_lossy(), "stream": true, "page": 3000,
        }),
    );
    assert_eq!(out["rows"], json!(12_000), "{out}");
    assert_eq!(out["pages"], json!(4), "{out}");
}

// ---------------------------------------------------------------- step 5: the wizard

#[test]
fn the_wizard_writes_nothing_and_reports_the_mapping() {
    let mut f = Fixture::new("wizard");
    f.sql("CREATE TABLE people (id INTEGER PRIMARY KEY, email TEXT, joined TEXT)");
    f.sql("INSERT INTO people (id, email, joined) VALUES (99, 'old@x.y', '2020-01-01')");
    let path = f.file(
        "p.csv",
        "id,mail,joined\n1,a@b.c,2024-01-31\n2,d@e.f,2024-02-01\n",
    );
    let out = f.run(
        "tools.importWizard",
        json!({"conn": f.conn.clone(), "path": path, "table": "people"}),
    );
    assert_eq!(out["writes"], json!(0), "{out}");
    assert_eq!(out["target_exists"], json!(true));
    assert_eq!(f.count("people"), 1, "the wizard wrote a row");
    let columns = out["columns"].as_array().expect("columns");
    let id = columns
        .iter()
        .find(|c| c["source"] == json!("id"))
        .expect("id");
    assert_eq!(id["inferred"], json!("integer"));
    let mail = columns
        .iter()
        .find(|c| c["source"] == json!("mail"))
        .expect("mail");
    assert_eq!(
        mail["target"],
        json!(null),
        "a column the table lacks must not claim a target"
    );
    let notes = out["notes"].as_array().expect("notes");
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("mail")),
        "the wizard must say 'mail' would be skipped: {out}"
    );
    let preview = out["preview"].as_array().expect("preview");
    assert_eq!(preview.len(), 2);
}

#[test]
fn the_wizard_infers_dates_and_reports_an_empty_column() {
    let mut f = Fixture::new("wizard-types");
    let path = f.file(
        "t.csv",
        "day,when,amount,blank\n2024-01-31,2024-01-31T10:00:00,1.5,\n2024-02-01,2024-02-01 11:00:00,2.5,\n",
    );
    let out = f.run(
        "tools.importWizard",
        json!({"conn": f.conn.clone(), "path": path, "table": "t"}),
    );
    let columns = out["columns"].as_array().expect("columns");
    let by = |name: &str| {
        columns
            .iter()
            .find(|c| c["source"] == json!(name))
            .unwrap_or_else(|| panic!("{name}"))
            .clone()
    };
    assert_eq!(by("day")["inferred"], json!("date"));
    assert_eq!(by("when")["inferred"], json!("timestamp"));
    assert_eq!(by("amount")["inferred"], json!("float"));
    assert_eq!(by("blank")["always_empty"], json!(true));
    let notes = out["notes"].as_array().expect("notes");
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("blank")),
        "{out}"
    );
}

// ---------------------------------------------------------------- step 7: backup filters

#[test]
fn a_compressed_backup_is_a_real_gzip_file() {
    let mut f = Fixture::new("gzip");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY)");
    f.sql("INSERT INTO t (id) VALUES (1)");
    let path = f.dir.join("dump.sql");
    let out = f.try_run(
        "backup.compression",
        json!({
            "conn": f.conn.clone(), "path": path.to_string_lossy(), "compress": true,
        }),
    );
    let Ok(out) = out else {
        // Compression shells out to `gzip`, and the refusal happens before any rename: the plain dump
        // is written, nothing claims the compressed name, and the error names the missing tool.
        let err = out.unwrap_err();
        assert!(err.message().contains("gzip"), "{err}");
        assert!(
            path.exists(),
            "the uncompressed dump survives a refused filter"
        );
        assert!(!path.with_file_name("dump.sql.gz").exists(), "no empty .gz");
        return;
    };
    let written = out["path"].as_str().expect("path");
    assert!(written.ends_with(".gz"), "{out}");
    let bytes = std::fs::read(written).expect("the .gz exists");
    assert_eq!(&bytes[..2], &[0x1f, 0x8b], "not a gzip stream");
    assert!(!path.exists(), "the plain file was left behind");
}

#[test]
fn encryption_without_the_binary_is_refused_and_leaves_no_file() {
    let mut f = Fixture::new("encrypt-missing");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY)");
    let path = f.dir.join("dump.sql");
    let result = f.try_run(
        "backup.encryption",
        json!({
            "conn": f.conn.clone(), "path": path.to_string_lossy(),
            "encrypt": "age", "recipient": "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq",
        }),
    );
    match result {
        Ok(out) => {
            // `age` is installed: then the file must be a real age file and the plain one gone.
            let written = out["path"].as_str().unwrap_or_default();
            assert!(written.ends_with(".age"), "{out}");
            let bytes = std::fs::read(written).expect("the .age exists");
            assert!(bytes.starts_with(b"age-encryption.org"), "not an age file");
        }
        Err(e) => {
            assert!(
                e.message().contains("not on this machine"),
                "the refusal must name the missing tool: {e}"
            );
            // Whatever happened, no unencrypted file may be sitting under the encrypted name.
            assert!(!path.with_file_name("dump.sql.age").exists());
        }
    }
}

#[test]
fn keeping_three_copies_prunes_the_older_ones() {
    let mut f = Fixture::new("keep");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY)");
    let keep = f.dir.join("offsite");
    for i in 0..5 {
        // Distinct content so each file is a separate backup, and a distinct mtime.
        f.sql(&format!("INSERT INTO t (id) VALUES ({})", i + 1));
        let path = f.dir.join(format!("dump-{i}.sql"));
        f.run(
            "backup.local",
            json!({
                "conn": f.conn.clone(), "path": path.to_string_lossy(),
                "keep": {"dir": keep.to_string_lossy(), "copies": 3},
            }),
        );
    }
    let mut files: Vec<String> = std::fs::read_dir(&keep)
        .expect("keep dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files.len(), 3, "prune left {files:?}");
    assert!(
        files.iter().all(|n| n.contains("dump-")),
        "a stray file was pruned or kept: {files:?}"
    );
}

// ---------------------------------------------------------------- step 8: transfer and sync

#[test]
fn a_speed_probe_writes_nothing() {
    let mut f = Fixture::new("speed");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)");
    for i in 1..=500 {
        f.sql(&format!("INSERT INTO t (id, name) VALUES ({i}, 'row{i}')"));
    }
    let dst = f.second("speed-dst");
    let out = f.run(
        "transfer.speedTest",
        json!({"from": f.conn.clone(), "to": dst, "table": "t", "rows": 200}),
    );
    assert_eq!(out["rows"], json!(200), "{out}");
    assert_eq!(out["wrote"], json!(0));
    assert_eq!(f.count("t"), 500);
    // The scratch table was created in the destination and rolled back: nothing may survive.
    let leftovers = f.run(
        "sql.run",
        json!({
            "conn": dst,
            "sql": "SELECT name FROM sqlite_master WHERE name LIKE 'nook_speed_%'",
        }),
    );
    assert_eq!(
        leftovers["objects"],
        json!([]),
        "the scratch table survived the rollback"
    );
}

#[test]
fn the_delete_half_is_a_dry_run_until_confirmed() {
    let mut f = Fixture::new("delete");
    f.sql("CREATE TABLE a (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("CREATE TABLE b (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("INSERT INTO a (id, name) VALUES (1, 'kept')");
    let dst = f.second("delete-dst");
    f.run(
        "sql.run",
        json!({
            "conn": dst,
            "sql": "CREATE TABLE a (id INTEGER PRIMARY KEY, name TEXT)",
        }),
    );
    f.run(
        "sql.run",
        json!({
            "conn": dst,
            "sql": "INSERT INTO a (id, name) VALUES (1, 'kept'), (2, 'only in the target')",
        }),
    );
    // The source has row 1 only, so row 2 is a delete.
    let dry = f.run(
        "sync.dataDelete",
        json!({"from": f.conn.clone(), "to": dst.clone(), "tables": ["a"]}),
    );
    assert_eq!(dry["dry_run"], json!(true), "{dry}");
    assert_eq!(dry["deletes"], json!(1), "{dry}");
    let still = f.run(
        "sql.run",
        json!({"conn": dst.clone(), "sql": "SELECT COUNT(*) AS n FROM a"}),
    );
    assert_eq!(
        still["objects"][0]["n"],
        json!(2),
        "a dry run deleted a row"
    );

    let applied = f.run(
        "sync.dataDelete",
        json!({
            "from": f.conn.clone(), "to": dst.clone(), "tables": ["a"],
            "confirm": true, "dry_run": false,
        }),
    );
    assert_eq!(applied["applied"], json!(1), "{applied}");
    let after = f.run("sql.run", json!({"conn": dst, "sql": "SELECT id FROM a"}));
    assert_eq!(
        after["objects"],
        json!([{"id": 1}]),
        "row 2 was not deleted"
    );
}

#[test]
fn conflict_rules_are_validated_before_a_sync_uses_them() {
    let mut f = Fixture::new("rules");
    let ok = f.run(
        "sync.conflictRules",
        json!({"rules": [
            {"column": "total", "on_conflict": "newer", "by": "updated_at"},
            {"column": "note", "on_conflict": "skip"},
        ]}),
    );
    assert_eq!(ok["count"], json!(2), "{ok}");
    let err = f
        .try_run(
            "sync.conflictRules",
            json!({"rules": [{"column": "total", "on_conflict": "newer"}]}),
        )
        .expect_err("newer without by is refused");
    assert!(err.message().contains("by"), "{err}");
    let err = f
        .try_run(
            "sync.conflictRules",
            json!({"rules": [{"column": "x", "on_conflict": "coin flip"}]}),
        )
        .expect_err("an unknown rule is refused");
    assert!(err.message().contains("coin flip"), "{err}");
}

#[test]
fn a_bidirectional_sync_reports_a_row_it_cannot_settle() {
    let mut f = Fixture::new("bidir");
    f.sql("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)");
    f.sql("INSERT INTO t (id, name) VALUES (1, 'one')");
    let b = f.second("bidir-b");
    f.run(
        "sql.run",
        json!({"conn": b.clone(), "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)"}),
    );
    f.run(
        "sql.run",
        json!({"conn": b.clone(), "sql": "INSERT INTO t (id, name) VALUES (1, 'one'), (2, 'two')"}),
    );
    // Row 1 matches. Row 2 is on b only: from a→b it is a delete, from b→a an insert. Those are
    // the same evidence read two ways, so the row is a conflict — not a merge, and not a side
    // picked by whoever ran the command last.
    // `dry_run: false` alone must not be enough: the gate is `confirm: true`.
    let ungated = f.run(
        "sync.bidirectional",
        json!({"a": f.conn.clone(), "b": b.clone(), "tables": ["t"], "dry_run": false}),
    );
    assert_eq!(ungated["dry_run"], json!(true), "{ungated}");
    assert_eq!(ungated["applied"], json!(0), "{ungated}");
    assert_eq!(f.count("t"), 1, "an unconfirmed sync wrote to the source");

    let out = f.run(
        "sync.bidirectional",
        json!({
            "a": f.conn.clone(), "b": b.clone(), "tables": ["t"],
            "dry_run": false, "confirm": true,
        }),
    );
    assert_eq!(out["a_to_b"], json!(0), "{out}");
    assert_eq!(out["b_to_a"], json!(0), "{out}");
    assert_eq!(out["applied"], json!(0), "{out}");
    let conflicts = out["conflicts"].as_array().expect("conflicts");
    assert_eq!(conflicts.len(), 1, "{out}");
    assert!(
        conflicts[0]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("on one side only"),
        "the conflict must name the ambiguity: {out}"
    );
    assert_eq!(f.count("t"), 1, "a conflict must not have written anything");
}

#[test]
fn the_column_mapping_command_normalises_editor_rows() {
    let mut f = Fixture::new("mapping");
    let out = f.run(
        "transfer.columnMapping",
        json!({"map": [
            {"source": "mail", "target": "email"},
            {"source": "joined", "target": "created_at"},
        ]}),
    );
    assert_eq!(
        out["columnMapping"],
        json!(["mail=email", "joined=created_at"])
    );
    let err = f
        .try_run(
            "transfer.columnMapping",
            json!({"map": [
                {"source": "a", "target": "x"},
                {"source": "b", "target": "x"},
            ]}),
        )
        .expect_err("two sources into one target");
    assert!(err.message().contains("one source"), "{err}");
}

// ---------------------------------------------------------------- step 9: XML and fixed width

#[test]
fn a_flat_xml_file_imports() {
    let mut f = Fixture::new("xml");
    let path = f.file(
        "rows.xml",
        "<rows>\n<record><id>1</id><name>Ada</name></record>\n\
         <record><id>2</id><name>Grace &amp; co</name></record>\n</rows>",
    );
    let out = f.run(
        "tools.importXml",
        json!({"conn": f.conn.clone(), "path": path, "table": "people"}),
    );
    assert_eq!(out["inserted"], json!(2), "{out}");
    let rows = f.sql("SELECT name FROM people ORDER BY id");
    assert_eq!(rows["objects"][1]["name"], json!("Grace & co"));
}

#[test]
fn a_nested_xml_file_is_refused_naming_the_element() {
    let mut f = Fixture::new("xml-nested");
    let path = f.file(
        "n.xml",
        "<rows><record><id>1</id><addr><city>x</city></addr></record></rows>",
    );
    let err = f
        .try_run(
            "tools.importXml",
            json!({"conn": f.conn.clone(), "path": path, "table": "people"}),
        )
        .expect_err("nested xml is refused");
    assert!(err.message().contains("city"), "{err}");
}

#[test]
fn a_fixed_width_file_splits_on_the_given_widths() {
    let mut f = Fixture::new("fixed");
    // 3 + 12 + 8 characters per line, no delimiter anywhere.
    let path = f.file(
        "codes.txt",
        "001Ada Lovelace20240131\n002Grace Hopper20240201\n",
    );
    let out = f.run(
        "tools.textImporter",
        json!({
            "conn": f.conn.clone(), "path": path, "table": "codes",
            "widths": [3, 12, 8],
        }),
    );
    assert_eq!(out["inserted"], json!(2), "{out}");
    let rows =
        f.sql("SELECT column_1 AS code, column_2 AS name, column_3 AS day FROM codes ORDER BY 1");
    assert_eq!(rows["objects"][0]["code"], json!("001"));
    assert_eq!(rows["objects"][0]["name"], json!("Ada Lovelace"));
    assert_eq!(rows["objects"][0]["day"], json!("20240131"));
}

#[test]
fn a_fixed_width_file_without_widths_is_refused_with_the_shape_to_pass() {
    let mut f = Fixture::new("fixed-nowidths");
    let path = f.file("c.txt", "001x\n");
    let err = f
        .try_run(
            "tools.textImporter",
            json!({"conn": f.conn.clone(), "path": path, "table": "codes"}),
        )
        .expect_err("widths are required");
    assert!(err.message().contains("widths"), "{err}");
}

// ---------------------------------------------------------------- jsonl

#[test]
fn a_jsonl_file_streams_line_by_line() {
    let mut f = Fixture::new("jsonl");
    let path = f.file(
        "rows.jsonl",
        "{\"id\":1,\"name\":\"Ada\"}\n\n{\"id\":2,\"name\":\"Grace\"}\n",
    );
    let out = f.run(
        "tools.importJson",
        json!({"conn": f.conn.clone(), "path": path, "table": "people"}),
    );
    assert_eq!(out["format"], json!("jsonl"), "{out}");
    assert_eq!(out["inserted"], json!(2), "{out}");
    assert_eq!(f.count("people"), 2);
}

// ---------------------------------------------------------------- dbf

/// A dBASE III file, built here so the bytes the parser depends on are visible in the test.
fn dbf(fields: &[(&str, u8, usize)], records: &[&[&str]]) -> Vec<u8> {
    let record_length: usize = 1 + fields.iter().map(|f| f.2).sum::<usize>();
    let header_length = 32 + fields.len() * 32 + 1;
    let mut out = vec![0x03, 126, 1, 1];
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    out.extend_from_slice(&(header_length as u16).to_le_bytes());
    out.extend_from_slice(&(record_length as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 20]);
    for (name, kind, length) in fields {
        let mut slot = [0u8; 32];
        slot[..name.len()].copy_from_slice(name.as_bytes());
        slot[11] = *kind;
        slot[16] = *length as u8;
        out.extend_from_slice(&slot);
    }
    out.push(0x0D);
    for values in records {
        out.push(0x20);
        for ((_, kind, length), value) in fields.iter().zip(values.iter()) {
            let bytes = value.as_bytes();
            let take = bytes.len().min(*length);
            let at = if matches!(kind, b'N' | b'F') && take < *length {
                *length - take
            } else {
                0
            };
            let mut cell = vec![b' '; *length];
            cell[at..at + take].copy_from_slice(&bytes[..take]);
            out.extend_from_slice(&cell);
        }
    }
    out.push(0x1A);
    out
}

#[test]
fn a_dbf_file_imports_its_fixed_width_records() {
    let mut f = Fixture::new("dbf");
    let bytes = dbf(
        &[("NAME", b'C', 10), ("AMOUNT", b'N', 8), ("BORN", b'D', 8)],
        &[&["Ada", "42", "18151210"], &["Grace", "-7", "19060809"]],
    );
    let path = f.dir.join("people.dbf");
    std::fs::write(&path, bytes).expect("write dbf");
    let out = f.run(
        "tools.importDbf",
        json!({"conn": f.conn.clone(), "path": path.to_string_lossy(), "table": "people"}),
    );
    assert_eq!(out["format"], json!("dbf"), "{out}");
    assert_eq!(out["inserted"], json!(2), "{out}");
    let rows =
        f.sql("SELECT NAME AS name, AMOUNT AS amount, BORN AS born FROM people ORDER BY NAME");
    assert_eq!(rows["objects"][0]["name"], json!("Ada"));
    assert_eq!(rows["objects"][0]["amount"], json!("42"));
    assert_eq!(rows["objects"][0]["born"], json!("1815-12-10"));
    assert_eq!(rows["objects"][1]["amount"], json!("-7"));
}

#[test]
fn a_deleted_dbf_record_is_skipped_and_reported() {
    let mut f = Fixture::new("dbf-deleted");
    let mut bytes = dbf(&[("NAME", b'C', 5)], &[&["keep"], &["gone"], &["also"]]);
    // The second record's deletion flag: header is 32 + 32 + 1 bytes, each record is 6.
    bytes[65 + 6] = 0x2A;
    let path = f.dir.join("gone.dbf");
    std::fs::write(&path, bytes).expect("write dbf");
    let out = f.run(
        "tools.importDbf",
        json!({"conn": f.conn.clone(), "path": path.to_string_lossy(), "table": "people"}),
    );
    assert_eq!(out["inserted"], json!(2), "{out}");
    assert_eq!(out["deleted"], json!(1), "{out}");
    assert_eq!(f.count("people"), 2);
}

#[test]
fn a_visual_foxpro_table_is_refused_before_the_table_is_touched() {
    let mut f = Fixture::new("dbf-fox");
    let mut bytes = dbf(&[("NAME", b'C', 5)], &[&["x"]]);
    bytes[0] = 0x30;
    let path = f.dir.join("fox.dbf");
    std::fs::write(&path, bytes).expect("write dbf");
    let err = f
        .try_run(
            "tools.importDbf",
            json!({"conn": f.conn.clone(), "path": path.to_string_lossy(), "table": "people"}),
        )
        .expect_err("foxpro is refused");
    assert!(err.message().contains("Visual FoxPro"), "{err}");
    // No table was created: a refused file must not leave a half-built table behind.
    let objects = f.run("schema.tables", json!({"conn": f.conn.clone()}));
    let names = objects["objects"].to_string();
    assert!(!names.contains("people"), "no table was created: {names}");
}

#[test]
fn the_import_wizard_previews_a_dbf_without_writing() {
    let mut f = Fixture::new("dbf-wizard");
    let bytes = dbf(
        &[("NAME", b'C', 6), ("AMOUNT", b'N', 5)],
        &[&["Ada", "42"], &["Grace", "7"]],
    );
    let path = f.dir.join("preview.dbf");
    std::fs::write(&path, bytes).expect("write dbf");
    let out = f.run(
        "tools.importWizard",
        json!({"conn": f.conn.clone(), "path": path.to_string_lossy(), "table": "people"}),
    );
    assert_eq!(out["columns"][0]["source"], json!("NAME"), "{out}");
    assert_eq!(out["preview"][1][0], json!("Grace"), "{out}");
    // Nothing was written, so the table must not exist at all.
    let objects = f.run("schema.tables", json!({"conn": f.conn.clone()}));
    assert!(
        !objects["objects"].to_string().contains("people"),
        "the wizard writes nothing: {objects}"
    );
}
