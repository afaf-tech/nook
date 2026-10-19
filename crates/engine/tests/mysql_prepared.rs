use nook_engine::cmd::open_now;
use nook_engine::{Conn, Session};
use serde_json::json;

#[test]
fn mysql_prepared_binds_and_decodes_typed_columns() {
    let uri = std::env::var("NOOK_MYSQL_URI").unwrap_or_default();
    if uri.is_empty() { return; }
    std::env::set_var("NOOK_CONFIG_DIR", "/tmp/nook-mysqlcfg");
    let cfg = nook_core::ConnectionConfig::parse(&uri).expect("uri parses");
    let mut session = Session::new();
    session.conns.push(Conn::new(cfg));
    let index = session.conn_index(&session.conns[0].cfg.id.clone()).unwrap();
    open_now(&mut session, index).expect("pg opens");
    let out = session.run("sql.run", json!({
        "sql": "select email, price, at from u where email = :email",
        "params": {"email": "a@b.c"},
    })).expect("prepared run");
    println!("OUT {}", serde_json::to_string_pretty(&out).unwrap());
    assert_eq!(out["prepared"], true);
    assert_eq!(out["rows"], 1);
    assert_eq!(out["data"][0][0], "a@b.c");
    assert_eq!(out["data"][0][1], "12.34", "numeric decoded exactly");
    assert_eq!(out["data"][0][2], "2026-01-02 03:04:05", "timestamp decoded");
}
