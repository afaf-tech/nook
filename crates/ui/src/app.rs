//! The application shell: ribbon-style menus built from the command registry, the tree, the tabs,
//! the message log, the dialogs, the control channel and the scheduled-job tick.

use datacraft_engine::{Conn, Level, Session, TabKind, TableRef};
use egui::RichText;
use serde_json::Value;

use crate::control::Control;
use crate::theme::Tokens;
use crate::{dialogs, grid, tree, views, Options};

/// The connection editor's working copy.
#[derive(Debug, Clone)]
pub struct ConnectionForm {
    pub name: String,
    pub kind: datacraft_core::DriverKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    pub file: String,
    pub group: String,
    pub ssl: datacraft_core::SslMode,
    pub read_only: bool,
    pub keychain: bool,
    pub ssh_enabled: bool,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub ssh_key: String,
}

impl Default for ConnectionForm {
    fn default() -> Self {
        ConnectionForm {
            name: String::new(),
            kind: datacraft_core::DriverKind::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            user: std::env::var("USER").unwrap_or_default(),
            password: String::new(),
            database: String::new(),
            file: String::new(),
            group: String::new(),
            ssl: datacraft_core::SslMode::Prefer,
            read_only: false,
            keychain: true,
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_user: String::new(),
            ssh_key: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DatagenForm {
    pub rows: usize,
    pub seed: u64,
    pub overrides: std::collections::BTreeMap<String, String>,
}

impl Default for DatagenForm {
    fn default() -> Self {
        DatagenForm {
            rows: 500,
            seed: 1,
            overrides: std::collections::BTreeMap::new(),
        }
    }
}

/// Everything the UI itself remembers between frames.
pub struct UiState {
    pub selected_cell: Option<(usize, usize)>,
    pub cell_edit: String,
    pub cell_editing: bool,
    pub filter_edit: String,
    pub connection_filter: Option<String>,
    pub connection_open: bool,
    pub connection_editing: Option<usize>,
    pub connection_form: ConnectionForm,
    pub command_open: bool,
    pub command_id: String,
    pub command_params: String,
    pub command_result: Option<String>,
    pub datagen_target: Option<(usize, TableRef)>,
    pub datagen_form: DatagenForm,
    pub export_table: Option<TableRef>,
    pub export_conn: Option<usize>,
    pub export_format: String,
    pub export_grid_format: Option<String>,
    pub settings_open: bool,
    pub palette_open: bool,
    pub palette_query: String,
    pub diagram_offset: egui::Vec2,
    pub diagram_zoom: f32,
    pub monitor_filter: String,
    pub active_tab_override: Option<usize>,
    pub want_demo: bool,
    pub apply_after_edit: bool,
    pub editor_dirty: bool,
    pub control: Option<Control>,
    pub shot: Option<String>,
    shot_requested: bool,
    /// True when the process was started with `--shot`: capture once and quit.
    shot_quit: bool,
    pub toast: Option<(String, f64)>,
    pub quit: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            selected_cell: None,
            cell_edit: String::new(),
            cell_editing: false,
            filter_edit: String::new(),
            connection_filter: None,
            connection_open: false,
            connection_editing: None,
            connection_form: ConnectionForm::default(),
            command_open: false,
            command_id: "sql.run".into(),
            command_params: "{\n  \"sql\": \"select 1\"\n}".into(),
            command_result: None,
            datagen_target: None,
            datagen_form: DatagenForm::default(),
            export_table: None,
            export_conn: None,
            export_format: "csv".into(),
            export_grid_format: None,
            settings_open: false,
            palette_open: false,
            palette_query: String::new(),
            diagram_offset: egui::vec2(0.0, 0.0),
            diagram_zoom: 1.0,
            monitor_filter: String::new(),
            active_tab_override: None,
            want_demo: false,
            apply_after_edit: false,
            editor_dirty: false,
            control: None,
            shot: None,
            shot_requested: false,
            shot_quit: false,
            toast: None,
            quit: false,
        }
    }
}

impl UiState {
    pub fn open_connection_dialog(&mut self, index: Option<usize>) {
        self.connection_open = true;
        self.connection_editing = index;
        if index.is_none() {
            self.connection_form = ConnectionForm::default();
        }
    }

