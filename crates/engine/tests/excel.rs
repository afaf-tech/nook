//! Excel import and export against a real `.xlsx` file: `tools.importExcel` lands rows from the
//! first sheet, `tools.exportExcel` writes a file calamine can read back. Both ends assert the
//! stored rows themselves, not just the command's reply.

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
        let dir = std::env::temp_dir().join("nook-excel");
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NOOK_CONFIG_DIR", dir.join("config"));
        let db = dir.join(format!("{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);

        let cfg = nook_core::ConnectionConfig::parse(&format!("sqlite:{}", db.display()))
            .expect("a sqlite URI parses");
        let conn = cfg.id.clone();
        let mut session = Session::new();
        session.conns.push(Conn::new(cfg));
        let index = session
            .conn_index(&conn)
            .expect("the connection is registered");
        open_now(&mut session, index).expect("sqlite opens");
        Fixture { session, conn, dir }
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
}

#[test]
fn import_excel_lands_rows_and_respects_column_mapping() {
    let mut f = Fixture::new("import");
    let source = f.dir.join("users.xlsx");
    nook_drivers::xlsx::write_rows(
        &source,
        &[
            vec!["id".into(), "email".into()],
            vec!["1".into(), "ann@example.com".into()],
            vec!["2".into(), "bob@example.com".into()],
        ],
    )
    .expect("write source workbook");

    let out = f
        .session
        .run(
            "tools.importExcel",
            json!({"path": source.display().to_string(), "table": "users", "conn": f.conn}),
        )
        .expect("import");
    assert_eq!(out["inserted"], 2);
    assert_eq!(out["format"], "xlsx");

    let rows = f.rows("select id, email from users order by id");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1], "ann@example.com");
    assert_eq!(rows[1][1], "bob@example.com");

    // A target with differently ordered columns still lands the values by name.
    f.create("create table mirror (email text, id integer)");
    let mapped = f
        .session
        .run(
            "tools.importExcel",
            json!({
                "path": source.display().to_string(),
                "table": "mirror",
                "conn": f.conn,
                "columnMapping": ["id=id", "email=email"]
            }),
        )
        .expect("mapped import");
    assert_eq!(mapped["inserted"], 2);
    let rows = f.rows("select email from mirror where id = 1");
    assert_eq!(rows[0][0], "ann@example.com");
}

#[test]
fn export_excel_writes_a_file_that_reads_back() {
    let mut f = Fixture::new("export");
    f.create("create table t (id integer, name text)");
    f.session
        .run(
            "sql.run",
            json!({"sql": "insert into t values (1,'Ann'),(2,'Bob & <Co>')" , "conn": f.conn}),
        )
        .expect("seed");

    let target = f.dir.join("exported.xlsx");
    let out = f
        .session
        .run(
            "tools.exportExcel",
            json!({"sql": "select id, name from t order by id", "path": target.display().to_string(), "conn": f.conn}),
        )
        .expect("export");
    assert_eq!(out["rows"], 2);
    assert_eq!(out["format"], "xlsx");

    let read_back = nook_drivers::xlsx::read_rows(&target).expect("the export reads back");
    assert_eq!(
        read_back,
        vec![
            vec!["id".to_string(), "name".to_string()],
            vec!["1".to_string(), "Ann".to_string()],
            vec!["2".to_string(), "Bob & <Co>".to_string()],
        ]
    );
}

impl Fixture {
    fn create(&mut self, ddl: &str) {
        self.session
            .run("sql.run", json!({"sql": ddl, "conn": self.conn}))
            .expect("create table");
    }
}
