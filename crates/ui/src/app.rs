//! The application shell: app bar with real menus, tab strip, status bar, log drawer, sidebar,
//! dialogs, control channel and the scheduled-job tick.

use datacraft_engine::{Conn, Level, Session, TabKind, TableRef};
use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, Vec2};
use serde_json::Value;

use crate::control::Control;
use crate::table::TableState;
use crate::theme::{self as t, Tokens, APP_BAR_H, STATUS_H, TAB_H};
use crate::{dialogs, grid, tree, views, Options};

/// Top-level menus. Each one owns registry groups, so the registry stays the single list of actions
/// while the bar stays readable.
const MENUS: &[(&str, &[&str])] = &[
    ("File", &["Connection", "App"]),
    ("Data", &["Data", "Design", "Modeler", "Tools"]),
    ("Query", &["Query", "Report"]),
    (
        "Server",
        &["Monitor", "Privileges", "Backup", "Sync", "Transfer"],
    ),
    ("Automation", &["Automation"]),
    ("Help", &["Help"]),
];

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
    /// Inline-editor state, one entry per tab (data grid and query results).
    pub tables: std::collections::BTreeMap<usize, TableState>,
    pub filter_edit: String,
    /// Sidebar search text and the expanded connection/schema keys.
    pub tree_filter: String,
    pub tree_expanded: std::collections::BTreeSet<String>,
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
    /// Log drawer: collapsed by default so the data gets the room.
    pub log_open: bool,
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
            tables: std::collections::BTreeMap::new(),
            filter_edit: String::new(),
            tree_filter: String::new(),
            tree_expanded: std::collections::BTreeSet::new(),
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
            log_open: false,
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
    /// Inline-editor state for one tab.
    pub fn table(&mut self, index: usize) -> &mut TableState {
        self.tables.entry(index).or_default()
    }

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
                    if index < 0 {
                        // Negative index: show the start page, which has no tab of its own.
                        self.session.active_tab = None;
                        serde_json::json!({"ok": true, "active_tab": null})
                    } else if (index as usize) < self.session.tabs.len() {
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
                "ui.setTheme" => {
                    let theme = request
                        .params
                        .get("theme")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default();
                    // Delegate to the command the Settings button uses, so the control channel cannot
                    // accept a value the product refuses (and vice versa).
                    match self.session.run(
                        "app.setSetting",
                        serde_json::json!({"key": "theme", "value": theme}),
                    ) {
                        Ok(value) => {
                            ctx.request_repaint();
                            serde_json::json!({"ok": true, "result": value})
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
                self.session
                    .log(Level::Info, format!("screenshot written: {path}"));
            }
        }
    }

    /// Application bar: identity, menu bar, search, and the state cluster on the right.
    fn app_bar(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        t::band_h(ui, tokens, APP_BAR_H, |ui| {
            ui.horizontal(|ui| {
                let (mark_rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
                ui.painter()
                    .rect_filled(mark_rect, CornerRadius::same(6), tokens.accent);
                ui.painter().text(
                    mark_rect.center(),
                    Align2::CENTER_CENTER,
                    "DC",
                    tokens.font(11.5),
                    tokens.accent_text,
                );
                ui.add_space(2.0);
                ui.label(RichText::new("DataCraft").size(13.5).color(tokens.text));
                ui.add_space(6.0);
                t::divider(ui, tokens);

                for (menu, groups) in MENUS {
                    ui.menu_button(RichText::new(*menu).size(12.5), |ui| {
                        ui.set_min_width(280.0);
                        for group in *groups {
                            let specs: Vec<&'static datacraft_engine::CommandSpec> =
                                datacraft_engine::registry()
                                    .iter()
                                    .filter(|c| c.group == *group)
                                    .collect();
                            if specs.is_empty() {
                                continue;
                            }
                            ui.menu_button(RichText::new(*group).size(12.5), |ui| {
                                ui.set_min_width(340.0);
                                for spec in specs {
                                    let response = ui.button(
                                        RichText::new(spec.label).size(12.5).color(tokens.text),
                                    );
                                    if response.clicked() {
                                        self.state.open_command(spec.id.to_string());
                                        ui.close();
                                    }
                                    if !spec.help.is_empty() {
                                        response.on_hover_text(spec.help);
                                    }
                                }
                            });
                        }
                    });
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if t::ghost_button(ui, tokens, "Settings", true)
                        .on_hover_text("Preferences")
                        .clicked()
                    {
                        self.state.settings_open = true;
                    }
                    let theme = self.session.settings.theme.clone();
                    let next = if theme == "dark" { "light" } else { "dark" };
                    if t::ghost_button(
                        ui,
                        tokens,
                        if theme == "dark" { "Light" } else { "Dark" },
                        true,
                    )
                    .on_hover_text(format!("Switch to the {next} theme"))
                    .clicked()
                    {
                        let _ = self.session.run(
                            "app.setSetting",
                            serde_json::json!({"key": "theme", "value": next}),
                        );
                    }
                    if t::ghost_button(ui, tokens, "Jobs", true)
                        .on_hover_text("Scheduled jobs")
                        .clicked()
                    {
                        let index = self.session.add_tab(TabKind::Jobs, "Jobs", None);
                        self.state.active_tab_override = Some(index);
                    }

                    // Search field, which is also the command palette.
                    let label = RichText::new("Search or run a command")
                        .size(12.0)
                        .color(tokens.text_faint);
                    let response = ui.add(
                        egui::Button::new(label)
                            .fill(tokens.background)
                            .stroke(Stroke::new(1.0, tokens.stroke))
                            .corner_radius(CornerRadius::same(6))
                            .min_size(Vec2::new(230.0, 24.0)),
                    );
                    if response.clicked() {
                        self.state.palette_open = true;
                        self.state.palette_query.clear();
                    }
                    response.on_hover_text("Command palette (Cmd+K)");
                    t::keycap(ui, tokens, "Cmd K");
                    t::divider(ui, tokens);
                });
            });
        });
    }

    /// Tab strip: painted tabs, accent underline on the active one, close on hover.
    fn tab_strip(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let mut select: Option<usize> = self.state.active_tab_override.take();
        let mut close: Option<usize> = None;

        egui::ScrollArea::horizontal()
            .id_salt("tab-strip")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (index, tab) in self.session.tabs.iter().enumerate() {
                        let active = self.session.active_tab == Some(index);
                        let dirty = tab.grid.dirty();
                        let pending = tab.grid.pending.len();
                        let text_width = ui
                            .painter()
                            .layout_no_wrap(
                                tab.title.clone(),
                                tokens.font(12.5),
                                Color32::PLACEHOLDER,
                            )
                            .size()
                            .x;
                        let width = (text_width + if dirty || pending > 0 { 74.0 } else { 52.0 })
                            .clamp(96.0, 260.0);
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::new(width, TAB_H), Sense::click());
                        let hovered = response.hovered();

                        if active {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_max(
                                    rect.min,
                                    egui::pos2(rect.right(), rect.bottom() + 1.0),
                                ),
                                CornerRadius {
                                    nw: tokens.radius,
                                    ne: tokens.radius,
                                    sw: 0,
                                    se: 0,
                                },
                                tokens.panel,
                            );
                        } else if hovered {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::ZERO, tokens.hover);
                        }
                        if active {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(rect.left() + 8.0, rect.bottom() - 2.0),
                                    Vec2::new((rect.width() - 16.0).max(8.0), 2.0),
                                ),
                                CornerRadius::same(1),
                                tokens.accent,
                            );
                        }

                        let mut x = rect.left() + 12.0;
                        if dirty || pending > 0 {
                            ui.painter().circle_filled(
                                egui::pos2(x + 3.0, rect.center().y),
                                3.0,
                                tokens.warn,
                            );
                            x += 12.0;
                        }
                        ui.painter().text(
                            egui::pos2(x, rect.center().y),
                            Align2::LEFT_CENTER,
                            &tab.title,
                            tokens.font(12.5),
                            if active { tokens.text } else { tokens.text_dim },
                        );

                        // Close cross: always on the active tab, on hover for the rest.
                        if active || hovered {
                            let close_rect = egui::Rect::from_center_size(
                                egui::pos2(rect.right() - 15.0, rect.center().y),
                                Vec2::new(18.0, 18.0),
                            );
                            let close_response = ui.interact(
                                close_rect,
                                ui.id().with(("close", index)),
                                Sense::click(),
                            );
                            if close_response.hovered() {
                                ui.painter().rect_filled(
                                    close_rect,
                                    CornerRadius::same(4),
                                    tokens.panel_high,
                                );
                            }
                            ui.painter().text(
                                close_rect.center(),
                                Align2::CENTER_CENTER,
                                "×",
                                tokens.font(13.0),
                                if close_response.hovered() {
                                    tokens.error
                                } else {
                                    tokens.text_dim
                                },
                            );
                            if close_response.clicked() {
                                close = Some(index);
                            }
                        }

                        ui.painter().vline(
                            rect.right(),
                            rect.top() + 8.0..=rect.bottom() - 8.0,
                            Stroke::new(1.0, tokens.stroke),
                        );
                        if response.clicked() {
                            select = Some(index);
                        }
                        if response.middle_clicked() {
                            close = Some(index);
                        }
                    }

                    ui.add_space(2.0);
                    if t::ghost_button(ui, tokens, "+", true)
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
            TabKind::Report => views::report(ui, &mut self.session, index, &mut self.state, tokens),
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

    /// Status bar: one line, everything glanceable.
    fn status_bar(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let pending: usize = self
            .session
            .tabs
            .iter()
            .map(|tab| tab.grid.pending.len())
            .sum();
        let mut toggle_log = false;
        let features = format!(
            "{}/{} features",
            datacraft_engine::parity::measure().covered.len(),
            datacraft_engine::parity::measure().total()
        );
        t::band_h(ui, tokens, STATUS_H, |ui| {
            ui.horizontal(|ui| {
                match self
                    .session
                    .active_conn
                    .and_then(|i| self.session.conns.get(i))
                {
                    Some(conn) => {
                        t::dot(
                            ui,
                            if conn.is_open() {
                                tokens.ok
                            } else {
                                tokens.error
                            },
                        );
                        ui.label(RichText::new(&conn.cfg.name).size(12.0).color(tokens.text));
                        let cfg = &conn.cfg;
                        let target = match cfg.kind {
                            datacraft_core::DriverKind::Sqlite => cfg.file.clone(),
                            _ => {
                                let (host, port) = cfg.target();
                                let db = if cfg.database.is_empty() {
                                    String::new()
                                } else {
                                    format!("/{}", cfg.database)
                                };
                                format!("{}@{}:{}{}", cfg.user, host, port, db)
                            }
                        };
                        ui.label(RichText::new(target).size(11.5).color(tokens.text_faint));
                    }
                    None => {
                        t::dot(ui, tokens.text_faint);
                        ui.label(
                            RichText::new("no connection")
                                .size(12.0)
                                .color(tokens.text_dim),
                        );
                    }
                }

                if let Some(index) = self.session.active_tab {
                    if let Some(tab) = self.session.tabs.get(index) {
                        t::divider(ui, tokens);
                        let (rows, ms) = match tab.kind {
                            TabKind::Query => (
                                tab.sql.last_result.as_ref().map(|r| r.row_count()),
                                tab.sql.last_result.as_ref().map(|r| r.elapsed_ms),
                            ),
                            TabKind::Data => (
                                Some(tab.grid.result.row_count()),
                                Some(tab.grid.result.elapsed_ms),
                            ),
                            _ => (None, None),
                        };
                        let mut line = tab.title.clone();
                        if let Some(rows) = rows {
                            line.push_str(&format!(" · {rows} rows"));
                        }
                        if let Some(ms) = ms {
                            line.push_str(&format!(" · {ms:.0} ms"));
                        }
                        ui.label(RichText::new(line).size(11.5).color(tokens.text_dim));
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(control) = &self.state.control {
                        t::chip(
                            ui,
                            tokens,
                            &format!("control {}", control.port),
                            tokens.info,
                        )
                        .on_hover_text("JSON control channel listening on 127.0.0.1");
                    }
                    if pending > 0 {
                        t::chip(ui, tokens, &format!("{pending} pending"), tokens.warn);
                    }
                    t::chip(ui, tokens, &features, tokens.text_faint);
                    let level = self
                        .session
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.level != Level::Info)
                        .map(|m| m.level)
                        .unwrap_or(Level::Info);
                    let colour = tokens.level_color(level);
                    let label = format!("{} messages", self.session.messages.len());
                    if t::chip(
                        ui,
                        tokens,
                        &label,
                        if self.state.log_open {
                            tokens.accent
                        } else {
                            colour
                        },
                    )
                    .interact(Sense::click())
                    .on_hover_text("Show or hide the log")
                    .clicked()
                    {
                        toggle_log = true;
                    }
                    if let Some((message, seconds)) = self.state.toast.clone() {
                        if seconds > 0.0 {
                            ui.label(RichText::new(&message).size(11.5).color(tokens.ok));
                            self.state.toast = Some((message, seconds - 0.05));
                        }
                    }
                });
            });
        });
        if toggle_log {
            self.state.log_open = !self.state.log_open;
        }
    }

    fn log_panel(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Log").size(12.0).color(tokens.text));
            ui.label(
                RichText::new(format!("{} entries", self.session.messages.len()))
                    .size(11.0)
                    .color(tokens.text_faint),
            );
            if t::ghost_button(ui, tokens, "Clear", true).clicked() {
                let _ = self.session.run("app.clearLog", serde_json::json!({}));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if t::ghost_button(ui, tokens, "Hide", true).clicked() {
                    self.state.log_open = false;
                }
            });
        });
        egui::ScrollArea::vertical()
            .id_salt("messages")
            .max_height(ui.available_height() - 4.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for message in self.session.messages.iter().rev().take(200) {
                    ui.horizontal(|ui| {
                        t::dot(ui, tokens.level_color(message.level));
                        ui.label(
                            RichText::new(&message.text)
                                .size(11.5)
                                .monospace()
                                .color(tokens.level_color(message.level)),
                        );
                    });
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
        let (palette, run_query, apply, close_tab, format_sql, toggle_log) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::K),
                i.modifiers.command && i.key_pressed(egui::Key::Enter),
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::W),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::F),
                i.modifiers.command && i.key_pressed(egui::Key::L),
            )
        });
        if palette {
            self.state.palette_open = true;
        }
        if run_query {
            if let Err(e) = self.session.run("sql.run", serde_json::json!({})) {
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
        if toggle_log {
            self.state.log_open = !self.state.log_open;
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
        // egui only repaints on input; with the control channel open, wake up regularly so a
        // request from a script is answered even when nobody is touching the window.
        if self.state.control.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        self.shortcuts(&ctx);
        self.tick_jobs();

        if self.state.want_demo {
            self.state.want_demo = false;
            self.session = crate::demo_session();
        }

        egui::Panel::top("app-bar")
            .frame(t::top_panel_frame(&tokens, APP_BAR_H))
            .show(ui, |ui| self.app_bar(ui, &tokens));
        egui::Panel::top("tab-strip")
            .frame(t::top_panel_frame(&tokens, TAB_H))
            .show(ui, |ui| self.tab_strip(ui, &tokens));

        egui::Panel::bottom("status-bar")
            .frame(t::top_panel_frame(&tokens, STATUS_H))
            .show(ui, |ui| self.status_bar(ui, &tokens));
        if self.state.log_open {
            egui::Panel::bottom("log")
                .resizable(true)
                .default_size(150.0)
                .max_size(400.0)
                .frame(t::top_panel_frame(&tokens, 120.0))
                .show(ui, |ui| self.log_panel(ui, &tokens));
        }

        egui::Panel::left("tree")
            .resizable(true)
            .default_size(t::TREE_W)
            .frame(t::side_panel_frame(&tokens))
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
