//! `schema.tableRebuild`: the diff contract, the row-count guard, and the refusals.

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
        let dir = std::env::temp_dir().join("nook-table-rebuild");
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
        let mut f = Fixture {
            session,
            conn,
            path,
        };
        f.run(
            "sql.run",
            json!({"sql": "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, note TEXT, total INTEGER); \
                           CREATE INDEX users_email ON users (email); \
                           INSERT INTO users VALUES (1,'a@b.c','hi',7), (2,'d@e.f','yo',9);"}),
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

    fn scalar(&mut self, sql: &str) -> String {
        let out = self.run("sql.run", json!({ "sql": sql }));
        out["data"][0][0]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| out["data"][0][0].to_string())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn a_rename_keeps_the_rows_the_index_and_the_columns_nobody_named() {
    let mut f = Fixture::new("rename");
    let out = f.run(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"rename": {"email": "mail"}}}),
    );
    assert_eq!(out["rows_before"], 2);
    assert_eq!(out["rows_after"], 2);
    assert_eq!(out["lost"], json!([]));
    // `note` and `total` were never mentioned, and they survive: that is the whole point of taking a
    // diff instead of a column list.
    assert_eq!(out["after"], json!(["id", "mail", "note", "total"]));
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users"), "2");
    assert_eq!(f.scalar("SELECT mail FROM users WHERE id = 1"), "a@b.c");
    // The index came with it, under the same name.
    assert_eq!(
        f.scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'users_email'"
        ),
        "1"
    );
}

#[test]
fn a_retype_round_trips_the_values() {
    let mut f = Fixture::new("retype");
    let out = f.run(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"retype": {"total": "TEXT"}}}),
    );
    assert_eq!(out["rows_after"], 2);
    assert_eq!(f.scalar("SELECT total FROM users WHERE id = 1"), "7");
    // The declared type is what changed, and the catalog agrees.
    let columns = f.run("schema.columns", json!({"table": "users"}));
    let total = columns["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .find(|c| c["name"] == "total")
        .expect("total");
    assert_eq!(total["type"], "TEXT");
}

#[test]
fn a_drop_of_a_missing_column_refuses_instead_of_rebuilding_anything() {
    let mut f = Fixture::new("drop-missing");
    let message = f.refuse(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"drop": ["nope"]}}),
    );
    assert!(message.contains("no column 'nope'"), "{message}");
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users"), "2");
}

#[test]
fn an_unknown_change_key_refuses_so_a_typo_cannot_do_nothing() {
    let mut f = Fixture::new("typo");
    let message = f.refuse(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"renmae": {"email": "mail"}}}),
    );
    assert!(message.contains("unknown change 'renmae'"), "{message}");
    assert!(message.contains("rename"), "{message}");
}

#[test]
fn a_rebuild_inside_an_open_transaction_refuses() {
    let mut f = Fixture::new("in-tx");
    f.run("sql.run", json!({"sql": "BEGIN"}));
    let message = f.refuse(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"rename": {"email": "mail"}}}),
    );
    assert!(
        message.contains("commit or roll back the open transaction"),
        "{message}"
    );
    f.run("sql.run", json!({"sql": "ROLLBACK"}));
}

#[test]
fn changes_that_would_empty_the_table_refuse() {
    let mut f = Fixture::new("empty");
    let message = f.refuse(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"drop": ["id", "email", "note", "total"]}}),
    );
    assert!(message.contains("no columns"), "{message}");
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users"), "2");
}

#[test]
fn a_column_added_by_the_diff_arrives_with_its_default_null() {
    let mut f = Fixture::new("add");
    let out = f.run(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"add": [{"name": "tag", "type": "TEXT"}]}}),
    );
    assert_eq!(out["after"], json!(["id", "email", "note", "total", "tag"]));
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM users WHERE tag IS NULL"),
        "2"
    );
}

#[test]
fn a_rebuild_that_would_orphan_a_reference_is_rolled_back() {
    let mut f = Fixture::new("orphan");
    // `posts.author` names `users(id)`. Renaming that column through a rebuild leaves the child
    // pointing at a column that no longer exists — the one loss a rename can cause, and the reason
    // the check runs inside the transaction.
    f.run(
        "sql.run",
        json!({"sql": "CREATE TABLE posts (id INTEGER PRIMARY KEY, author INTEGER REFERENCES users(id)); \
                       INSERT INTO posts VALUES (1, 1);"}),
    );
    let message = f.refuse(
        "schema.tableRebuild",
        json!({"table": "users", "changes": {"rename": {"id": "uid"}}}),
    );
    assert!(message.contains("nothing was changed"), "{message}");
    // The rollback is the point: the old table is still there, with its row and its column name.
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users"), "2");
    let columns = f.run("schema.columns", json!({"table": "users"}));
    let names: Vec<&str> = columns["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .map(|c| c["name"].as_str().unwrap_or_default())
        .collect();
    assert!(names.contains(&"id"), "{names:?}");
    assert!(!names.contains(&"uid"), "{names:?}");
}
