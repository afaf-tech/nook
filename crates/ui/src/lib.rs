#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L6: the desktop front end. Thin on purpose — it reads `Session` state, paints it, and dispatches
//! every action through the command registry, exactly like the CLI and MCP do.

pub mod app;
pub mod control;
pub mod dialogs;
pub mod editor;
pub mod grid;
pub mod table;
pub mod theme;
pub mod tree;
pub mod views;

pub use app::DatacraftApp;

/// Options the desktop binary passes in.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Bind the JSON control channel to this port.
    pub control_port: Option<u16>,
    /// Render one frame, write a PNG here and exit.
    pub shot: Option<String>,
    /// Connections given as URIs on the command line.
    pub connect: Vec<String>,
    /// SQL to load into the first query tab.
    pub sql: Option<String>,
    /// Create a sample SQLite database and open it.
    pub demo: bool,
}

/// Build the sample database used by `--demo` and by the screenshot check.
pub fn demo_session() -> datacraft_engine::Session {
    use datacraft_core::{ConnectionConfig, Value};
    use datacraft_engine::{Conn, Level, Session, TabKind};

    let path = std::env::temp_dir().join("datacraft-demo.db");
    let _ = std::fs::remove_file(&path);
    let cfg = ConnectionConfig::sqlite(path.to_string_lossy());
    let mut session = Session::new();
    session.conns.push(Conn::new(cfg));
    if let Err(e) = datacraft_engine::cmd::open_now(&mut session, 0) {
        session.log(Level::Error, format!("demo setup failed: {e}"));
        return session;
    }
    let setup = "\
        CREATE TABLE customers (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, email TEXT, city TEXT, joined TEXT);\
        CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), total REAL, status TEXT);\
        CREATE INDEX ix_orders_customer ON orders(customer_id);\
        INSERT INTO customers (name, email, city, joined) VALUES \
          ('Ada Lovelace','ada@example.com','London','2026-01-04'),\
          ('Grace Hopper','grace@example.com','New York','2026-01-19'),\
          ('Linus Torvalds','linus@example.com','Helsinki','2026-02-02'),\
          ('Margaret Hamilton','margaret@example.com','Paoli','2026-02-21'),\
          ('Barbara Liskov','barbara@example.com','Boston','2026-03-11'),\
          ('Ken Thompson','ken@example.com','New Orleans','2026-03-30');\
        INSERT INTO orders (customer_id, total, status) VALUES \
          (1, 120.50, 'paid'), (1, 42.00, 'paid'), (2, 7.25, 'pending'), (3, 999.99, 'paid'),\
          (4, 15.00, 'refunded'), (5, 250.75, 'pending'), (6, 60.00, 'paid'), (2, 31.40, 'paid');";
    if let Err(e) = datacraft_engine::cmd::run_sql(&mut session, 0, setup, Some(10)) {
        session.log(Level::Error, format!("demo data failed: {e}"));
        return session;
    }
    let _ = datacraft_engine::cmd::refresh_catalog(&mut session, 0);
    let index = session.add_tab(TabKind::Query, "Query", Some(0));
    if let Some(tab) = session.tabs.get_mut(index) {
        tab.sql.text = "SELECT c.city, count(o.id) AS orders, round(sum(o.total), 2) AS revenue\n  FROM customers c\n  LEFT JOIN orders o ON o.customer_id = c.id\n GROUP BY c.city\n ORDER BY revenue DESC;".into();
    }
    let query = session
        .tabs
        .get(index)
        .map(|tab| tab.sql.text.clone())
        .unwrap_or_default();
    if let Err(e) = session.run(
        "sql.run",
        serde_json::json!({"conn": session.conns[0].cfg.id.clone(), "sql": query, "limit": 200}),
    ) {
        session.log(Level::Warn, format!("demo query: {e}"));
    }
    let monitor_tab = session.add_tab(TabKind::Monitor, "Monitor", Some(0));
    if let Some(tab) = session.tabs.get_mut(monitor_tab) {
        tab.payload
            .insert("monitor".into(), serde_json::json!({"kind": "dashboard"}));
    }
    let data_tab = session.add_tab(TabKind::Data, "main.customers", Some(0));
    let _ = datacraft_engine::cmd::bind_grid_table(
        &mut session,
        data_tab,
        datacraft_engine::TableRef {
            schema: None,
            table: "customers".into(),
        },
    );
    let _ = datacraft_engine::cmd::load_grid(&mut session, data_tab);
    let _ = Value::Int(0);
    session.log(Level::Success, "sample database ready (customers, orders)");
    session
}

/// Called by the desktop binary: install fonts, run the app.
pub fn run(options: Options) -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("DataCraft"),
        ..Default::default()
    };
    eframe::run_native(
        "DataCraft",
        native_options,
        Box::new(move |cc| Ok(Box::new(DatacraftApp::new(cc, options.clone())))),
    )
}
