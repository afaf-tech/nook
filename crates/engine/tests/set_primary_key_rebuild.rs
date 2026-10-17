//! `schema.setPrimaryKey` on SQLite rebuilds the table. These cases drive the command through the
//! real product path — `execute_ddl` → `run_sql` → `run_script` → `split_script` — where a naive
//! splitter would cut a trigger body at its inner semicolon.

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
        let dir = std::env::temp_dir().join("nook-set-pk");
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
            path,
        }
    }

    fn run(&mut self, id: &str, mut params: serde_json::Value) -> serde_json::Value {
        params["conn"] = json!(self.conn);
        self.session
            .run(id, params)
            .unwrap_or_else(|e| panic!("{id} failed: {e}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn a_rebuild_through_the_command_carries_triggers_and_respects_the_pragma() {
    let mut f = Fixture::new("trigger-rebuild");
    f.open_keyless_table();
    f.run(
        "sql.run",
        json!({"sql": "CREATE TRIGGER users_touch AFTER UPDATE ON users BEGIN UPDATE users SET email = email WHERE id = NEW.id; END;"}),
    );
    f.run("sql.run", json!({"sql": "PRAGMA foreign_keys = OFF"}));

    f.run(
        "schema.setPrimaryKey",
        json!({"table": "users", "columns": ["id"]}),
    );

    let schema = f.run(
        "sql.run",
        json!({"sql": "SELECT sql FROM sqlite_master WHERE type = 'trigger'"}),
    );
    let sql = schema["data"][0][0].as_str().unwrap_or_default();
    assert!(
        sql.contains("CREATE TRIGGER users_touch"),
        "the trigger must survive the rebuild through the product path: {sql}"
    );

    // The session's foreign_keys state is restored, not forced on.
    let state = f.run("sql.run", json!({"sql": "PRAGMA foreign_keys"}));
    assert_eq!(
        state["data"][0][0].as_i64(),
        Some(0),
        "the pragma is restored to OFF, not flipped on by the rebuild"
    );
}

impl Fixture {
    fn open_keyless_table(&mut self) {
        self.run(
            "schema.createTable",
            json!({"table": "users", "columns": [
                {"name": "id", "type": "INTEGER"},
                {"name": "email", "type": "TEXT"},
            ]}),
        );
    }
}
