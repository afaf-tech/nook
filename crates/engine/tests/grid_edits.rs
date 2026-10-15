//! Grid edits against a table that changed under the tab: the binding is taken when a data tab
//! opens, so an edit planned from it after a schema edit used to name columns that were gone.
//!
//! SQLite reads a double-quoted name that matches no column as a string literal, so the statement
//! matched no row and the grid still reported the change applied. Every case here asserts the row
//! itself, not the answer.

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
        let dir = std::env::temp_dir().join("nook-grid-edits");
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

    /// The same, for the cases that assert on the error a command answers.
    fn try_run(
        &mut self,
        id: &str,
        mut params: serde_json::Value,
    ) -> nook_core::Result<serde_json::Value> {
        params["conn"] = json!(self.conn);
        self.session.run(id, params)
    }

    /// `SELECT email FROM users` in insertion order — the data, not the grid's account of it.
    fn emails(&mut self) -> Vec<String> {
        let value = self.run(
            "sql.run",
            json!({"sql": "SELECT email FROM users ORDER BY rowid"}),
        );
        value["data"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|row| row[0].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// The indices of the open data tabs, in tab order.
    fn data_tabs(&mut self) -> Vec<u64> {
        self.run("tab.list", json!({}))["tabs"]
            .as_array()
            .expect("tabs")
            .iter()
            .filter(|tab| tab["kind"] == "data")
            .filter_map(|tab| tab["index"].as_u64())
            .collect()
    }

    fn open_users(&mut self) {
        self.run(
            "schema.createTable",
            json!({"table": "users", "columns": [
                {"name": "id", "type": "INTEGER", "primary_key": true},
                {"name": "email", "type": "TEXT"},
            ]}),
        );
        self.run(
            "sql.run",
            json!({"sql": "INSERT INTO users (id, email) VALUES (1, 'a@b.c'), (2, 'd@e.f')"}),
        );
        self.run("grid.open", json!({"table": "users"}));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn an_edit_lands_after_the_primary_key_was_renamed_under_the_tab() {
    let mut f = Fixture::new("rename-key");
    f.open_users();

    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "id", "new_name": "uid"}),
    );

    let planned = f.run(
        "grid.editCell",
        json!({"row": 0, "column": "email", "value": "z@z.z"}),
    );
    assert!(
        planned["sql"]
            .as_str()
            .unwrap_or_default()
            .contains("\"uid\" = 1"),
        "the edit has to be keyed on the column the table has now: {}",
        planned["sql"]
    );

    let applied = f.run("grid.apply", json!({}));
    assert_eq!(applied["applied"], 1);
    assert_eq!(
        f.emails(),
        vec!["z@z.z".to_string(), "d@e.f".to_string()],
        "the row the grid said it changed has to have changed"
    );
}

#[test]
fn an_edit_planned_before_a_schema_change_still_lands() {
    let mut f = Fixture::new("pending");
    f.open_users();

    // Planned against the table as it was…
    f.run(
        "grid.editCell",
        json!({"row": 1, "column": "email", "value": "after@edit"}),
    );
    // …and the table gains a column before the apply: the pending statement is planned again, so it
    // is not thrown away and it cannot be run against a column that moved.
    f.run(
        "schema.addColumn",
        json!({"table": "users", "name": "note", "type": "TEXT"}),
    );
    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "id", "new_name": "uid"}),
    );

    let applied = f.run("grid.apply", json!({}));
    assert_eq!(applied["applied"], 1);
    assert_eq!(
        f.emails(),
        vec!["a@b.c".to_string(), "after@edit".to_string()],
        "the pending edit has to land on the row it was made against"
    );
}

#[test]
fn a_rename_run_in_the_editor_is_seen_by_the_next_edit() {
    let mut f = Fixture::new("editor-ddl");
    f.open_users();

    // The user's own door: a statement typed in the editor, not a designer command.
    f.run(
        "sql.run",
        json!({"sql": "ALTER TABLE users RENAME COLUMN id TO uid"}),
    );

    let planned = f.run(
        "grid.editCell",
        json!({"row": 0, "column": "email", "value": "e@d.it"}),
    );
    assert!(
        planned["sql"]
            .as_str()
            .unwrap_or_default()
            .contains("\"uid\" = 1"),
        "the edit has to be keyed on the column the table has now: {}",
        planned["sql"]
    );
    f.run("grid.apply", json!({}));
    assert_eq!(f.emails(), vec!["e@d.it".to_string(), "d@e.f".to_string()]);
}

#[test]
fn an_edit_of_a_row_that_is_gone_is_refused_not_reported() {
    let mut f = Fixture::new("row-gone");
    f.open_users();
    f.run(
        "grid.editCell",
        json!({"row": 1, "column": "email", "value": "x@y.z"}),
    );

    // Another client deletes the row the edit was made against.
    f.run("sql.run", json!({"sql": "DELETE FROM users WHERE id = 2"}));

    let err = f
        .try_run("grid.apply", json!({}))
        .expect_err("an edit of a row that is gone has to be refused");
    assert!(
        err.to_string().contains("changed no row"),
        "the refusal has to say what happened: {err}"
    );
    assert_eq!(
        f.emails(),
        vec!["a@b.c".to_string()],
        "and it must not have changed anything else"
    );
}