    /// `id` for a plain command; `id&key=value` prefills those keys.
    pub fn open_command(&mut self, spec: String) {
        let (id, prefill) = match spec.split_once('&') {
            Some((id, rest)) => (id.to_string(), rest.to_string()),
            None => (spec, String::new()),
        };
        self.command_id = id.clone();
        let base = datacraft_engine::find(&id)
            .map(|s| s.params.to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "{}".into());
        let mut params: serde_json::Map<String, Value> =
            serde_json::from_str(&base).unwrap_or_default();
        for pair in prefill.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                if !key.is_empty() {
                    params.insert(key.to_string(), Value::String(value.to_string()));
                }
            }
        }
        self.command_params =
            serde_json::to_string_pretty(&Value::Object(params)).unwrap_or_else(|_| "{}".into());
        self.command_result = None;
        self.command_open = true;
    }

    pub fn open_datagen(&mut self, conn: usize, table: TableRef) {
        self.datagen_target = Some((conn, table));
        self.datagen_form = DatagenForm::default();
    }

    pub fn open_export(&mut self, conn: usize, table: TableRef) {
        self.export_conn = Some(conn);
        self.export_table = Some(table);
        self.export_grid_format = None;
        self.export_format = "sql".into();
    }

    pub fn open_export_grid(&mut self, format: &str) {
        self.export_table = None;
        self.export_grid_format = Some(format.to_string());
        self.export_format = format.to_string();
    }

    pub fn pick_save_path(&self, default: &str) -> Option<String> {
        rfd::FileDialog::new()
            .set_title("Save as")
            .set_file_name(default)
            .save_file()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Load the ER diagram into the active tab.
    pub fn request_diagram(&mut self, session: &mut Session, conn: usize) {
        let params = serde_json::json!({ "conn": session.conns[conn].cfg.id });
        match session.run("diagram.erDiagram", params) {
            Ok(value) => {
                if let Ok(tab) = session.active_tab_mut() {
                    tab.payload.insert("diagram".into(), value);
                    tab.conn = Some(conn);
                }
            }
            Err(e) => session.log(Level::Error, e.to_string()),
        }
    }

    pub fn request_monitor(&mut self, session: &mut Session, conn: usize, kind: &str) {
        let command = match kind {
            "processes" => "monitor.processes",
            "status" => "monitor.status",
            "variables" => "monitor.variables",
            _ => "monitor.dashboard",
        };
        let params = serde_json::json!({ "conn": session.conns[conn].cfg.id });
        match session.run(command, params) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("kind".into(), serde_json::json!(kind));
                }
                if let Ok(tab) = session.active_tab_mut() {
                    tab.payload.insert("monitor".into(), value);
                    tab.conn = Some(conn);
                }
            }
            Err(e) => session.log(Level::Error, e.to_string()),
        }
    }

    pub fn request_users(&mut self, session: &mut Session, conn: usize) {
        let params = serde_json::json!({ "conn": session.conns[conn].cfg.id });
        match session.run("priv.users", params) {
            Ok(value) => {
                if let Ok(tab) = session.active_tab_mut() {
                    tab.payload.insert("users".into(), value);
                    tab.conn = Some(conn);
                }
            }
            Err(e) => session.log(Level::Error, e.to_string()),
        }
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), 4.0));
    }
}

pub struct DatacraftApp {
    pub session: Session,
    pub state: UiState,
    last_job_tick: std::time::Instant,
    options: Options,
}

