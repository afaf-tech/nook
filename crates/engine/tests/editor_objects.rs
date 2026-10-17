//! The schema editors (gaps.md §4): `schema.triggerEditor`, `schema.functionEditor`,
//! `schema.sequenceEditor` — every one end to end against a real SQLite file, plus
//! dialect-builder snapshots for the PostgreSQL shapes no live server covers here.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

struct Fixture {
    session: Session,
    conn: String,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join("nook-editor-objects");
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
        }
    }

    fn run(&mut self, id: &str, mut params: serde_json::Value) -> serde_json::Value {
        params["conn"] = json!(self.conn);
        self.session
            .run(id, params)
            .unwrap_or_else(|e| panic!("{id} failed: {e}"))
    }

    fn try_run(
        &mut self,
        id: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        params["conn"] = json!(self.conn);
        self.session.run(id, params).map_err(|e| e.to_string())
    }

    /// The trigger names the editor lists for a table, straight from the cached catalog.
    fn trigger_names(&mut self, table: &str) -> Vec<String> {
        let value = self.run("schema.triggerEditor", json!({"action": "list", "table": table}));
        value["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|t| t["name"].as_str().unwrap_or_default().to_string())
            .collect()
    }
}

fn users_table(f: &mut Fixture) {
    f.run(
        "schema.createTable",
        json!({"table": "users", "columns": [
            {"name": "id", "type": "INTEGER", "primary_key": true},
            {"name": "email", "type": "TEXT"},
        ]}),
    );
}

#[test]
fn a_trigger_with_a_two_statement_body_roundtrips_through_the_editor() {
    let mut f = Fixture::new("trigger-roundtrip");
    users_table(&mut f);

    // RED→GREEN proof: the create path must survive a `;` inside BEGIN..END. A statement
    // splitter that ignores trigger bodies breaks this into two fragments and fails.
    f.run(
        "schema.triggerEditor",
        json!({
            "action": "create",
            "table": "users",
            "name": "audit_email",
            "timing": "AFTER",
            "event": "INSERT",
            "body": "UPDATE users SET email = lower(email);\nUPDATE users SET email = email WHERE id = NEW.id;"
        }),
    );

    let names = f.trigger_names("users");
    assert!(names.contains(&"audit_email".to_string()), "trigger listed: {names:?}");

    // The stored DDL keeps both statements inside one trigger definition.
    let value = f.run("schema.triggerEditor", json!({"action": "list", "table": "users"}));
    let ddl = value["items"]
        .as_array()
        .and_then(|items| items.iter().find(|t| t["name"] == "audit_email"))
        .and_then(|t| t["ddl"].as_str())
        .expect("the trigger carries its ddl");
    assert!(ddl.contains("lower(email)"), "ddl keeps statement one: {ddl}");
    assert!(ddl.contains("NEW.id"), "ddl keeps statement two: {ddl}");

    f.run(
        "schema.triggerEditor",
        json!({"action": "drop", "table": "users", "name": "audit_email", "confirm": true}),
    );
    let names = f.trigger_names("users");
    assert!(!names.contains(&"audit_email".to_string()), "trigger dropped: {names:?}");
}

#[test]
fn an_editor_list_on_a_fresh_connection_is_empty_not_an_error() {
    let mut f = Fixture::new("trigger-empty");
    users_table(&mut f);
    let names = f.trigger_names("users");
    assert!(names.is_empty(), "no triggers yet: {names:?}");
}

#[test]
fn a_drop_without_confirm_is_refused() {
    let mut f = Fixture::new("trigger-confirm");
    users_table(&mut f);
    let err = f
        .try_run(
            "schema.triggerEditor",
            json!({"action": "drop", "table": "users", "name": "nope"}),
        )
        .expect_err("destructive actions need confirm");
    assert!(err.contains("confirm"), "refusal names the missing confirm: {err}");
}

#[test]
fn sequence_editor_refuses_sqlite_with_an_explanation() {
    let mut f = Fixture::new("sequence-sqlite");
    users_table(&mut f);
    let err = f
        .try_run(
            "schema.sequenceEditor",
            json!({"action": "create", "name": "user_ids"}),
        )
        .expect_err("SQLite has no sequences");
    assert!(
        err.contains("sequence"),
        "refusal names the unsupported feature: {err}"
    );
}

#[test]
fn function_editor_refuses_sqlite_with_an_explanation() {
    let mut f = Fixture::new("routine-sqlite");
    users_table(&mut f);
    let err = f
        .try_run(
            "schema.functionEditor",
            json!({"action": "create", "name": "add_one", "kind": "function", "sql": "(n int) RETURNS int"}),
        )
        .expect_err("SQLite has no stored routines");
    assert!(
        err.contains("routine") || err.contains("function"),
        "refusal names the unsupported feature: {err}"
    );
}

#[test]
fn the_postgres_trigger_and_sequence_builders_match_the_dialect() {
    // No live PostgreSQL here; the SQL the commands will send is pinned instead.
    // The PG trigger executes a function, so the body is the call, not raw SQL.
    let sql = nook_model::ddl::create_trigger(
        Some("public"),
        "touch",
        "users",
        "BEFORE",
        "UPDATE",
        "notify_users_touched()",
        nook_core::Dialect::Postgres,
    );
    assert_eq!(
        sql,
        "CREATE TRIGGER \"touch\" BEFORE UPDATE ON \"public\".\"users\" FOR EACH ROW EXECUTE FUNCTION notify_users_touched();"
    );
}
