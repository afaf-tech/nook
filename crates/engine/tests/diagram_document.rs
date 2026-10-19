//! The diagram as a document: reverse engineering, positions, the model file, forward engineering,
//! and the data dictionary.

use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

/// One config directory for the whole test binary, created once.
fn config_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("nook-diagram-config-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

struct Fixture {
    session: Session,
    conn: String,
    dir: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("nook-diagram-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        // `NOOK_CONFIG_DIR` is process-global, so every test in this binary shares one store: giving
        // each its own would have them overwrite each other's variable and delete the store out
        // from under a sibling. The database files below stay per-test.
        std::env::set_var("NOOK_CONFIG_DIR", config_dir());
        let path = dir.join("app.db");
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
            dir,
        };
        f.run(
            "sql.run",
            json!({"sql": "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL); \
                           CREATE TABLE posts (id INTEGER PRIMARY KEY, \
                                               author INTEGER REFERENCES users(id), \
                                               title TEXT DEFAULT 'untitled'); \
                           CREATE INDEX ix_posts_title ON posts(title);"}),
        );
        f
    }

    fn run(&mut self, id: &str, mut params: serde_json::Value) -> serde_json::Value {
        params["conn"] = json!(self.conn);
        self.session
            .run(id, params)
            .unwrap_or_else(|e| panic!("{id} failed: {e}"))
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn reverse_engineering_returns_the_catalog_as_a_document_with_positions() {
    let mut f = Fixture::new("reverse");
    let out = f.run("diagram.reverseEngineer", json!({}));
    assert_eq!(out["version"], 1);
    let boxes = out["boxes"].as_array().expect("boxes");
    assert_eq!(boxes.len(), 2, "one box per table");
    assert!(boxes.iter().all(|b| b["x"].is_number() && b["y"].is_number()));
    // The model travels with the tables, which is what makes it a document.
    let model = &out["model"];
    let tables = model["tables"].as_array().expect("tables");
    assert_eq!(tables.len(), 2);
    let posts = tables
        .iter()
        .find(|t| t["name"] == "posts")
        .expect("posts");
    assert_eq!(posts["columns"].as_array().expect("columns").len(), 3);
    assert_eq!(posts["foreign_keys"].as_array().expect("fks").len(), 1);
}

#[test]
fn a_manual_position_survives_and_a_reset_clears_it() {
    let mut f = Fixture::new("layout");
    let before = f.run("diagram.reverseEngineer", json!({}));
    let key = before["boxes"][0]["key"].as_str().expect("a key").to_string();
    let original = (
        before["boxes"][0]["x"].as_f64().expect("x"),
        before["boxes"][0]["y"].as_f64().expect("y"),
    );

    let moved = f.run(
        "diagram.manualLayout",
        json!({"positions": {key.clone(): [400.0, 250.0]}}),
    );
    assert_eq!(moved["moved"], 1);

    let after = f.run("diagram.reverseEngineer", json!({}));
    let box_after = after["boxes"]
        .as_array()
        .expect("boxes")
        .iter()
        .find(|b| b["key"] == key.as_str())
        .expect("the same box");
    assert_eq!(box_after["x"], 400.0);
    assert_eq!(box_after["y"], 250.0);
    assert_ne!(
        (box_after["x"].as_f64(), box_after["y"].as_f64()),
        (Some(original.0), Some(original.1)),
        "the override must actually differ from the deterministic layout"
    );

    f.run("diagram.manualLayout", json!({"reset": true}));
    let reset = f.run("diagram.reverseEngineer", json!({}));
    let box_reset = reset["boxes"]
        .as_array()
        .expect("boxes")
        .iter()
        .find(|b| b["key"] == key.as_str())
        .expect("the same box");
    assert_eq!(box_reset["x"].as_f64(), Some(original.0));
}

#[test]
fn a_model_file_round_trips_the_positions_it_saved() {
    let mut f = Fixture::new("modelfile");
    let before = f.run("diagram.reverseEngineer", json!({}));
    let users_key = before["boxes"]
        .as_array()
        .expect("boxes")
        .iter()
        .map(|b| b["key"].as_str().unwrap_or_default().to_string())
        .find(|k| k.ends_with("users"))
        .expect("a users box");
    let mut positions = serde_json::Map::new();
    positions.insert(users_key.clone(), json!([512.0, 64.0]));
    f.run("diagram.manualLayout", json!({"positions": positions}));
    let path = f.path("shop.json");
    let saved = f.run(
        "diagram.modelFile",
        json!({"path": path, "action": "save"}),
    );
    assert!(saved["bytes"].as_u64().unwrap_or(0) > 0);
    // The file is readable without a connection: the tables are inside it.
    let text = std::fs::read_to_string(&path).expect("the file exists");
    assert!(text.contains("\"users\""), "{text}");
    assert!(text.contains("posts"), "{text}");

    // Clear the layout, then load the file back and the positions return.
    f.run("diagram.manualLayout", json!({"reset": true}));
    let cleared = f.run("diagram.reverseEngineer", json!({}));
    let cleared_box = cleared["boxes"]
        .as_array()
        .expect("boxes")
        .iter()
        .find(|b| b["key"] == users_key.as_str())
        .expect("users");
    assert_ne!(cleared_box["x"], 512.0);

    let loaded = f.run("diagram.modelFile", json!({"path": path, "action": "load"}));
    assert_eq!(loaded["tables"], 2);
    let restored = f.run("diagram.reverseEngineer", json!({}));
    let restored_box = restored["boxes"]
        .as_array()
        .expect("boxes")
        .iter()
        .find(|b| b["key"] == users_key.as_str())
        .expect("users");
    assert_eq!(restored_box["x"], 512.0);
    assert_eq!(restored_box["y"], 64.0);
}

#[test]
fn forward_engineering_puts_a_referenced_table_first() {
    let mut f = Fixture::new("forward");
    let model = f.run("diagram.reverseEngineer", json!({}))["model"].clone();
    let out = f.run("diagram.forwardEngineer", json!({"model": model}));
    let statements: Vec<String> = out["statements"]
        .as_array()
        .expect("statements")
        .iter()
        .map(|s| s.as_str().unwrap_or_default().to_string())
        .collect();
    // Two CREATEs plus the index; the FK rides inside `posts`' CREATE.
    assert_eq!(statements.len(), 3, "{statements:#?}");
    // `posts` references `users`, so it cannot come first.
    assert!(statements[0].contains("users"), "{}", statements[0]);
    assert!(statements[1].contains("posts"), "{}", statements[1]);
    assert!(
        statements[1].contains("FOREIGN KEY"),
        "the key belongs in the CREATE: {}",
        statements[1]
    );
    // The index rides with its table rather than being forgotten.
    assert!(
        statements.iter().any(|s| s.contains("ix_posts_title")),
        "{statements:#?}"
    );
}

#[test]
fn a_cycle_is_broken_with_a_later_alter_table() {
    // A server with `ALTER TABLE … ADD CONSTRAINT` can defer half a cycle. This asserts the shape of
    // that plan directly: the ordering is a property of the model, not of a particular server.
    let model = json!({
        "version": 1,
        "conn": "scratch",
        "tables": [
            {"name": "a", "kind": "table",
             "columns": [{"name": "id", "type_name": "INTEGER"}, {"name": "b_id", "type_name": "INTEGER"}],
             "foreign_keys": [{"name": "fk_a_b", "columns": ["b_id"], "ref_table": "b", "ref_columns": ["id"]}]},
            {"name": "b", "kind": "table",
             "columns": [{"name": "id", "type_name": "INTEGER"}, {"name": "a_id", "type_name": "INTEGER"}],
             "foreign_keys": [{"name": "fk_b_a", "columns": ["a_id"], "ref_table": "a", "ref_columns": ["id"]}]}
        ]
    });
    let doc: nook_model::DiagramDocument = serde_json::from_value(model).expect("a document");
    let statements = nook_model::forward_engineer(&doc, nook_core::Dialect::Postgres)
        .expect("postgres can defer");
    let creates = statements
        .iter()
        .filter(|s| s.trim_start().to_ascii_uppercase().starts_with("CREATE TABLE"))
        .count();
    let alters = statements
        .iter()
        .filter(|s| s.contains("ADD CONSTRAINT"))
        .count();
    assert_eq!(creates, 2, "{statements:#?}");
    assert_eq!(alters, 1, "{statements:#?}");
    // The ALTER comes last: every table has to exist before a key can reference it.
    assert!(
        statements.last().expect("a statement").contains("ADD CONSTRAINT"),
        "{statements:#?}"
    );
}

#[test]
fn sqlite_refuses_a_cycle_it_cannot_defer() {
    // SQLite has no `ALTER TABLE … ADD CONSTRAINT`, so the deferred half of a cycle cannot be
    // emitted there. A script that dies halfway is worse than a refusal that names the cycle.
    let mut f = Fixture::new("cycle-sqlite");
    let model = json!({
        "version": 1,
        "conn": "scratch",
        "tables": [
            {"name": "a", "kind": "table",
             "columns": [{"name": "id", "type_name": "INTEGER"}, {"name": "b_id", "type_name": "INTEGER"}],
             "foreign_keys": [{"name": "fk_a_b", "columns": ["b_id"], "ref_table": "b", "ref_columns": ["id"]}]},
            {"name": "b", "kind": "table",
             "columns": [{"name": "id", "type_name": "INTEGER"}, {"name": "a_id", "type_name": "INTEGER"}],
             "foreign_keys": [{"name": "fk_b_a", "columns": ["a_id"], "ref_table": "a", "ref_columns": ["id"]}]}
        ]
    });
    let mut params = json!({"model": model});
    params["conn"] = json!(f.conn);
    let err = f
        .session
        .run("diagram.forwardEngineer", params)
        .expect_err("sqlite cannot defer");
    let message = err.to_string();
    assert!(message.contains("cycle"), "{message}");
    assert!(message.contains("rebuild"), "{message}");
}

#[test]
fn the_dictionary_carries_columns_indexes_and_foreign_keys() {
    let mut f = Fixture::new("dict");
    let md = f.run(
        "tools.dataDictionary",
        json!({"format": "markdown", "schema": "main"}),
    );
    let text = md["text"].as_str().expect("text");
    assert!(text.contains("### posts") || text.contains("### main.posts"), "{text}");
    assert!(text.contains("| email | TEXT | no |"), "{text}");
    assert!(text.contains("ix_posts_title"), "{text}");
    assert!(text.contains("fk") || text.contains("Foreign keys"), "{text}");
    assert_eq!(md["tables"], 2);
    assert_eq!(md["columns"], 5);

    let html = f.run(
        "tools.dataDictionary",
        json!({"format": "html", "schema": "main"}),
    );
    let html_text = html["text"].as_str().expect("html");
    assert!(html_text.starts_with("<!doctype html>"), "{html_text}");
    assert!(html_text.contains("<table>"), "{html_text}");
}

#[test]
fn the_dictionary_refuses_a_format_it_does_not_write() {
    let mut f = Fixture::new("dict-format");
    let mut params = json!({"format": "pdf"});
    params["conn"] = json!(f.conn);
    let err = f
        .session
        .run("tools.dataDictionary", params)
        .expect_err("pdf is not written");
    assert!(err.to_string().contains("markdown or html"), "{err}");
}