#[test]
fn an_edit_that_writes_the_value_a_row_already_holds_still_applies() {
    // The guard refuses an instruction that changed no row. A write of the value the row already has
    // is not that: the row is there and the statement ran against it, so a driver that counts matched
    // rows reports one — and the guard must not turn a legitimate edit into an error.
    let mut f = Fixture::new("same-value");
    f.open_users();
    f.run(
        "grid.editCell",
        json!({"row": 0, "column": "email", "value": "a@b.c"}),
    );
    assert_eq!(f.run("grid.apply", json!({}))["applied"], 1);
    assert_eq!(f.emails(), vec!["a@b.c".to_string(), "d@e.f".to_string()]);
}

#[test]
fn the_form_lists_the_columns_the_table_has_now() {
    let mut f = Fixture::new("form");
    f.open_users();
    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "id", "new_name": "uid"}),
    );

    let form = f.run("grid.formView", json!({"row": 0}));
    let names: Vec<&str> = form["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| field["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        vec!["uid", "email"],
        "the form offers the fields the table has"
    );
    assert_eq!(
        form["fields"][0]["primary_key"], true,
        "and the key it reports is the key the edit will be planned with"
    );
    assert_eq!(form["fields"][0]["value"], 1);
}

#[test]
fn a_delete_lands_after_the_key_column_was_renamed() {
    let mut f = Fixture::new("delete");
    f.open_users();
    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "id", "new_name": "uid"}),
    );
    f.run("grid.deleteRow", json!({"row": 1}));
    let applied = f.run("grid.apply", json!({}));
    assert_eq!(applied["applied"], 1);
    assert_eq!(f.emails(), vec!["a@b.c".to_string()]);
}

#[test]
fn a_pending_edit_stays_with_its_own_tab() {
    let mut f = Fixture::new("two-tabs");
    f.open_users(); // tab 0
    f.run("grid.open", json!({"table": "users"})); // tab 1, active, same table
    f.run(
        "grid.editCell",
        json!({"row": 0, "column": "email", "value": "tab1@edit"}),
    );
    f.run(
        "schema.renameColumn",
        json!({"table": "users", "column": "id", "new_name": "uid"}),
    );

    // Tab 0 was not the tab edited, so there is nothing of its own to apply.
    let data_tabs = f.data_tabs();
    assert_eq!(data_tabs.len(), 2, "two tables open: {data_tabs:?}");
    f.run("tab.select", json!({"index": data_tabs[0]}));
    assert_eq!(f.run("grid.apply", json!({}))["applied"], 0);
    assert_eq!(f.emails(), vec!["a@b.c".to_string(), "d@e.f".to_string()]);

    // Tab 1's edit is still its own, and still lands.
    f.run("tab.select", json!({"index": data_tabs[1]}));
    assert_eq!(f.run("grid.apply", json!({}))["applied"], 1);
    assert_eq!(
        f.emails(),
        vec!["tab1@edit".to_string(), "d@e.f".to_string()]
    );
}

#[test]
fn an_insert_lands_after_a_column_was_added() {
    let mut f = Fixture::new("insert");
    f.open_users();
    f.run(
        "schema.addColumn",
        json!({"table": "users", "name": "note", "type": "TEXT"}),
    );
    f.run(
        "grid.insertRow",
        json!({"values": {"id": 3, "email": "c@d.e", "note": "fresh"}}),
    );
    let applied = f.run("grid.apply", json!({}));
    assert_eq!(applied["applied"], 1);
    assert_eq!(
        f.emails(),
        vec![
            "a@b.c".to_string(),
            "d@e.f".to_string(),
            "c@d.e".to_string()
        ]
    );
}

/// A pending edit belongs to the record it was made on. The page under the tab can be sorted,
/// filtered or paged between the edit and the apply, and a statement re-planned from the row *index*
/// then points at whichever record moved into that row — changing it while reporting the edit
/// applied, which is the failure the row-count guard cannot see because the wrong row does change.
#[test]
fn an_edit_applied_after_a_sort_lands_on_the_record_it_was_made_on() {
    let mut f = Fixture::new("sort-then-apply");
    f.open_users();

    f.run(
        "grid.editCell",
        json!({"row": 0, "column": "email", "value": "z@z.z"}),
    );
    // `a@b.c` sorts below `d@e.f`, so row 0 of the page is now the record with id 2.
    f.run("grid.sort", json!({"column": "email", "desc": true}));

    let applied = f.run("grid.apply", json!({}));
    assert_eq!(applied["applied"], 1);
    assert_eq!(
        f.emails(),
        vec!["z@z.z".to_string(), "d@e.f".to_string()],
        "the edit belongs to the row it was made on, wherever the page put it"
    );
}