impl DatacraftApp {
    pub fn new(cc: &eframe::CreationContext, options: Options) -> Self {
        let mut session = if options.demo {
            crate::demo_session()
        } else {
            Session::with_profiles()
        };
        let tokens = Tokens::of(&session.settings.theme);
        tokens.apply(&cc.egui_ctx);

        let mut state = UiState::default();
        if let Some(port) = options.control_port {
            match Control::start(port) {
                Ok(control) => {
                    session.log(
                        Level::Info,
                        format!(
                            "control channel on 127.0.0.1:{} · key {}",
                            control.port,
                            control.key_path().display()
                        ),
                    );
                    state.control = Some(control);
                }
                Err(e) => session.log(Level::Error, format!("control channel: {e}")),
            }
        }
        for uri in &options.connect {
            match datacraft_core::ConnectionConfig::parse(uri) {
                Ok(cfg) => {
                    let id = cfg.id.clone();
                    if session.conn_index(&id).is_err() {
                        session.conns.push(Conn::new(cfg));
                    }
                    match session.conn_index(&id) {
                        Ok(index) => {
                            if let Err(e) = datacraft_engine::cmd::open_now(&mut session, index) {
                                session.log(Level::Error, format!("{uri}: {e}"));
                            }
                        }
                        Err(e) => session.log(Level::Error, format!("{uri}: {e}")),
                    }
                }
                Err(e) => session.log(Level::Error, format!("{uri}: {e}")),
            }
        }
        if let Some(sql) = options.sql.clone() {
            let index = session.add_tab(TabKind::Query, "Query", session.active_conn);
            if let Some(tab) = session.tabs.get_mut(index) {
                tab.sql.text = sql;
            }
        }
        state.shot = options.shot.clone();
        state.shot_quit = options.shot.is_some();
        DatacraftApp {
            session,
            state,
            last_job_tick: std::time::Instant::now(),
            options,
        }
    }

    fn tokens(&self) -> Tokens {
        Tokens::of(&self.session.settings.theme)
    }

    /// Run every job that has come due: the app is the scheduler.
    fn tick_jobs(&mut self) {
        if self.last_job_tick.elapsed().as_secs() < 30 {
            return;
        }
        self.last_job_tick = std::time::Instant::now();
        if self.session.jobs.jobs.iter().any(|j| j.enabled) {
            if let Err(e) = self.session.run("job.runDue", serde_json::json!({})) {
                self.session.log(Level::Warn, format!("scheduler: {e}"));
            }
        }
    }

