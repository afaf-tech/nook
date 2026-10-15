//! The design tab's edits, end to end: every command the designer buttons and the command dialog
//! send, against a real SQLite file. A regression here is what the user sees as "edit table errors".

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
        let dir = std::env::temp_dir().join("nook-design-table");
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

    fn try_run(
        &mut self,
        id: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        params["conn"] = json!(self.conn);
        self.session.run(id, params).map_err(|e| e.to_string())
    }

    fn columns(&mut self, table: &str) -> Vec<String> {
        let value = self.run("schema.columns", json!({"table": table}));
        value["columns"]
            .as_array()
            .expect("columns")
            .iter()
            .map(|c| c["name"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// A `CREATE TABLE` columns list comes back through `schema.columns`, and that is the read the
    /// design tab draws — the designer's edits have to show up here.
    fn create_users(&mut self) {
        self.run(
            "schema.createTable",
            json!({"table": "users", "columns": [
                {"name": "id", "type": "INTEGER", "primary_key": true},
                {"name": "email", "type": "TEXT"},
            ]}),
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn every_design_table_edit_that_sqlite_has_runs() {
    let mut f = Fixture::new("edits");
    f.create_users();

    f.run(
        "schema.addColumn",
        json!({"table": "users", "name": "nickname", "type": "TEXT"}),
    );
    assert!(f.columns("users").contains(&"nickname".to_string()));

    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "nickname", "new_name": "handle"}),
    );
    assert!(f.columns("users").contains(&"handle".to_string()));

    f.run(
        "schema.renameTable",
        json!({"table": "users", "new_name": "people"}),
    );
    f.run(
        "schema.dropColumn",
        json!({"table": "people", "column": "handle", "confirm": true}),
    );
    assert!(!f.columns("people").contains(&"handle".to_string()));

    f.run(
        "schema.createIndex",
        json!({"table": "people", "name": "ix_people_email", "columns": ["email"]}),
    );
    f.run(
        "schema.dropIndex",
        json!({"table": "people", "index": "ix_people_email", "confirm": true}),
    );

    f.run(
        "schema.copyTable",
        json!({"table": "people", "new_name": "people_copy"}),
    );
    f.run(
        "schema.dropTable",
        json!({"table": "people_copy", "confirm": true}),
    );
    f.run(
        "schema.truncateTable",
        json!({"table": "people", "confirm": true}),
    );

    // The reads the design tab and the inspector run on the same table.
    f.run("schema.indexes", json!({"table": "people"}));
    f.run("schema.foreignKeys", json!({"table": "people"}));
    f.run("schema.ddl", json!({"table": "people"}));
}

#[test]
fn a_change_sqlite_cannot_make_in_place_names_itself_as_the_reason() {
    let mut f = Fixture::new("gates");
    f.create_users();

    // SQLite has no ALTER COLUMN. The designer must hear which change is impossible and why, rather
    // than a syntax error the server refused — that is what makes the tab look broken.
    for (id, params, expected) in [
        (
            "schema.changeColumnType",
            json!({"table": "users", "column": "email", "type": "VARCHAR(64)"}),
            "cannot change a column type in place",
        ),
        (
            "schema.setNullable",
            json!({"table": "users", "column": "email", "nullable": false}),
            "cannot change NOT NULL in place",
        ),
        (
            "schema.setDefault",
            json!({"table": "users", "column": "email", "default": "'x'"}),
            "cannot change a default in place",
        ),
        (
            "schema.addForeignKey",
            json!({"table": "users", "columns": ["id"], "ref_table": "users", "ref_columns": ["id"]}),
            "cannot add a foreign key to an existing table",
        ),
    ] {
        let error = f.try_run(id, params).expect_err("sqlite refuses this");
        assert!(
            error.contains(expected),
            "{id} should say '{expected}', said: {error}"
        );
        assert!(
            error.contains("SQLite"),
            "{id} should name the dialect: {error}"
        );
    }
}

#[test]
fn an_empty_table_parameter_names_the_parameter_not_the_server() {
    let mut f = Fixture::new("empty");
    f.create_users();
    // The designer buttons used to open the dialog with `table` prefilled to nothing, so a run
    // failed with "no such table: " — a message that blames the server for a missing argument.
    for spec in ["", "   "] {
        let error = f
            .try_run(
                "schema.addColumn",
                json!({"table": spec, "name": "note", "type": "TEXT"}),
            )
            .expect_err("an empty table is not a table");
        assert!(
            error.contains("pass \"table\""),
            "the message should ask for the parameter, said: {error}"
        );
    }
}

#[test]
fn a_connection_whose_id_holds_a_delimiter_still_edits() {
    // A SQLite connection's id is the database path, so it holds whatever the path holds. The
    // designer commands reach it as a parameter, and this is the value that a `&`/`=`-delimited
    // hand-off would have cut in half (`no such connection named '/Users/ann/R'`).
    let mut f = Fixture::new("R&D=1");
    assert!(
        f.conn.contains('&') && f.conn.contains('='),
        "the fixture has to exercise both delimiters, id was {}",
        f.conn
    );
    f.create_users();
    f.run(
        "schema.addColumn",
        json!({"table": "users", "name": "note", "type": "TEXT"}),
    );
    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "note", "new_name": "memo"}),
    );
    assert!(f.columns("users").contains(&"memo".to_string()));
}
