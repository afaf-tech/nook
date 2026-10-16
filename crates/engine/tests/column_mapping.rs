//! Column mapping for imports and transfers: a `columnMapping` of `src=target` pairs must land
//! rows in the named table columns, not by position. Every case asserts the stored row itself.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
    path: std::path::PathBuf,
    dir: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-column-mapping");
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
            dir,
            path,
        }
    }

    fn create(&mut self, ddl: &str) {
        self.session
            .run("sql.run", json!({"sql": ddl, "conn": self.conn}))
            .expect("create table");
    }

    fn rows(&mut self, sql: &str) -> Vec<Vec<serde_json::Value>> {
        let out = self
            .session
            .run("sql.run", json!({"sql": sql, "conn": self.conn}))
            .expect("query");
        out["data"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| r.as_array().cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn rows_conn(&mut self, conn: &str, sql: &str) -> Vec<Vec<serde_json::Value>> {
        let out = self
            .session
            .run("sql.run", json!({"sql": sql, "conn": conn}))
            .expect("query");
        out["data"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| r.as_array().cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn csv(dir: &std::path::Path, tag: &str, text: &str) -> String {
    let path = dir.join(format!("{tag}.csv"));
    std::fs::write(&path, text).expect("write csv");
    path.to_string_lossy().into_owned()
}

#[test]
fn a_full_mapping_lands_rows_in_the_named_columns() {
    let mut f = Fixture::new("mapping-full");
    f.create("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, email TEXT)");
    let path = csv(&f.dir, "full", "email,who\nnew@e.f,Ann\nold@e.f,Bob\n");
    let out = f
        .session
        .run(
            "tools.importCsv",
            json!({
                "path": path,
                "table": "people",
                "header": true,
                "create_table": false,
                "columnMapping": ["email=email", "who=name"],
                "conn": f.conn,
            }),
        )
        .expect("mapped import");
    assert_eq!(out["inserted"], json!(2), "{out}");

    let rows = f.rows("SELECT id, name, email FROM people ORDER BY id");
    assert_eq!(rows[0][1], json!("Ann"), "{rows:?}");
    assert_eq!(rows[0][2], json!("new@e.f"), "{rows:?}");
    assert_eq!(rows[1][1], json!("Bob"), "{rows:?}");
    assert_eq!(rows[1][2], json!("old@e.f"), "{rows:?}");
}

#[test]
fn a_partial_mapping_imports_only_the_mapped_columns() {
    let mut f = Fixture::new("mapping-partial");
    f.create("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, email TEXT)");
    let path = csv(&f.dir, "partial", "who\nAnn\n");
    let out = f
        .session
        .run(
            "tools.importCsv",
            json!({
                "path": path,
                "table": "people",
                "header": true,
                "create_table": false,
                "columnMapping": ["who=name"],
                "conn": f.conn,
            }),
        )
        .expect("partial import");
    assert_eq!(out["inserted"], json!(1), "{out}");

    let rows = f.rows("SELECT name, email FROM people");
    assert_eq!(rows[0][0], json!("Ann"), "{rows:?}");
    assert_eq!(rows[0][1], serde_json::Value::Null, "{rows:?}");
}

#[test]
fn an_unknown_mapping_key_is_an_error_and_inserts_nothing() {
    let mut f = Fixture::new("mapping-unknown");
    f.create("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)");
    // The positional path would succeed on this header, so only the mapping validator's
    // rejection of the unknown table column makes this an error — not a name collision.
    let path = csv(&f.dir, "unknown", "name\nAnn\n");
    let result = f.session.run(
        "tools.importCsv",
        json!({
            "path": path,
            "table": "people",
            "header": true,
            "create_table": false,
            "columnMapping": ["name=nope"],
            "conn": f.conn,
        }),
    );
    assert!(result.is_err(), "unknown target must be rejected: {result:?}");
    let rows = f.rows("SELECT COUNT(*) FROM people");
    assert_eq!(rows[0][0], json!(0), "{rows:?}");
}

#[test]
fn no_mapping_keeps_the_positional_path() {
    let mut f = Fixture::new("mapping-absent");
    f.create("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)");
    let path = csv(&f.dir, "absent", "name\nAnn\n");
    let out = f
        .session
        .run(
            "tools.importCsv",
            json!({
                "path": path,
                "table": "people",
                "header": true,
                "conn": f.conn,
            }),
        )
        .expect("positional import");
    assert_eq!(out["inserted"], json!(1), "{out}");
    let rows = f.rows("SELECT name FROM people");
    assert_eq!(rows[0][0], json!("Ann"), "{rows:?}");
}
#[test]
fn a_mapped_transfer_copies_across_differently_ordered_columns() {
    let mut f = Fixture::new("mapping-transfer-src");
    // Register a second, independent sqlite database in the same session: `transfer.data`
    // requires two distinct connections, and the destination table carries the same name with a
    // different column layout, so only the mapping can place the values correctly.
    let dst_path = f.dir.join(format!("mapping-transfer-dst-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&dst_path);
    let dst_cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", dst_path.display()))
        .expect("a sqlite URI parses");
    let dst_conn = dst_cfg.id.clone();
    f.session.conns.push(Conn::new(dst_cfg));
    let dst_index = f
        .session
        .conn_index(&dst_conn)
        .expect("the second connection is registered");
    open_now(&mut f.session, dst_index).expect("the second sqlite opens");

    f.create("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, email TEXT)");
    f.session
        .run(
            "sql.run",
            json!({"sql": "CREATE TABLE people (id INTEGER PRIMARY KEY, mail TEXT, who TEXT)", "conn": dst_conn}),
        )
        .expect("create dst");
    f.session
        .run(
            "sql.run",
            json!({"sql": "INSERT INTO people (id, name, email) VALUES (1, 'Ann', 'a@e.f')", "conn": f.conn}),
        )
        .expect("seed");
    let out = f
        .session
        .run(
            "transfer.data",
            json!({
                "from": f.conn,
                "to": dst_conn,
                "tables": ["people"],
                "columnMapping": ["name=who", "email=mail"],
            }),
        )
        .expect("mapped transfer");
    assert_eq!(out["failed"], json!(0), "{out}");
    let rows = f.rows_conn(&dst_conn, "SELECT who, mail FROM people");
    assert_eq!(rows[0][0], json!("Ann"), "{rows:?}");
    assert_eq!(rows[0][1], json!("a@e.f"), "{rows:?}");
}