    /// Handle the requests the control channel collected since the last frame.
    fn handle_control(&mut self, ctx: &egui::Context) {
        let Some(control) = self.state.control.as_mut() else {
            return;
        };
        let requests = control.drain();
        if requests.is_empty() {
            return;
        }
        let control_key_path = control.key_path().display().to_string();
        for request in requests {
            let reply = match request.method.as_str() {
                "engine.run" => {
                    let command = request
                        .params
                        .get("command")
                        .and_then(|c| c.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let params = request
                        .params
                        .get("params")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    match self.session.run(&command, params) {
                        Ok(value) => serde_json::json!({"ok": true, "result": value}),
                        Err(e) => serde_json::json!({"ok": false, "error": e.to_string()}),
                    }
                }
                "ui.state" => serde_json::json!({
                    "active_tab": self.session.active_tab,
                    "active_connection": self.session.active_conn,
                    "connections": self.session.conns.iter().map(|c| serde_json::json!({
                        "id": c.cfg.id, "name": c.cfg.name, "open": c.is_open(),
                        "catalog_tables": c.catalog.as_ref().map(|cat| cat.table_count()),
                    })).collect::<Vec<_>>(),
                    "tabs": self.session.tabs.iter().enumerate().map(|(index, t)| serde_json::json!({
                        "index": index, "title": t.title, "kind": format!("{:?}", t.kind).to_lowercase(),
                        "rows": t.grid.result.row_count(), "pending": t.grid.pending.len(),
                    })).collect::<Vec<_>>(),
                    "messages": self.session.messages.len(),
                    "control_key": control_key_path,
                }),
                "ui.selectTab" => {
                    let index = request
                        .params
                        .get("index")
                        .and_then(|i| i.as_i64())
                        .unwrap_or(0);
                    if index >= 0 && (index as usize) < self.session.tabs.len() {
                        self.session.active_tab = Some(index as usize);
                        serde_json::json!({"ok": true, "active_tab": index})
                    } else {
                        serde_json::json!({"ok": false, "error": "no such tab"})
                    }
                }
                "ui.setEditorText" => {
                    let sql = request
                        .params
                        .get("sql")
                        .and_then(|s| s.as_str())
                        .unwrap_or_default()
                        .to_string();
                    match self.session.active_tab_mut() {
                        Ok(tab) => {
                            tab.sql.text = sql;
                            serde_json::json!({"ok": true})
                        }
                        Err(e) => serde_json::json!({"ok": false, "error": e.to_string()}),
                    }
                }
                "ui.messages" => serde_json::json!({
                    "messages": self.session.messages.iter().rev().take(50).map(|m| serde_json::json!({
                        "level": format!("{:?}", m.level).to_lowercase(), "text": m.text,
                    })).collect::<Vec<_>>()
                }),
                "ui.commands" => serde_json::json!({
                    "count": datacraft_engine::registry().len(),
                    "ids": datacraft_engine::registry().iter().map(|c| c.id).collect::<Vec<_>>(),
                }),
                "ui.shot" => {
                    let path = request
                        .params
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("/tmp/datacraft.png")
                        .to_string();
                    self.state.shot = Some(path.clone());
                    self.state.shot_requested = false;
                    // A control-channel screenshot must not close the app.
                    self.state.shot_quit = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    serde_json::json!({"ok": true, "queued": path})
                }
                "ui.quit" => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    serde_json::json!({"ok": true})
                }
                other => serde_json::json!({
                    "ok": false,
                    "error": format!("unknown method '{other}'; try {}", crate::control::METHODS.join(", "))
                }),
            };
            let _ = request.reply.send(reply);
        }
    }

    /// `--shot`: render, ask for a screenshot, save it and quit. Used by the UI check.
    fn handle_shot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.state.shot.clone() else {
            return;
        };
        if !self.state.shot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.state.shot_requested = true;
            // Give the window a moment to settle before the capture.
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            return;
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&path, &image) {
                Ok(bytes) => eprintln!("screenshot: {path} ({bytes} bytes)"),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            self.state.shot = None;
            self.state.shot_requested = false;
            if self.state.shot_quit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                // The caller polls for the file; say so in the log.
                self.session.log(Level::Info, format!("screenshot written: {path}"));
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("DataCraft").strong().size(15.0));
            ui.separator();

            // Menus come from the registry: every group, every command.
            let mut groups: Vec<&'static str> = datacraft_engine::registry()
                .iter()
                .map(|c| c.group)
                .collect();
            groups.sort_unstable();
            groups.dedup();
            for group in groups {
                ui.menu_button(group, |ui| {
                    let mut current_sub: &str = "";
                    for spec in datacraft_engine::registry()
                        .iter()
                        .filter(|c| c.group == group)
                    {
                        // A cheap grouping: prefix before the dot becomes a submenu.
                        let prefix = spec.id.split('.').next().unwrap_or("");
                        if prefix != current_sub {
                            current_sub = prefix;
                            if current_sub.is_empty() {
                                continue;
                            }
                        }
                        let label = format!("{}  ({})", spec.label, spec.id);
                        if ui.button(label).clicked() {
                            self.state.open_command(spec.id.to_string());
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
            if ui
                .button(RichText::new("Palette").strong())
                .on_hover_text("Command palette (Cmd+K)")
                .clicked()
            {
                self.state.palette_open = true;
            }
            if ui.button("+ Connection").clicked() {
                self.state.open_connection_dialog(None);
            }
            let theme = self.session.settings.theme.clone();
            let icon = if theme == "dark" { "Light" } else { "Dark" };
            if ui.button(icon).on_hover_text("Switch theme").clicked() {
                let next = if theme == "dark" { "light" } else { "dark" };
                let _ = self.session.run(
                    "app.setSetting",
                    serde_json::json!({"key": "theme", "value": next}),
                );
            }
            if ui.button("Settings").clicked() {
                self.state.settings_open = true;
            }
            if ui.button("Jobs").clicked() {
                let index = self.session.add_tab(TabKind::Jobs, "Jobs", None);
                self.state.active_tab_override = Some(index);
            }
            if ui.button("Transfer wizard").clicked() {
                let index =
                    self.session
                        .add_tab(TabKind::Transfer, "Transfer", self.session.active_conn);
                self.state.active_tab_override = Some(index);
            }
            ui.separator();
            let report = datacraft_engine::parity::measure();
            ui.label(
                RichText::new(format!(
                    "{} commands · {} catalog features ({:.0}%)",
                    datacraft_engine::registry().len(),
                    report.total(),
                    report.percent()
                ))
                .size(11.0)
                .color(tokens.text_dim),
            );
            if let Some(control) = &self.state.control {
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "control :{} · {} requests",
                        control.port, control.served
                    ))
                    .size(11.0)
                    .color(tokens.text_dim),
                );
            }
        });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let mut select: Option<usize> = self.state.active_tab_override.take();
        let mut close: Option<usize> = None;

        egui::ScrollArea::horizontal()
            .id_salt("tab-strip")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, tab) in self.session.tabs.iter().enumerate() {
                        let active = self.session.active_tab == Some(index);
                        let dirty = tab.grid.dirty();
                        let label = format!(
                            "{}{}{}",
                            tab.title,
                            if dirty { " ●" } else { "" },
                            match tab.kind {
                                TabKind::Query => "  sql",
                                TabKind::Data => "  ",
                                TabKind::Design => "  ddl",
                                TabKind::Diagram => "  er",
                                TabKind::Report => "  rpt",
                                TabKind::Monitor => "  mon",
                                TabKind::Jobs => "  job",
                                TabKind::Transfer => "  xfer",
                                TabKind::Users => "  usr",
                                TabKind::Welcome => "",
                            }
                        );
                        let color = if dirty { tokens.warn } else { tokens.text };
                        if ui
                            .selectable_label(active, RichText::new(label).color(color))
                            .clicked()
                        {
                            select = Some(index);
                        }
                        if ui
                            .small_button("x")
                            .on_hover_text("Close the tab")
                            .clicked()
                        {
                            close = Some(index);
                        }
                        ui.separator();
                    }
                    if ui
                        .small_button("+")
                        .on_hover_text("New query tab")
                        .clicked()
                    {
                        let index =
                            self.session
                                .add_tab(TabKind::Query, "Query", self.session.active_conn);
                        select = Some(index);
                    }
                });
            });
        if let Some(index) = select {
            self.session.active_tab = Some(index);
        }
        if let Some(index) = close {
            if let Err(e) = self
                .session
                .run("tab.close", serde_json::json!({"index": index}))
            {
                self.session.log(Level::Warn, e.to_string());
            }
        }
    }

    fn central(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let Some(index) = self.session.active_tab else {
            views::welcome(ui, &mut self.session, &mut self.state, tokens);
            return;
        };
        if index >= self.session.tabs.len() {
            self.session.active_tab = Some(0);
            return;
        }
        let kind = self.session.tabs[index].kind;
        match kind {
            TabKind::Welcome => views::welcome(ui, &mut self.session, &mut self.state, tokens),
            TabKind::Query => {
                crate::editor::show(ui, &mut self.session, index, &mut self.state, tokens)
            }
            TabKind::Data => grid::show(ui, &mut self.session, index, &mut self.state, tokens),
            TabKind::Design => views::design(ui, &mut self.session, index, &mut self.state, tokens),
            TabKind::Diagram => {
                if !self.session.tabs[index].payload.contains_key("diagram") {
                    if let Some(conn) = self.session.tabs[index].conn {
                        self.state.request_diagram(&mut self.session, conn);
                    }
                }
                views::diagram(ui, &mut self.session, index, &mut self.state, tokens)
            }
            TabKind::Report => views::report(ui, &mut self.session, index, tokens),
            TabKind::Monitor => {
                if !self.session.tabs[index].payload.contains_key("monitor") {
                    if let Some(conn) = self.session.tabs[index].conn {
                        self.state
                            .request_monitor(&mut self.session, conn, "dashboard");
                    }
                }
                views::monitor(ui, &mut self.session, index, &mut self.state, tokens)
            }
            TabKind::Jobs => views::jobs(ui, &mut self.session, &mut self.state, tokens),
            TabKind::Transfer => {
                views::transfer(ui, &mut self.session, index, &mut self.state, tokens)
            }
            TabKind::Users => {
                if !self.session.tabs[index].payload.contains_key("users") {
                    if let Some(conn) = self.session.tabs[index].conn {
                        self.state.request_users(&mut self.session, conn);
                    }
                }
                views::users(ui, &mut self.session, index, &mut self.state, tokens)
            }
        }
    }

    fn message_log(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} messages", self.session.messages.len()))
                    .size(11.0)
                    .strong(),
            );
            if ui.small_button("clear").clicked() {
                let _ = self.session.run("app.clearLog", serde_json::json!({}));
            }
            if let Some((message, seconds)) = self.state.toast.clone() {
                if seconds > 0.0 {
                    ui.label(RichText::new(message.clone()).size(11.0).color(tokens.ok));
                    self.state.toast = Some((message, seconds - 0.02));
                }
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("messages")
            .max_height(90.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for message in self.session.messages.iter().rev().take(60) {
                    ui.label(
                        RichText::new(message.text.clone())
                            .size(11.0)
                            .monospace()
                            .color(tokens.level_color(message.level)),
                    );
                }
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let action = tree::show(ui, &self.session, &mut self.state, tokens);
        use tree::TreeAction;
        match action {
            TreeAction::None => {}
            TreeAction::OpenConnection(index) => {
                if let Err(e) = datacraft_engine::cmd::open_now(&mut self.session, index) {
                    self.session.log(Level::Error, e.to_string());
                }
            }
            TreeAction::CloseConnection(index) => {
                let _ = self.session.run(
                    "conn.close",
                    serde_json::json!({"conn": self.session.conns[index].cfg.id}),
                );
            }
            TreeAction::SelectConnection(index) => {
                self.session.active_conn = Some(index);
            }
            TreeAction::Refresh(index) => {
                let id = self.session.conns[index].cfg.id.clone();
                if let Err(e) = self
                    .session
                    .run("conn.refresh", serde_json::json!({"conn": id}))
                {
                    self.session.log(Level::Error, e.to_string());
                }
            }
            TreeAction::OpenData(index, reference) => {
                tree::open_data_tab(&mut self.session, index, reference);
            }
            TreeAction::OpenDesign(index, reference) => {
                let table = self
                    .session
                    .conns
                    .get(index)
                    .and_then(|c| c.catalog.as_ref())
                    .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
                    .cloned();
                let title = reference.key();
                let tab = self.session.add_tab(TabKind::Design, title, Some(index));
                if let Some(tab_ref) = self.session.tabs.get_mut(tab) {
                    tab_ref.table = table;
                    tab_ref.grid.table = Some(reference);
                }
            }
            TreeAction::OpenDdl(index, reference) => {
                let id = self.session.conns[index].cfg.id.clone();
                match self.session.run(
                    "schema.ddl",
                    serde_json::json!({"conn": id, "table": reference.key()}),
                ) {
                    Ok(value) => {
                        let sql = value["ddl"].as_str().unwrap_or("").to_string();
                        let tab = self.session.add_tab(
                            TabKind::Query,
                            format!("DDL {}", reference.table),
                            Some(index),
                        );
                        if let Some(tab_ref) = self.session.tabs.get_mut(tab) {
                            tab_ref.sql.text = sql;
                        }
                    }
                    Err(e) => self.session.log(Level::Error, e.to_string()),
                }
            }
            TreeAction::OpenDiagram(index) => {
                let tab = self
                    .session
                    .add_tab(TabKind::Diagram, "ER diagram", Some(index));
                self.state.active_tab_override = Some(tab);
                self.state.request_diagram(&mut self.session, index);
            }
            TreeAction::NewQuery(index) => {
                let tab = self.session.add_tab(TabKind::Query, "Query", Some(index));
                self.state.active_tab_override = Some(tab);
            }
            TreeAction::Monitor(index) => {
                let tab = self
                    .session
                    .add_tab(TabKind::Monitor, "Monitor", Some(index));
                self.state.active_tab_override = Some(tab);
                self.state
                    .request_monitor(&mut self.session, index, "dashboard");
            }
            TreeAction::Users(index) => {
                let tab = self.session.add_tab(TabKind::Users, "Users", Some(index));
                self.state.active_tab_override = Some(tab);
                self.state.request_users(&mut self.session, index);
            }
        }
    }

    /// Keyboard shortcuts that are not tied to the menu.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let (palette, run_query, apply, close_tab, format_sql) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::K),
                i.modifiers.command && i.key_pressed(egui::Key::Enter),
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::W),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::F),
            )
        });
        if palette {
            self.state.palette_open = true;
        }
        if run_query {
            let params = serde_json::json!({});
            if let Err(e) = self.session.run("sql.run", params) {
                self.session.log(Level::Warn, e.to_string());
            }
        }
        if apply {
            if let Err(e) = self.session.run("grid.apply", serde_json::json!({})) {
                self.session.log(Level::Warn, e.to_string());
            }
        }
        if format_sql {
            if let Err(e) = self.session.run("sql.format", serde_json::json!({})) {
                self.session.log(Level::Warn, e.to_string());
            }
        }
        if close_tab {
            if let Some(index) = self.session.active_tab {
                if let Err(e) = self
                    .session
                    .run("tab.close", serde_json::json!({"index": index}))
                {
                    self.session.log(Level::Warn, e.to_string());
                }
            }
        }
    }
}

impl eframe::App for DatacraftApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let tokens = self.tokens();
        tokens.apply(&ctx);

        self.handle_control(&ctx);
        self.handle_shot(&ctx);
        self.shortcuts(&ctx);
        self.tick_jobs();

        if self.state.want_demo {
            self.state.want_demo = false;
            self.session = crate::demo_session();
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            self.top_bar(ui, &tokens);
            ui.separator();
            self.tab_strip(ui, &tokens);
        });

        egui::Panel::bottom("messages")
            .resizable(true)
            .default_size(110.0)
            .show(ui, |ui| self.message_log(ui, &tokens));

        egui::Panel::left("tree")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.sidebar(ui, &tokens));

        egui::CentralPanel::default_margins().show(ui, |ui| self.central(ui, &tokens));

        dialogs::show(ui, &mut self.session, &mut self.state, &tokens);

        if self.state.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let _ = &self.options;
    }
}

/// Write an egui screenshot as a PNG.
fn save_png(path: &str, image: &egui::ColorImage) -> Result<usize, String> {
    let (width, height) = (image.width() as u32, image.height() as u32);
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for pixel in &image.pixels {
        bytes.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), pixel.a()]);
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes.len())
}
