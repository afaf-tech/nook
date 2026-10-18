//! The application shell: app bar with real menus, tab strip, status bar, log drawer, sidebar,
//! dialogs, control channel and the scheduled-job tick.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Vec2};
use nook_engine::{Conn, Density, Level, Session, TabKind, TableRef};
use serde_json::Value;

use crate::control::Control;
use crate::icons::{self, Glyph};
use crate::table::TableState;
use crate::theme::{self as t, Tokens, APP_BAR_H, STATUS_H, TAB_BAR_H, TAB_H};
use crate::{dialogs, grid, tree, views, Options};

/// Top-level menus. Each one owns registry groups, so the registry stays the single list of actions
/// while the bar stays readable.
const MENUS: &[(&str, &[&str])] = &[
    ("File", &["Connection", "App"]),
    ("Data", &["Data", "Design", "Modeler", "Tools"]),
    ("Query", &["Query", "AI", "Report"]),
    (
        "Server",
        &["Monitor", "Privileges", "Backup", "Sync", "Transfer"],
    ),
    ("Automation", &["Automation"]),
    ("Help", &["Help"]),
];

/// Every field `ui.setConnectionField` accepts, in the connection editor's own order. The refusal
/// lists these names, so a script learns the whole surface from one bad call.
const CONNECTION_FIELDS: &[&str] = &[
    "name",
    "kind",
    "host",
    "port",
    "user",
    "password",
    "database",
    "file",
    "group",
    "ssl",
    "read_only",
    "keychain",
    "timeout",
    "step",
    "ssh_host",
    "ssh_port",
    "ssh_user",
    "ssh_key",
    "ssh_auth",
    "ssh_password",
    "ssh_save_password",
    "ssh_local_port",
];

/// The connection editor's steps, in the design's tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnStep {
    General,
    Tls,
    Tunnel,
    Advanced,
}

impl ConnStep {
    pub const ALL: [ConnStep; 4] = [
        ConnStep::General,
        ConnStep::Tls,
        ConnStep::Tunnel,
        ConnStep::Advanced,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConnStep::General => "General",
            ConnStep::Tls => "TLS",
            ConnStep::Tunnel => "Tunnel",
            ConnStep::Advanced => "Advanced",
        }
    }

    pub fn parse(label: &str) -> Option<ConnStep> {
        ConnStep::ALL
            .into_iter()
            .find(|step| step.label().eq_ignore_ascii_case(label))
    }
}

/// How the tunnel authenticates. The design's one-of-three is expressed from the fields the profile
/// already carries (`key_file`, a stored password) plus the user's own choice when the two collide:
/// a stored password and a key file can both be set, and the segmented control is what says which one
/// the tunnel uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshAuth {
    KeyFile,
    Agent,
    Password,
}

impl SshAuth {
    pub const ALL: [SshAuth; 3] = [SshAuth::KeyFile, SshAuth::Agent, SshAuth::Password];

    pub fn label(self) -> &'static str {
        match self {
            SshAuth::KeyFile => "Key file",
            SshAuth::Agent => "Agent",
            SshAuth::Password => "Password",
        }
    }
}

/// A destructive action waiting for the user's confirmation: the command, the parameters it will run
/// with, the object's name and what the user has typed so far. The engine's own `confirm` gate is what
/// makes this required rather than decorative.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub command: String,
    pub params: Value,
    /// `.dlg-t`: what is about to happen.
    pub title: String,
    /// `.dlg-sub`: the object, in mono.
    pub subject: String,
    /// `.para`: the sentence that explains it.
    pub sentence: String,
    /// `.facts`: the chips under the sentence.
    pub facts: Vec<String>,
    /// The typed-name gate: the button's label, and the exact text it waits for.
    pub action: String,
    pub typed: String,
    pub expected: String,
}

impl Confirm {
    /// The typed name matches, so the dangerous button is live.
    pub fn satisfied(&self) -> bool {
        self.typed.trim() == self.expected
    }

    /// The `.why` sentence beside the action row.
    pub fn why(&self) -> String {
        if self.satisfied() {
            format!("matches {}", self.expected)
        } else {
            format!("type {} exactly to enable {}", self.expected, self.action)
        }
    }
}

/// One `conn.test` result, kept so the pinned row survives the frame that produced it. A test is a
/// fact about the connection, not a log line.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub ok: bool,
    pub line: String,
}

/// The connection editor's working copy.
#[derive(Debug, Clone)]
pub struct ConnectionForm {
    pub step: ConnStep,
    pub name: String,
    pub kind: nook_core::DriverKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    pub file: String,
    pub group: String,
    pub ssl: nook_core::SslMode,
    /// `col.label()`; `""` when the profile carries none.
    pub color: String,
    /// Seconds; the engine clamps to 1..=300.
    pub timeout: u16,
    pub read_only: bool,
    pub keychain: bool,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub ssh_key: String,
    pub ssh_auth: SshAuth,
    pub ssh_password: String,
    pub ssh_save_password: bool,
    /// 0 = let the tunnel pick a free one, which is the design's `auto`.
    pub ssh_local_port: u16,
}

impl Default for ConnectionForm {
    fn default() -> Self {
        ConnectionForm {
            step: ConnStep::General,
            name: String::new(),
            kind: nook_core::DriverKind::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            user: std::env::var("USER").unwrap_or_default(),
            password: String::new(),
            database: String::new(),
            file: String::new(),
            group: String::new(),
            ssl: nook_core::SslMode::Prefer,
            color: String::new(),
            timeout: 15,
            read_only: false,
            keychain: nook_engine::profiles::keychain_available(),
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_user: String::new(),
            ssh_key: String::new(),
            ssh_auth: SshAuth::Agent,
            ssh_password: String::new(),
            ssh_save_password: false,
            ssh_local_port: 0,
        }
    }
}

impl ConnectionForm {
    /// The form for an existing profile. Secrets are never read back: the profile keeps only a
    /// reference, and the field stays empty until the user types a new one, which is what makes an
    /// untouched save unable to rewrite the keychain.
    pub fn from_config(cfg: &nook_core::ConnectionConfig) -> Self {
        let ssh_password_stored = nook_engine::profiles::keychain_available()
            && nook_engine::profiles::keychain_lookup(&nook_engine::profiles::ssh_account(&cfg.id))
                .ok()
                .flatten()
                .is_some();
        ConnectionForm {
            step: ConnStep::General,
            name: cfg.name.clone(),
            kind: cfg.kind,
            host: cfg.host.clone(),
            port: cfg.port,
            user: cfg.user.clone(),
            password: String::new(),
            database: cfg.database.clone(),
            file: cfg.file.clone(),
            group: cfg.group.clone(),
            ssl: cfg.ssl,
            color: cfg.color.clone(),
            timeout: cfg.connect_timeout_s.min(300) as u16,
            read_only: cfg.read_only,
            keychain: nook_engine::profiles::keychain_available(),
            ssh_host: cfg.ssh.host.clone(),
            ssh_port: cfg.ssh.port,
            ssh_user: cfg.ssh.user.clone(),
            ssh_key: cfg.ssh.key_file.clone(),
            ssh_auth: ssh_auth_of(cfg.ssh.key_file.as_str(), ssh_password_stored),
            ssh_password: String::new(),
            ssh_save_password: cfg.ssh.enabled && ssh_password_stored,
            ssh_local_port: cfg.ssh.local_port,
        }
    }

    /// The design's one-of-three, derived from the fields the profile carries: a key file means Key
    /// file, a stored password means Password, and neither means the agent. The user's own choice wins
    /// once it disagrees with both, which is how the segmented control resolves the case where a key
    /// file and a stored password are set together.
    pub fn ssh_auth(&self) -> SshAuth {
        if self.ssh_auth != SshAuth::Agent {
            self.ssh_auth
        } else {
            ssh_auth_of(&self.ssh_key, self.ssh_save_password)
        }
    }

    /// `setSshTunnel`'s `enabled`: the design's Tunnel step has no toggle of its own, so a host is what
    /// says the tunnel is on.
    pub fn ssh_enabled(&self) -> bool {
        !self.ssh_host.trim().is_empty()
    }

    /// The one sentence the `.why` line and Save's disabled state both read. It is the engine's own
    /// refusal for the same input, so the form cannot say "complete" while `conn.new` would refuse.
    pub fn blocker(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("a connection needs a name".into());
        }
        if self.kind.is_file_based() && self.file.trim().is_empty() {
            return Some("a SQLite connection needs \"file\"".into());
        }
        None
    }
}

/// The design's auth mode from the two profile fields, without the user's own choice.
pub fn ssh_auth_of(key_file: &str, password_stored: bool) -> SshAuth {
    if !key_file.trim().is_empty() {
        SshAuth::KeyFile
    } else if password_stored {
        SshAuth::Password
    } else {
        SshAuth::Agent
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

/// The AI band's working copy, seeded from the session when the band opens and written back through
/// the command registry when a field changes. It is the *only* place a half-typed address lives, so a
/// frame that is merely painted never touches the engine.
#[derive(Debug, Clone, Default)]
pub struct AiForm {
    pub prompt: String,
    pub provider: nook_ai::Provider,
    /// Empty means the provider's own default, the engine's rule.
    pub model: String,
    pub base_url: String,
    /// The key the field holds. Never written to the store; the moment it is committed it is handed
    /// to `ai.setKey` and the keychain owns it.
    pub key: String,
    /// Whether the keychain already holds a key for the chosen provider.
    pub key_set: bool,
    /// The key field holds something the keychain has not been given yet. Committed when the field
    /// loses focus or when a command is about to run — never per keystroke, which would store six
    /// half-keys, each one a `security` process.
    pub key_dirty: bool,
    /// Set when the key field loses focus, so the key is committed on the way out.
    pub commit_key: bool,
    pub forget_key: bool,
    /// Set by any control that changed a saved setting, so the band writes it once.
    pub dirty: bool,
    /// What `ai.tables` offered for the active connection: name and column count, only as many as
    /// the column paints.
    pub mentions: Vec<(String, usize)>,
    /// How many the engine holds in total, so `+N more` counts the catalog and not this window of it.
    pub mentions_total: usize,
    /// The query the current `mentions` window was fetched with. `Some` — including `Some("")` for a
    /// bare `@`, which lists the catalog — is the live-filter mode; `None` is the fallback top-N.
    pub mention_filter: Option<String>,
}

impl AiForm {
    /// The model a request would be sent with: the typed one, or the provider's own default. Empty for
    /// a provider that has neither (vLLM), which is the one case a form must refuse to generate.
    pub fn effective_model(&self) -> String {
        if self.model.trim().is_empty() {
            self.provider.default_model().to_string()
        } else {
            self.model.trim().to_string()
        }
    }

    /// Whether a request can name a model at all. The engine sends `model()` verbatim, so an empty
    /// answer is a provider the user must name a model for before Generate means anything.
    pub fn has_model(&self) -> bool {
        !self.effective_model().is_empty()
    }
}

/// One top-bar menu: the design keeps six of them, each opening the command groups the registry
/// already publishes. Returns the id of the spec that was picked — the click is answered by the
/// caller, after this function's `&mut UiState` is gone, because running the command needs
/// `&mut Session` at the same time.
fn menu_button(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    menu: &str,
    groups: &[&str],
) -> Option<&'static str> {
    let mut picked = None;
    // The design's `.menu` is a bare label: no fill, no border, 28px tall and 6px of padding, with a
    // fill only while the pointer is on it. egui's default button paints chrome in every state, so
    // the visuals are scoped here rather than faked with an explicit fill that would kill the hover.
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;
        widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        widgets.inactive.bg_stroke = Stroke::NONE;
        widgets.hovered.weak_bg_fill = tokens.hover;
        widgets.hovered.bg_stroke = Stroke::NONE;
        widgets.active.weak_bg_fill = tokens.panel_high;
        widgets.active.bg_stroke = Stroke::NONE;
        ui.style_mut().spacing.button_padding = Vec2::new(f32::from(t::S3), 0.0);
        let button = egui::Button::new(RichText::new(menu).size(t::T_SM).color(tokens.text_dim))
            .corner_radius(CornerRadius::same(t::R_CTL))
            .min_size(Vec2::new(0.0, t::MENU_H));
        egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
            ui.set_min_width(280.0);
            for group in groups {
                let specs: Vec<&'static nook_engine::CommandSpec> = nook_engine::registry()
                    .iter()
                    .filter(|c| c.group == *group)
                    .collect();
                if specs.is_empty() {
                    continue;
                }
                ui.menu_button(RichText::new(*group).size(t::T_SM), |ui| {
                    ui.set_min_width(340.0);
                    for spec in specs {
                        let response =
                            ui.button(RichText::new(spec.label).size(t::T_SM).color(tokens.text));
                        if response.clicked() {
                            picked = Some(spec.id);
                            ui.close();
                        }
                        if !spec.help.is_empty() {
                            response.on_hover_text(spec.help);
                        }
                    }
                });
            }
        });
    });
    picked
}

/// How many rows the inspector's Preview tab asks for: enough to recognise the data, few enough
/// that a wide table on a slow link does not stall the panel.
const PREVIEW_ROWS: usize = 50;

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
    /// Which connection the form was built from, so an edit dialog fills it once per open instead of
    /// every frame (a frame's typing would be overwritten).
    pub connection_form_id: Option<Option<usize>>,
    /// The pinned `conn.test` row, kept across frames.
    pub test_state: Option<TestOutcome>,
    /// The destructive action waiting for confirmation, if any.
    pub confirm: Option<Confirm>,
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
    /// Which palette is open: commands (Cmd K) or objects (Cmd P). `>` and `#` prefixes switch
    /// mode from the field itself.
    pub palette_objects: bool,
    pub diagram_offset: egui::Vec2,
    pub diagram_zoom: f32,
    pub monitor_filter: String,
    pub active_tab_override: Option<usize>,
    pub want_demo: bool,
    pub apply_after_edit: bool,
    pub editor_dirty: bool,
    /// The results register of each query tab, keyed by tab index: the view of its last run. A
    /// closed tab's entry goes with it, and a run that lands rebuilds the entry around its own
    /// statements.
    pub registers: std::collections::BTreeMap<usize, crate::register::Register>,
    /// Where the caret sits in the active query tab's script, for the editor's and the status bar's
    /// `Ln x, Col y`. `None` until the editor has painted a caret.
    pub caret: Option<crate::editor::Caret>,
    /// The object inspector: what the tree has selected and what it has fetched about it.
    pub inspector: crate::inspector::Inspector,
    /// Which objects the tree lists: the design's All/Connections/Tables/Views control.
    pub tree_scope: crate::tree::Scope,
    /// Log drawer: collapsed by default so the data gets the room.
    pub log_open: bool,
    pub control: Option<Control>,
    pub shot: Option<String>,
    shot_requested: bool,
    /// True when the process was started with `--shot`: capture once and quit.
    shot_quit: bool,
    pub toast: Option<(String, f64)>,
    pub quit: bool,
    /// The AI band inside the query tab: open or closed, and the form it is holding.
    pub ai_band: bool,
    pub ai_form: AiForm,
    /// The in-flight `ai.generate` draft: one request at a time, and the tab index the draft
    /// opened when it landed, so the caller can switch to it.
    pub ai_request: Option<crate::ai::AiRequest>,
    /// The `ai.providers` answer the Preferences dialog paints, cached because resolving it costs one
    /// `security` process per provider. `None` means "read it again": set when the dialog opens and
    /// after a provider is picked.
    pub ai_providers: Option<serde_json::Value>,
    /// Form view on the data tab: the selected record as a vertical form. A flag on the UI state,
    /// not the engine's — the engine command is stateless and reads whatever row it is handed.
    pub form_view: bool,
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
            connection_form_id: None,
            test_state: None,
            confirm: None,
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
            palette_objects: false,
            diagram_offset: egui::vec2(0.0, 0.0),
            diagram_zoom: 1.0,
            monitor_filter: String::new(),
            active_tab_override: None,
            want_demo: false,
            apply_after_edit: false,
            editor_dirty: false,
            registers: std::collections::BTreeMap::new(),
            caret: None,
            inspector: crate::inspector::Inspector::default(),
            tree_scope: crate::tree::Scope::default(),
            log_open: false,
            control: None,
            shot: None,
            shot_requested: false,
            shot_quit: false,
            toast: None,
            quit: false,
            ai_band: false,
            ai_form: AiForm::default(),
            ai_providers: None,
            ai_request: None,
            form_view: false,
        }
    }
}

impl UiState {
    /// Inline-editor state for one tab.
    pub fn table(&mut self, index: usize) -> &mut TableState {
        self.tables.entry(index).or_default()
    }

    /// A closed tab takes its results register with it, and the registers of the tabs that moved down
    /// follow — otherwise the next tab along would wear the closed one's view of its run.
    pub fn forget_tab(&mut self, index: usize) {
        let mut kept = std::collections::BTreeMap::new();
        for (key, register) in std::mem::take(&mut self.registers) {
            match key.cmp(&index) {
                std::cmp::Ordering::Less => {
                    kept.insert(key, register);
                }
                std::cmp::Ordering::Equal => {}
                std::cmp::Ordering::Greater => {
                    kept.insert(key - 1, register);
                }
            }
        }
        self.registers = kept;
    }

    pub fn open_connection_dialog(&mut self, index: Option<usize>) {
        self.connection_open = true;
        self.connection_editing = index;
        // Force the next paint to rebuild the form from the connection it is editing: an edit dialog
        // that kept the previous form would save the wrong profile's fields.
        self.connection_form_id = None;
        self.test_state = None;
        if index.is_none() {
            self.connection_form = ConnectionForm::default();
        }
    }

    /// `id` for a plain command; `id&key=value` prefills those keys.
    ///
    /// The string form is meant for a caller that writes the id out by hand. A caller that already
    /// holds the values — a table named `R&D`, SQLite's file path as the connection id — must use
    /// [`UiState::open_command_with`]: `&` and `=` are the delimiters here.
    pub fn open_command(&mut self, spec: String) {
        let (id, prefill) = match spec.split_once('&') {
            Some((id, rest)) => (id.to_string(), rest.to_string()),
            None => (spec, String::new()),
        };
        let pairs: Vec<(String, String)> = prefill
            .split('&')
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                // An empty value is not a preference, it is a missing one: `table=` used to overwrite
                // the command's own example with nothing, so the dialog opened prefilled with a
                // parameter that could only fail.
                (!key.is_empty() && !value.is_empty()).then(|| (key.to_string(), value.to_string()))
            })
            .collect();
        self.open_command_prefilled(&id, &pairs);
    }

    /// [`UiState::open_command`] for a caller that has the parameters as data rather than as text.
    pub fn open_command_with(&mut self, id: &str, prefill: &[(&str, &str)]) {
        let pairs: Vec<(String, String)> = prefill
            .iter()
            .filter(|(key, value)| !key.is_empty() && !value.is_empty())
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        self.open_command_prefilled(id, &pairs);
    }

    fn open_command_prefilled(&mut self, id: &str, prefill: &[(String, String)]) {
        // Preferences is a form, not a JSON runner: the top bar has no Settings button any more
        // (the design does not), so its one entry in the File menu opens the dialog.
        if id == "app.settings" {
            self.settings_open = true;
            // The dialog's AI card reads the provider table once per open rather than once per frame.
            self.ai_providers = None;
            return;
        }
        self.command_id = id.to_string();
        let base = nook_engine::find(id)
            .map(|s| s.params.to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "{}".into());
        let mut params: serde_json::Map<String, Value> =
            serde_json::from_str(&base).unwrap_or_default();
        for (key, value) in prefill {
            params.insert(key.clone(), Value::String(value.clone()));
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

pub struct NookApp {
    pub session: Session,
    pub state: UiState,
    last_job_tick: std::time::Instant,
    /// Next wall-clock moment the tab snapshot may be written (throttled to 2 s).
    tab_persist_at: Option<std::time::Instant>,
    /// The snapshot string as of the last write, so an unchanged session writes nothing.
    last_tab_snapshot: Option<String>,
    options: Options,
}

impl NookApp {
    pub fn new(cc: &eframe::CreationContext, options: Options) -> Self {
        let mut session = if options.demo {
            crate::demo_session()
        } else {
            let mut session = Session::with_profiles();
            // Phase 1: a start rebuilds the tabs the last run left open. Before any UI paints, so
            // the first frame already shows the restored session.
            session.restore_tabs();
            session
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
            match nook_core::ConnectionConfig::parse(uri) {
                Ok(cfg) => {
                    let id = cfg.id.clone();
                    if session.conn_index(&id).is_err() {
                        session.conns.push(Conn::new(cfg));
                    }
                    match session.conn_index(&id) {
                        Ok(index) => {
                            if let Err(e) = nook_engine::cmd::open_now(&mut session, index) {
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
        NookApp {
            session,
            state,
            last_job_tick: std::time::Instant::now(),
            tab_persist_at: None,
            last_tab_snapshot: None,
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
                "ui.selectResult" => {
                    NookApp::select_result(&self.session, &mut self.state, &request.params)
                }
                "ui.setResultView" => {
                    NookApp::set_result_view(&self.session, &mut self.state, &request.params)
                }
                "ui.openConnection" => {
                    // `conn` names the profile to edit by id; omitted or null opens a new one. The
                    // dialog is opened through the same path the tree uses, so the form is rebuilt
                    // from the profile exactly once.
                    let target = match request.params.get("conn") {
                        None | Some(Value::Null) => Ok(None),
                        Some(value) => value
                            .as_str()
                            .ok_or_else(|| "conn must be a connection id".to_string())
                            .and_then(|id| {
                                self.session.conn_index(id).map(Some).map_err(|_| {
                                    format!(
                                        "no such connection '{id}'; try {}",
                                        self.connection_ids()
                                    )
                                })
                            }),
                    };
                    match target {
                        Err(error) => serde_json::json!({"ok": false, "error": error}),
                        Ok(index) => {
                            self.state.open_connection_dialog(index);
                            ctx.request_repaint();
                            serde_json::json!({"ok": true, "editing": index})
                        }
                    }
                }
                "ui.setConnectionTab" => {
                    let tab = request
                        .params
                        .get("tab")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default();
                    match ConnStep::parse(tab) {
                        None => serde_json::json!({
                            "ok": false,
                            "error": format!("unknown tab '{tab}'; try General, TLS, Tunnel, Advanced"),
                        }),
                        Some(_) if !self.state.connection_open => serde_json::json!({
                            "ok": false,
                            "error": "no connection dialog is open",
                        }),
                        Some(step) => {
                            self.state.connection_form.step = step;
                            ctx.request_repaint();
                            serde_json::json!({"ok": true, "tab": step.label()})
                        }
                    }
                }
                "ui.setConnectionField" => {
                    NookApp::set_connection_field(&mut self.state, &request.params)
                }
                "ui.selectTable" => {
                    let table = request
                        .params
                        .get("table")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let schema = request
                        .params
                        .get("schema")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_owned());
                    // The connection is resolved once. `close_conn` leaves the catalog behind, so
                    // "this connection can name the table" is a weaker question than "it can be
                    // described"; a connection that is gone or closed is refused before the tab or the
                    // selection is touched, and the panel is not left claiming a success it cannot
                    // show.
                    match NookApp::inspector_target(&self.session, request.params.get("conn")) {
                        Err(error) => serde_json::json!({"ok": false, "error": error}),
                        Ok(conn_index) => {
                            let known = match self.session.conns[conn_index].catalog.as_ref() {
                                None => true,
                                Some(catalog) => {
                                    catalog.find_table(schema.as_deref(), &table).is_some()
                                }
                            };
                            if table.is_empty() {
                                serde_json::json!({"ok": false, "error": "a table is required"})
                            } else if !known {
                                serde_json::json!({"ok": false, "error": "no such object"})
                            } else {
                                let tab = match request.params.get("tab").and_then(|t| t.as_str()) {
                                    None => Ok(self.state.inspector.tab),
                                    Some(name) => {
                                        let wanted = name.to_ascii_lowercase();
                                        crate::inspector::Tab::ALL
                                            .into_iter()
                                            .find(|candidate| {
                                                candidate.label().to_ascii_lowercase() == wanted
                                            })
                                            .ok_or_else(|| {
                                                let names = crate::inspector::Tab::ALL
                                                    .map(|tab| tab.label().to_ascii_lowercase())
                                                    .join(", ");
                                                format!("unknown tab '{name}'; try {names}")
                                            })
                                    }
                                };
                                match tab {
                                    Err(error) => {
                                        serde_json::json!({"ok": false, "error": error})
                                    }
                                    Ok(tab) => {
                                        self.state.inspector.tab = tab;
                                        self.select_table(
                                            conn_index,
                                            TableRef {
                                                schema,
                                                table: table.clone(),
                                            },
                                        );
                                        self.refresh_inspector();
                                        serde_json::json!({
                                            "ok": true,
                                            "selected": table,
                                            "tab": self.state.inspector.tab.label(),
                                        })
                                    }
                                }
                            }
                        }
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
                    // Delegate to the command the preferences dialog uses, so the control channel
                    // cannot accept a value the product refuses (and vice versa).
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
                "ui.setAiBand" => {
                    let open = request
                        .params
                        .get("open")
                        .and_then(|o| o.as_bool())
                        .unwrap_or(true);
                    self.state.ai_band = open;
                    if open {
                        crate::ai::seed(&mut self.session, &mut self.state.ai_form);
                    }
                    ctx.request_repaint();
                    serde_json::json!({
                        "ok": true,
                        "open": self.state.ai_band,
                        "provider": self.state.ai_form.provider.as_str(),
                        "base_url": self.state.ai_form.base_url,
                        "key_set": self.state.ai_form.key_set,
                        "mentions": self.state.ai_form.mentions.len(),
                    })
                }
                "ui.messages" => serde_json::json!({
                    "messages": self.session.messages.iter().rev().take(50).map(|m| serde_json::json!({
                        "level": format!("{:?}", m.level).to_lowercase(), "text": m.text,
                    })).collect::<Vec<_>>()
                }),
                "ui.commands" => serde_json::json!({
                    "count": nook_engine::registry().len(),
                    "ids": nook_engine::registry().iter().map(|c| c.id).collect::<Vec<_>>(),
                }),
                "ui.quickOpen" => {
                    // Open the palette in a chosen mode with a typed query — scriptable, so the
                    // overlay can be driven and screenshot-verified.
                    let mode = request
                        .params
                        .get("mode")
                        .and_then(|m| m.as_str())
                        .unwrap_or("objects");
                    let query = request
                        .params
                        .get("query")
                        .and_then(|q| q.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.state.palette_objects = mode != "commands";
                    self.state.palette_query = query;
                    self.state.palette_open = true;
                    ctx.request_repaint();
                    serde_json::json!({
                        "ok": true,
                        "objects": self.state.palette_objects,
                        "query": self.state.palette_query,
                    })
                }
                "ui.shot" => {
                    let path = request
                        .params
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("/tmp/nook.png")
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
    /// Top bar, in the design's order: brand, workspace, menus, palette field and keycap, then the
    /// state cluster (pending, control channel, density, theme) pinned to the right edge.
    fn app_bar(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let band = t::band_h(ui, tokens, APP_BAR_H, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = f32::from(t::S4);

                let (mark, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                ui.painter()
                    .rect_filled(mark, CornerRadius::same(t::R_BOX), tokens.accent);
                ui.painter().text(
                    mark.center(),
                    Align2::CENTER_CENTER,
                    "DC",
                    tokens.font(t::T_XS),
                    tokens.accent_text,
                );
                ui.label(RichText::new("Nook").size(t::T_MD).color(tokens.text));

                self.workspace_button(ui, tokens);
                t::divider(ui, tokens);

                let mut pick = None;
                ui.scope(|ui| {
                    // The design's `.menus` is one flex block, so the six labels sit flush.
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (menu, groups) in MENUS {
                        if let Some(id) = menu_button(ui, tokens, menu, groups) {
                            pick = Some(id);
                        }
                    }
                });
                // Answered out here, where `&mut UiState` is released: the pick runs the command,
                // and that needs `&mut Session` too. One menu can be picked per frame.
                if let Some(id) = pick {
                    Self::menu_pick(&mut self.session, &mut self.state, id);
                }

                // What the right cluster will need, measured before it is painted: the design's
                // `.palette` is `flex: 1 1 auto; min-width: 120px; max-width: 300px`, and this is
                // the same bargain — the field takes what the keycap and the cluster leave.
                let theme = self.session.settings.theme.clone();
                let theme_label = if theme == "dark" { "Light" } else { "Dark" };
                let density = self.session.settings.density;
                let density_options = [
                    ("Compact", density == Density::Compact),
                    ("Comfortable", density == Density::Comfortable),
                ];
                let agent = match &self.state.control {
                    Some(control) => format!("control channel :{}", control.port),
                    None => "agent idle".to_string(),
                };
                let pending: usize = self
                    .session
                    .tabs
                    .iter()
                    .map(|tab| tab.grid.pending.len())
                    .sum();
                let pending_text = format!("{pending} pending");
                let reserve = t::icon_button_width(ui, tokens, theme_label, true)
                    + t::segmented_width(ui, tokens, &["Compact", "Comfortable"])
                    + t::chip_width(ui, tokens, &agent)
                    + t::chip_width(ui, tokens, &pending_text)
                    + t::keycap_width(ui, tokens, "Cmd K")
                    + 5.0 * f32::from(t::S4);
                let field_width = (ui.available_width() - reserve).clamp(120.0, 300.0);

                // The palette is a real field: clicking it opens the overlay, which then owns the
                // focus, so the bar reads like a search box and behaves like one.
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.state.palette_query)
                        .hint_text(
                            RichText::new("Search or run a command")
                                .size(t::T_SM)
                                .color(tokens.text_faint),
                        )
                        .font(tokens.font(t::T_SM))
                        .text_color(tokens.text)
                        .desired_width(field_width)
                        // The design's field is 26 tall: a 12px galley, 1px of frame on each side and
                        // 6 above and below to make up the rest, with the magnifier inside.
                        .margin(egui::Margin {
                            left: t::S4 + t::GLYPH_SM as i8 + t::S3,
                            right: t::S4,
                            top: 6,
                            bottom: 6,
                        }),
                );
                if field.gained_focus() || field.clicked() {
                    self.state.palette_open = true;
                }
                let field_rect = field.rect;
                field.on_hover_text("Command palette (Cmd K)");
                icons::glyph(
                    ui,
                    egui::Rect::from_min_size(
                        egui::pos2(
                            field_rect.left() + f32::from(t::S4) + 1.0,
                            field_rect.center().y - t::GLYPH_SM / 2.0,
                        ),
                        Vec2::splat(t::GLYPH_SM),
                    ),
                    Glyph::Search,
                    tokens.text_faint,
                );
                t::keycap(ui, tokens, "Cmd K");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let next = if theme == "dark" { "light" } else { "dark" };
                    if t::icon_button(ui, tokens, theme_label, Some(Glyph::Sun), true)
                        .on_hover_text(format!("Switch to the {next} theme"))
                        .clicked()
                    {
                        let _ = self.session.run(
                            "app.setSetting",
                            serde_json::json!({"key": "theme", "value": next}),
                        );
                    }
                    // The design's density switch, between the agent chip and the theme toggle.
                    if let Some(index) = t::segmented(ui, tokens, &density_options) {
                        let _ = self.session.run(
                            "app.setSetting",
                            serde_json::json!({
                                "key": "density",
                                "value": if index == 0 { "compact" } else { "comfortable" },
                            }),
                        );
                    }
                    // The control channel is the agent surface, so it reports here rather than twice:
                    // the design's `.agent` is a border only, and it turns `accent_line` while running.
                    let (ink, dot, border) = match &self.state.control {
                        Some(_) => (tokens.text, tokens.info, tokens.accent_line),
                        None => (tokens.text_dim, tokens.text_faint, tokens.stroke),
                    };
                    t::state_chip(ui, tokens, &agent, None, Some(border), ink, dot)
                        .on_hover_text("Headless callers talk to the app over this channel");

                    // `.chip` when there is something to apply (filled, no border), `.chip.zero`
                    // otherwise (surface-2 inside a border).
                    if pending > 0 {
                        t::state_chip(
                            ui,
                            tokens,
                            &pending_text,
                            Some(tokens.dirty),
                            None,
                            tokens.dirty_text,
                            tokens.warn,
                        );
                    } else {
                        t::state_chip(
                            ui,
                            tokens,
                            &pending_text,
                            Some(tokens.panel_alt),
                            Some(tokens.stroke),
                            tokens.text_dim,
                            tokens.text_faint,
                        );
                    }
                });
            });
        });
        t::band_hairline(ui, tokens, band);
    }

    /// The design's `.ws`: the active workspace with its engine tag and the chevron that opens the
    /// list of the others. Switching a workspace is switching the connection the app is talking to.
    fn workspace_button(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let font = tokens.font(t::T_SM);
        let (name, engine) = match self
            .session
            .active_conn
            .and_then(|index| self.session.conns.get(index))
        {
            Some(conn) => (
                conn.cfg.name.clone(),
                Some(conn.cfg.kind.label().to_uppercase()),
            ),
            None => ("No connection".to_string(), None),
        };
        let name_width = ui
            .painter()
            .layout_no_wrap(name.clone(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x;
        let engine_width = engine
            .as_ref()
            .map(|label| t::tag_width(ui, tokens, label) + f32::from(t::S3))
            .unwrap_or(0.0);
        let width = 1.0
            + f32::from(t::S4)
            + t::GLYPH_BOX
            + f32::from(t::S3)
            + name_width
            + engine_width
            + t::GLYPH_SM
            + f32::from(t::S3)
            + f32::from(t::S3)
            + 1.0;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, t::ICON_BUTTON_H), Sense::click());
        let radius = CornerRadius::same(t::R_CTL);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, tokens.panel_alt);
        if response.hovered() {
            painter.rect_filled(rect, radius, tokens.hover);
        }
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, tokens.stroke),
            egui::StrokeKind::Inside,
        );
        let mut x = rect.left() + f32::from(t::S4) + 1.0;
        icons::glyph(
            ui,
            egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - t::GLYPH_BOX / 2.0),
                Vec2::splat(t::GLYPH_BOX),
            ),
            Glyph::Database,
            tokens.text_dim,
        );
        x += t::GLYPH_BOX + f32::from(t::S3);
        painter.text(
            egui::pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            &name,
            font,
            tokens.text,
        );
        x += name_width;
        if let Some(engine) = engine {
            x += f32::from(t::S3);
            let tag = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - t::TAG_H / 2.0),
                Vec2::new(engine_width - f32::from(t::S3), t::TAG_H),
            );
            t::paint_tag(ui, tokens, tag, &engine);
            x += engine_width - f32::from(t::S3);
        }
        icons::glyph(
            ui,
            egui::Rect::from_min_size(
                egui::pos2(x + f32::from(t::S3), rect.center().y - t::GLYPH_SM / 2.0),
                Vec2::splat(t::GLYPH_SM),
            ),
            Glyph::Chevron,
            tokens.text_dim,
        );

        // Which connection the app talks to is the app's own verb, so the list is the real one.
        let open = Self::workspace_rows(&self.session);
        let mut select: Option<String> = None;
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(200.0);
            if open.is_empty() {
                ui.label(
                    RichText::new("No connections yet")
                        .size(t::T_SM)
                        .color(tokens.text_dim),
                );
            }
            for (id, name, active) in &open {
                let label = if *active {
                    format!("· {name}")
                } else {
                    name.clone()
                };
                if ui
                    .button(RichText::new(label).size(t::T_SM).color(tokens.text))
                    .clicked()
                {
                    select = Some(id.clone());
                    ui.close();
                }
            }
        });
        if let Some(id) = select {
            if let Err(e) = self
                .session
                .run("conn.select", serde_json::json!({ "conn": id }))
            {
                self.session.log(Level::Warn, e.to_string());
            }
        }
    }

    /// One row of the workspace popup: the connection's **id** — the thing `conn.select` resolves —
    /// its name and whether it is the one the app is talking to.
    fn workspace_rows(session: &Session) -> Vec<(String, String, bool)> {
        session
            .conns
            .iter()
            .enumerate()
            .map(|(index, conn)| {
                (
                    conn.cfg.id.clone(),
                    conn.cfg.name.clone(),
                    session.active_conn == Some(index),
                )
            })
            .collect()
    }

    /// Activity rail: the five workspaces the design shows, each wired to a destination the app
    /// already has. Reports is the exception: a report is a chart built from a query result, so that
    /// button selects an existing report tab instead of opening an empty chart.
    fn rail(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let open_conn = self.session.active_conn.filter(|index| {
            self.session
                .conns
                .get(*index)
                .is_some_and(|conn| conn.is_open())
        });
        let scheduled = self
            .session
            .jobs
            .jobs
            .iter()
            .filter(|job| job.enabled)
            .count();
        let active = self
            .session
            .active_tab
            .and_then(|index| self.session.tabs.get(index))
            .map(|tab| tab.kind);
        // The welcome view is what both "no tab" and a Welcome tab put on screen, so the rail has to
        // call both of them the Connections workspace or the lit button disagrees with the view.
        let on_welcome = active.is_none() || active == Some(TabKind::Welcome);
        let report_tab = self
            .session
            .tabs
            .iter()
            .position(|tab| tab.kind == TabKind::Report);

        let entries = [
            (Glyph::Database, "Connections", true, on_welcome, 0),
            (
                Glyph::Queries,
                "New query",
                open_conn.is_some(),
                active == Some(TabKind::Query),
                0,
            ),
            (
                Glyph::Jobs,
                "Scheduled jobs",
                true,
                active == Some(TabKind::Jobs),
                scheduled,
            ),
            (
                Glyph::Reports,
                "Reports — build a chart from a query",
                report_tab.is_some(),
                active == Some(TabKind::Report),
                0,
            ),
            (
                Glyph::Monitor,
                "Monitor — needs an open connection",
                open_conn.is_some(),
                active == Some(TabKind::Monitor),
                0,
            ),
        ];

        // The indicator hangs on the rail's own left edge, which is where the design puts it: the
        // button is centred in the 44px rail, so the button's -6px offset is the rail's x.
        let rail_left = ui.max_rect().left();
        let mut clicked: Option<usize> = None;
        ui.add_space(f32::from(t::S4));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = f32::from(t::S2);
            for (index, (glyph, tip, enabled, active, badge)) in entries.into_iter().enumerate() {
                let sense = if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(t::RAIL_BUTTON), sense);
                let hovered = enabled && response.hovered();
                let painter = ui.painter();
                if active {
                    painter.rect_filled(rect, CornerRadius::same(t::R_CTL), tokens.accent_dim);
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rail_left, rect.center().y - 10.0),
                            Vec2::new(t::INDICATOR_W, 20.0),
                        ),
                        CornerRadius::same(1),
                        tokens.accent,
                    );
                } else if hovered {
                    painter.rect_filled(rect, CornerRadius::same(t::R_CTL), tokens.hover);
                }
                let ink = if !enabled {
                    tokens.text_faint
                } else if active {
                    tokens.accent
                } else if hovered {
                    tokens.text
                } else {
                    tokens.text_dim
                };
                icons::glyph(
                    ui,
                    egui::Rect::from_center_size(rect.center(), Vec2::splat(t::GLYPH_BOX)),
                    glyph,
                    ink,
                );
                if badge > 0 {
                    rail_badge(ui, tokens, rect, badge);
                }
                if response.clicked() {
                    clicked = Some(index);
                }
                response.on_hover_text(tip);
            }
        });
        if let Some(index) = clicked {
            self.rail_action(index, open_conn, report_tab);
        }
    }

    /// What each rail button does, kept apart so the rail's painting stays a function of state.
    fn rail_action(&mut self, index: usize, open_conn: Option<usize>, report_tab: Option<usize>) {
        match index {
            0 => {
                self.session.active_tab = self
                    .session
                    .tabs
                    .iter()
                    .position(|tab| tab.kind == TabKind::Welcome);
            }
            1 => {
                if let Some(conn) = open_conn {
                    let tab = self.session.add_tab(TabKind::Query, "Query", Some(conn));
                    self.state.active_tab_override = Some(tab);
                }
            }
            2 => {
                let tab = self.session.add_tab(TabKind::Jobs, "Jobs", None);
                self.state.active_tab_override = Some(tab);
            }
            3 => {
                if let Some(tab) = report_tab {
                    self.state.active_tab_override = Some(tab);
                }
            }
            4 => {
                if let Some(conn) = open_conn {
                    let tab = self
                        .session
                        .add_tab(TabKind::Monitor, "Monitor", Some(conn));
                    self.state
                        .request_monitor(&mut self.session, conn, "dashboard");
                    self.state.active_tab_override = Some(tab);
                }
            }
            _ => {}
        }
    }

    /// Tab strip: painted tabs, accent underline on the active one, close on hover.
    fn tab_strip(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let mut select: Option<usize> = self.state.active_tab_override.take();
        let mut close: Option<usize> = None;

        // The design's tab bar is one pixel taller than its tabs, so the active tab's bottom edge
        // covers the bar's border: that row is painted first and the tabs paint over it.
        t::band_fill(ui, TAB_BAR_H, tokens.panel_alt, 0, egui::Align::Min, |ui| {
            // No inset: the design's border spans the bar and only the tabs are indented, so the
            // hairline reaches both window edges like the bars above and below it.
            let band = ui.max_rect();
            t::band_hairline(ui, tokens, band);
            let strip = egui::Rect::from_min_max(
                egui::pos2(band.left() + f32::from(t::S4), band.top()),
                egui::pos2(band.right() - t::TAB_CTL_W, band.bottom()),
            );
            let mut strip_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(strip)
                    .layout(egui::Layout::left_to_right(egui::Align::Min)),
            );
            egui::ScrollArea::horizontal()
                .id_salt("tab-strip")
                .show(&mut strip_ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (index, tab) in self.session.tabs.iter().enumerate() {
                            let active = self.session.active_tab == Some(index);
                            let dirty = tab.grid.dirty();
                            let pending = tab.grid.pending.len();
                            let text_width = ui
                                .painter()
                                .layout_no_wrap(
                                    tab.title.clone(),
                                    tokens.font(t::T_SM),
                                    Color32::PLACEHOLDER,
                                )
                                .size()
                                .x;
                            let width = (text_width
                                + if dirty || pending > 0 { 74.0 } else { 52.0 })
                            .clamp(t::TAB_MIN_W, t::TAB_MAX_W);
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

                            let mut x = rect.left() + f32::from(t::S4);
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
                                tokens.font(t::T_SM),
                                if active { tokens.text } else { tokens.text_dim },
                            );

                            // Close cross: always on the active tab, on hover for the rest.
                            if active || hovered {
                                // `.tclose` is 24px with `margin-right:-4`, so its right edge sits 4px
                                // inside the tab's 8px padding.
                                let close_rect = egui::Rect::from_center_size(
                                    egui::pos2(
                                        rect.right() - 4.0 - t::TAB_CLOSE / 2.0,
                                        rect.center().y,
                                    ),
                                    Vec2::splat(t::TAB_CLOSE),
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
                                    tokens.font(t::T_LG),
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

                            // `.tab` carries a full-height border-right, not an inset divider.
                            ui.painter().vline(
                                rect.right(),
                                rect.top()..=rect.bottom(),
                                Stroke::new(1.0, tokens.stroke),
                            );
                            if response.clicked() {
                                select = Some(index);
                            }
                            if response.middle_clicked() {
                                close = Some(index);
                            }
                        }
                    });
                });

            // `.tabctl`: pinned to the bar's right edge rather than trailing the tabs, so it
            // stays put when the strip scrolls and when a tab is added.
            let ctl = egui::Rect::from_min_max(
                egui::pos2(band.right() - t::TAB_CTL_W, band.top()),
                band.max,
            );
            ui.painter().vline(
                ctl.left() + 0.5,
                ctl.top()..=ctl.bottom(),
                Stroke::new(1.0, tokens.stroke),
            );
            let button = egui::Rect::from_center_size(
                egui::pos2(
                    ctl.left() + 1.0 + f32::from(t::S4) + t::TAB_CLOSE / 2.0,
                    ctl.center().y,
                ),
                Vec2::splat(t::TAB_CLOSE),
            );
            let response = ui
                .interact(button, ui.id().with("tab-ctl-new"), Sense::click())
                .on_hover_text("New query tab");
            if response.hovered() {
                ui.painter()
                    .rect_filled(button, CornerRadius::same(t::R_CTL), tokens.hover);
            }
            ui.painter().text(
                button.center(),
                Align2::CENTER_CENTER,
                "+",
                tokens.font(t::T_LG),
                if response.hovered() {
                    tokens.text
                } else {
                    tokens.text_dim
                },
            );
            if response.clicked() {
                let index = self
                    .session
                    .add_tab(TabKind::Query, "Query", self.session.active_conn);
                select = Some(index);
            }
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
            } else {
                self.state.forget_tab(index);
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
                crate::editor::show(ui, &mut self.session, index, &mut self.state, tokens);
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
            nook_engine::parity::measure().covered.len(),
            nook_engine::parity::measure().total()
        );
        let band = t::band_h(ui, tokens, STATUS_H, |ui| {
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
                            nook_core::DriverKind::Sqlite => cfg.file.clone(),
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
                    // The design's rightmost fact: where the caret sits in the query tab's script.
                    let caret = self
                        .session
                        .active_tab
                        .filter(|index| self.session.tabs[*index].kind == TabKind::Query)
                        .and(self.state.caret);
                    if let Some(caret) = caret {
                        ui.label(
                            RichText::new(crate::editor::caret_note(caret))
                                .font(tokens.mono(t::T_XS))
                                .color(tokens.text_dim),
                        );
                    }
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
        t::band_top_hairline(ui, tokens, band);
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
                if let Err(e) = nook_engine::cmd::open_now(&mut self.session, index) {
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
            TreeAction::SelectTable(index, reference) => {
                self.select_table(index, reference);
            }
            TreeAction::OpenData(index, reference) => {
                tree::open_data_tab(&mut self.session, index, reference);
            }
            TreeAction::OpenDesign(index, reference) => {
                tree::open_design_tab(&mut self.session, index, reference);
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

    /// `ui.selectResult`: which of the active query tab's results the register shows. A tab that has
    /// never run builds the register `ui.setResultView` then moves; the refusal names the results
    /// that do exist, because the caller has no other way to learn them.
    fn select_result(session: &Session, state: &mut UiState, params: &Value) -> Value {
        let index = match NookApp::query_tab(session) {
            Ok(index) => index,
            Err(error) => return serde_json::json!({"ok": false, "error": error}),
        };
        let sql = &session.tabs[index].sql;
        let runs = sql.results.len();
        let wanted = params.get("index").and_then(Value::as_i64).unwrap_or(0);
        if wanted < 0 || wanted as usize >= runs {
            return serde_json::json!({
                "ok": false,
                "error": format!(
                    "no result {wanted}: the run holds {runs} ({})",
                    NookApp::result_range(runs)
                ),
            });
        }
        state
            .registers
            .entry(index)
            .or_insert_with(|| crate::register::Register::from(sql, None))
            .active = wanted as usize;
        serde_json::json!({"ok": true, "result": wanted, "results": runs})
    }

    /// `ui.setResultView`: show the active query tab's register as a grid or as JSON. The refusal
    /// lists the views the segment actually paints, so the two can never drift apart.
    fn set_result_view(session: &Session, state: &mut UiState, params: &Value) -> Value {
        let index = match NookApp::query_tab(session) {
            Ok(index) => index,
            Err(error) => return serde_json::json!({"ok": false, "error": error}),
        };
        let name = params
            .get("view")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(view) = crate::register::View::from_name(name) else {
            return serde_json::json!({
                "ok": false,
                "error": format!(
                    "unknown view {name:?}: valid views are {}",
                    crate::register::View::names()
                ),
            });
        };
        state
            .registers
            .entry(index)
            .or_insert_with(|| crate::register::Register::from(&session.tabs[index].sql, None))
            .view = view;
        serde_json::json!({"ok": true, "view": name, "views": crate::register::View::names()})
    }

    /// `ui.setConnectionField {field, value}`: the connection form's own fields by name, so a script
    /// can build a dialog frame. Every field the editor paints is named here, and the refusal lists
    /// them, which is what keeps the two in step.
    fn set_connection_field(state: &mut UiState, params: &Value) -> Value {
        if !state.connection_open {
            return serde_json::json!({"ok": false, "error": "no connection dialog is open"});
        }
        let Some(field) = params.get("field").and_then(Value::as_str) else {
            return serde_json::json!({
                "ok": false,
                "error": format!("a field name is required; try {}", CONNECTION_FIELDS.join(", ")),
            });
        };
        let value = params.get("value").cloned().unwrap_or(Value::Null);
        let text = value.as_str().unwrap_or_default().to_string();
        let flag = value.as_bool().unwrap_or(false);
        let number = value
            .as_u64()
            .filter(|n| *n <= u64::from(u16::MAX))
            .map(|n| n as u16);
        let form = &mut state.connection_form;
        let accepted = match field {
            "name" => {
                form.name = text;
                true
            }
            "kind" => match nook_core::DriverKind::parse(&text) {
                Some(kind) => {
                    form.kind = kind;
                    true
                }
                None => false,
            },
            "host" => {
                form.host = text;
                true
            }
            "port" => match number {
                Some(port) => {
                    form.port = port;
                    true
                }
                None => false,
            },
            "user" => {
                form.user = text;
                true
            }
            "password" => {
                form.password = text;
                true
            }
            "database" => {
                form.database = text;
                true
            }
            "file" => {
                form.file = text;
                true
            }
            "group" => {
                form.group = text;
                true
            }
            "ssl" => match nook_core::SslMode::parse(&text) {
                Some(mode) => {
                    form.ssl = mode;
                    true
                }
                None => false,
            },
            "read_only" => {
                form.read_only = flag;
                true
            }
            "keychain" => {
                form.keychain = flag;
                true
            }
            "timeout" => match number {
                Some(timeout) => {
                    form.timeout = timeout;
                    true
                }
                None => false,
            },
            "ssh_host" => {
                form.ssh_host = text;
                true
            }
            "ssh_port" => match number {
                Some(port) => {
                    form.ssh_port = port;
                    true
                }
                None => false,
            },
            "ssh_user" => {
                form.ssh_user = text;
                true
            }
            "ssh_key" => {
                form.ssh_key = text;
                true
            }
            "ssh_auth" => match text.to_ascii_lowercase().as_str() {
                "key file" | "keyfile" | "key" => {
                    form.ssh_auth = SshAuth::KeyFile;
                    true
                }
                "agent" => {
                    form.ssh_auth = SshAuth::Agent;
                    true
                }
                "password" => {
                    form.ssh_auth = SshAuth::Password;
                    true
                }
                _ => false,
            },
            "ssh_password" => {
                form.ssh_password = text;
                true
            }
            "ssh_save_password" => {
                form.ssh_save_password = flag;
                true
            }
            "ssh_local_port" => match number {
                Some(port) => {
                    form.ssh_local_port = port;
                    true
                }
                None => false,
            },
            "step" => match ConnStep::parse(&text) {
                Some(step) => {
                    form.step = step;
                    true
                }
                None => false,
            },
            other => {
                return serde_json::json!({
                    "ok": false,
                    "error": format!("unknown field '{other}'; try {}", CONNECTION_FIELDS.join(", ")),
                })
            }
        };
        if accepted {
            serde_json::json!({"ok": true, "field": field, "value": value})
        } else {
            serde_json::json!({
                "ok": false,
                "error": format!("field '{field}' did not accept {value}; try {}", CONNECTION_FIELDS.join(", ")),
            })
        }
    }

    /// The active tab, when it is a query tab: the two register methods act on the script on screen.
    fn query_tab(session: &Session) -> Result<usize, String> {
        match session.active_tab {
            None => Err("no active tab".into()),
            Some(index) if session.tabs[index].kind == TabKind::Query => Ok(index),
            Some(_) => Err("the active tab is not a query tab".into()),
        }
    }

    /// The result indices a run of `runs` holds, for a refusal that lists what is.
    fn result_range(runs: usize) -> String {
        match runs {
            0 => "none yet".into(),
            1 => "0".into(),
            _ => format!("0..{}", runs - 1),
        }
    }

    /// The ids the connection control methods name in their refusals.
    fn connection_ids(&self) -> String {
        self.session
            .conns
            .iter()
            .map(|conn| conn.cfg.id.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Resolve the connection a control request points at: the index it names, or the active one when
    /// it names none. `close_conn` leaves the catalog behind, so naming a table is not evidence the
    /// connection can be described — a connection that is missing, not numeric or closed is refused
    /// here, before the request touches the tab or the selection.
    fn inspector_target(session: &Session, conn: Option<&Value>) -> Result<usize, String> {
        let index = match conn {
            None => session.active_conn,
            Some(value) => value.as_u64().and_then(|value| usize::try_from(value).ok()),
        }
        .filter(|index| *index < session.conns.len())
        .ok_or_else(|| "no such connection (pass a numeric 'conn' index)".to_string())?;
        if !session.conns[index].is_open() {
            return Err(format!(
                "connection '{}' is not open",
                session.conns[index].cfg.name
            ));
        }
        Ok(index)
    }

    /// A single click in the tree: the inspector describes the object and nothing else happens.
    fn select_table(&mut self, index: usize, reference: TableRef) {
        // The connection is remembered by id: `conn.delete` removes an element and shifts every later
        // connection down, so a stored index would come to mean a different server.
        let Some(conn_id) = self
            .session
            .conns
            .get(index)
            .map(|conn| conn.cfg.id.clone())
        else {
            return;
        };
        let key = format!("{conn_id}:{}", reference.key());
        if self.state.inspector.key == key {
            return;
        }
        let table = self
            .session
            .conns
            .get(index)
            .and_then(|conn| conn.catalog.as_ref())
            .and_then(|catalog| catalog.find_table(reference.schema.as_deref(), &reference.table))
            .cloned();
        let engine = self
            .session
            .conns
            .get(index)
            .map(|conn| conn.cfg.kind.label().to_owned())
            .unwrap_or_default();
        let inspector = &mut self.state.inspector;
        inspector.key = key;
        inspector.selected = Some((conn_id, reference));
        inspector.engine = engine;
        inspector.cache = Default::default();
        inspector.preview = None;
        // A newly described object must not open on the row and column the previous one's table left
        // selected.
        inspector.preview_state = Default::default();
        inspector.total_rows = table
            .as_ref()
            .and_then(|table| table.row_estimate)
            .and_then(|rows| u64::try_from(rows).ok());
        inspector.meta = crate::inspector::meta(
            inspector.total_rows,
            table
                .as_ref()
                .and_then(|table| table.size_bytes)
                .and_then(|bytes| u64::try_from(bytes).ok()),
            table.as_ref().map(|table| table.columns.len()).unwrap_or(0),
        );
    }

    /// Fetch whatever the inspector's tab needs, once per object and tab. A fetch that fails is
    /// kept: the panel shows the message instead of asking again every frame.
    fn refresh_inspector(&mut self) {
        let Some((conn_id, reference)) = self.state.inspector.selected.clone() else {
            return;
        };
        // Resolved from the id every frame, so a connection that was deleted — or reshuffled — clears
        // the panel instead of being described with another connection's caches.
        let Some(index) = self
            .session
            .conns
            .iter()
            .position(|conn| conn.cfg.id == conn_id)
        else {
            self.state.inspector.clear();
            return;
        };
        let Some(conn) = self.session.conns.get(index) else {
            self.state.inspector.clear();
            return;
        };
        if !conn.is_open() {
            self.state.inspector.clear();
            return;
        }
        let id = conn.cfg.id.clone();
        let slot = self.state.inspector.tab.cache_slot();
        let needs_json = slot.is_some_and(|slot| self.state.inspector.cache[slot].is_none());
        if needs_json {
            if let Some(command) = self.state.inspector.tab.command() {
                let result = self
                    .session
                    .run(
                        command,
                        serde_json::json!({"conn": id, "table": reference.key()}),
                    )
                    .map_err(|error| error.to_string());
                if let Some(slot) = slot {
                    self.state.inspector.cache[slot] = Some(result);
                }
            }
        } else if slot.is_none() && self.state.inspector.preview.is_none() {
            let result =
                nook_engine::cmd::preview_table(&mut self.session, index, &reference, PREVIEW_ROWS)
                    .map_err(|error| error.to_string());
            self.state.inspector.preview = Some(result);
        }
    }

    /// The right-hand panel: the object inspector, shown while an object is selected.
    fn inspector_panel(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        if let crate::inspector::Action::OpenData =
            crate::inspector::show(ui, &mut self.state, tokens)
        {
            if let Some((conn_id, reference)) = self.state.inspector.selected.clone() {
                if let Some(index) = self
                    .session
                    .conns
                    .iter()
                    .position(|conn| conn.cfg.id == conn_id)
                {
                    tree::open_data_tab(&mut self.session, index, reference);
                }
            }
        }
    }

    /// A menu pick is a command run, not a form to fill in: `Session::run` resolves the connection
    /// and the tab a command needs (`resolve_conn`, `active_tab`) and every shortcut in the bar
    /// already calls it with `{}`, so the pick goes straight through. Only a command the engine
    /// refuses — a missing `key`, no open connection, a tab that is not there — opens the parameter
    /// window, and it opens on the engine's own message, so a value it genuinely needs is named.
    /// Every failure opens the window: the alternatives are sniffing error classes or adding a
    /// required-parameter field to the registry, and the message already says which value is short.
    fn menu_pick(session: &mut Session, state: &mut UiState, id: &str) {
        // Preferences is a form, not a runner: `open_command` owns that special case and opens the
        // settings dialog, which is the same thing the File ▸ App entry has always done.
        if id == "app.settings" {
            state.open_command(id.to_string());
            return;
        }
        if let Err(e) = session.run(id, serde_json::json!({})) {
            session.log(Level::Warn, e.to_string());
            state.open_command(id.to_string());
            state.command_result = Some(format!("error: {e}"));
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
            self.state.palette_objects = false;
            self.state.palette_query.clear();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) {
            self.state.palette_open = true;
            self.state.palette_objects = true;
            self.state.palette_query.clear();
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
                } else {
                    self.state.forget_tab(index);
                }
            }
        }
    }
    /// Reap finished `ai.generate` background requests; see [`crate::ai::drain_ai_outcome`] for the
    /// outcome rules. The thread did only the `curl` round-trip; everything session-mutating happens
    /// on the UI thread.
    fn drain_ai_outcomes(&mut self, ctx: &egui::Context) {
        crate::ai::drain_ai_outcome(&mut self.session, &mut self.state);
        // While a draft is in flight the spinner and the footer repaint need a new frame; without
        // this the one `request_repaint_after` the spawn asked for is the last one and the window
        // goes still even though the thread is still running.
        if self.state.ai_request.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl eframe::App for NookApp {
    /// The last write: a clean close snapshots the tabs even if the two-second idle window never
    /// came.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.session.persist_tabs();
    }

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
        self.drain_ai_outcomes(&ctx);
        self.shortcuts(&ctx);
        self.tick_jobs();

        // Phase 1: snapshot the tabs at most every two seconds of wall-clock time. egui repaints
        // only on input, so a "no input for 2 s" frame never arrives in the plain window — the
        // wake-up has to be scheduled (`request_repaint_after`), the same way `tick_jobs` ticks.
        // The write is skipped when the snapshot is unchanged, so an idle app writes nothing.
        self.tab_persist_at.get_or_insert_with(std::time::Instant::now);
        if self.tab_persist_at.unwrap().elapsed() >= std::time::Duration::from_secs(2) {
            self.tab_persist_at = Some(std::time::Instant::now());
            let before = self.last_tab_snapshot.clone();
            self.session.persist_tabs();
            let after = nook_engine::store::load_ui_state("tabs").ok().flatten();
            if after != before {
                self.last_tab_snapshot = after;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));

        if self.state.want_demo {
            self.state.want_demo = false;
            self.session = crate::demo_session();
        }

        // Bars paint their own fill and hairline inside an exact band, so neither the frame around
        // them nor egui's built-in separator line may add anything: each one costs a pixel, and the
        // design's bars are exactly 40, 35 and 26 tall with a single border inside that height.
        egui::Panel::top("app-bar")
            .show_separator_line(false)
            .frame(t::bar_frame(&tokens))
            .show(ui, |ui| self.app_bar(ui, &tokens));
        egui::Panel::top("tab-strip")
            .show_separator_line(false)
            .frame(t::bar_frame(&tokens))
            .show(ui, |ui| self.tab_strip(ui, &tokens));

        egui::Panel::bottom("status-bar")
            .show_separator_line(false)
            .frame(t::bar_frame(&tokens))
            .show(ui, |ui| self.status_bar(ui, &tokens));
        if self.state.log_open {
            egui::Panel::bottom("log")
                .resizable(true)
                .default_size(150.0)
                .max_size(400.0)
                .show_separator_line(false)
                .frame(t::side_panel_frame(&tokens))
                .show(ui, |ui| self.log_panel(ui, &tokens));
        }

        // The rail borrows the tree's left border rather than drawing its own, so the boundary
        // between them is one hairline instead of two.
        egui::Panel::left("activity-rail")
            .exact_size(t::RAIL_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(t::bar_frame(&tokens))
            .show(ui, |ui| self.rail(ui, &tokens));

        egui::Panel::left("tree")
            .resizable(true)
            .default_size(t::TREE_W)
            .show_separator_line(false)
            .frame(t::side_panel_frame(&tokens))
            .show(ui, |ui| self.sidebar(ui, &tokens));

        self.refresh_inspector();
        // The pane is part of the design's workbench whether or not something is selected: it explains
        // itself when it is empty, and showing it only on the first selection made the whole layout
        // shift under the click.
        egui::Panel::right("inspector")
            .exact_size(t::INSPECTOR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(t::side_panel_frame(&tokens))
            .show(ui, |ui| self.inspector_panel(ui, &tokens));

        // The design's workbench surfaces sit flush: the pane's bands and cards paint their own
        // insets, and the grid's hairlines run edge to edge inside it (`.reg-grid` has no margin of
        // its own), so the central pane carries none.
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(tokens.panel))
            .show(ui, |ui| self.central(ui, &tokens));

        dialogs::show(ui, &mut self.session, &mut self.state, &tokens);

        if self.state.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let _ = &self.options;
    }
}

/// The design's `.rbadge`: a 14px pill on the button's top-right corner, mono and strong-bordered so
/// it stays legible over whichever glyph sits under it.
fn rail_badge(ui: &egui::Ui, tokens: &Tokens, button: egui::Rect, count: usize) {
    let label = count.to_string();
    let font = tokens.mono(t::T_XS);
    let text = ui
        .painter()
        .layout_no_wrap(label.clone(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    let width = (text + 8.0).max(14.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(button.right() - 1.0 - width, button.top() + 1.0),
        Vec2::new(width, 14.0),
    );
    let radius = CornerRadius::same(t::R_CHIP);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tokens.panel_high);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke_strong),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        font,
        tokens.text,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_dialog_gets_the_table_it_was_opened_for() {
        // The buttons used to be sent as `…&table=` with nothing after it, which filled the dialog's
        // parameters with an empty table and made each button's Run fail.
        let mut state = UiState::default();
        state.open_command_with(
            "schema.addColumn",
            &[("conn", "conn-1"), ("table", "public.users")],
        );
        assert_eq!(state.command_id, "schema.addColumn");
        let params: serde_json::Value =
            serde_json::from_str(&state.command_params).expect("the dialog holds JSON");
        assert_eq!(params["conn"], "conn-1");
        assert_eq!(params["table"], "public.users");
        assert!(
            state.command_open,
            "the dialog has to be the thing that opened"
        );
    }

    #[test]
    fn a_table_or_a_path_holding_a_delimiter_survives_the_dialog() {
        // SQLite's connection id is the database path, and both a path and a table name may hold `&`
        // or `=`. Handing those through the `id&key=value` string would cut them in half — exactly
        // the failure the designer buttons used to have.
        let mut state = UiState::default();
        state.open_command_with(
            "schema.addColumn",
            &[("conn", "/Users/ann/R&D=1/app.db"), ("table", "R&D")],
        );
        let params: serde_json::Value =
            serde_json::from_str(&state.command_params).expect("the dialog holds JSON");
        assert_eq!(params["conn"], "/Users/ann/R&D=1/app.db");
        assert_eq!(params["table"], "R&D");
        assert!(
            params.get("1/app.db").is_none(),
            "nothing may leak in as a key: {params}"
        );
    }

    #[test]
    fn the_workspace_popup_offers_ids_not_positions() {
        let mut session = Session::new();
        for name in ["first", "second"] {
            session
                .conns
                .push(Conn::new(nook_core::ConnectionConfig::new(
                    name,
                    nook_core::DriverKind::Sqlite,
                )));
        }
        session.active_conn = Some(1);

        let rows = NookApp::workspace_rows(&session);
        assert_eq!(rows.len(), 2);
        // `conn.select` resolves an id or a name and never a position, so the popup must hand over
        // the id: passing the index made every pick fail with `no connection named '0'`.
        assert_eq!(rows[0].0, session.conns[0].cfg.id);
        assert_eq!(rows[1].0, session.conns[1].cfg.id);
        assert_eq!(rows[0].1, "first");
        assert!(!rows[0].2 && rows[1].2);
    }

    #[test]
    fn a_control_request_refuses_a_connection_it_cannot_describe() {
        // One open connection (the demo's SQLite) and one that was never opened: `close_conn` leaves
        // the catalog behind, so a connection that can name a table is not necessarily one that can
        // be read, and the request must be refused rather than answered `ok`.
        let mut session = crate::demo_session();
        session
            .conns
            .push(Conn::new(nook_core::ConnectionConfig::new(
                "closed",
                nook_core::DriverKind::Sqlite,
            )));
        assert!(session.conns[0].is_open());
        assert_eq!(
            NookApp::inspector_target(&session, Some(&serde_json::json!(0))),
            Ok(0)
        );
        assert_eq!(
            NookApp::inspector_target(&session, Some(&serde_json::json!(1))),
            Err("connection 'closed' is not open".to_string())
        );
        assert_eq!(
            NookApp::inspector_target(&session, Some(&serde_json::json!("zero"))),
            Err("no such connection (pass a numeric 'conn' index)".to_string())
        );
        assert!(NookApp::inspector_target(&session, Some(&serde_json::json!(7))).is_err());
        session.active_conn = Some(1);
        assert_eq!(
            NookApp::inspector_target(&session, None),
            Err("connection 'closed' is not open".to_string())
        );
    }

    #[test]
    fn a_menu_pick_runs_the_command_and_only_the_engine_refusal_opens_the_runner() {
        let mut session = crate::demo_session();
        let mut state = UiState::default();

        // A pick the engine can serve from context runs and opens nothing: the report is that every
        // pick landed in 'Run a Command', including the many commands that need no input.
        assert!(!session.messages.is_empty());
        NookApp::menu_pick(&mut session, &mut state, "app.clearLog");
        assert!(session.messages.is_empty(), "the pick ran the command");
        assert!(!state.command_open, "a command that ran opens no window");

        // Preferences is a form, not a command: the settings dialog, never the runner.
        NookApp::menu_pick(&mut session, &mut state, "app.settings");
        assert!(state.settings_open);
        assert!(!state.command_open, "Preferences is a form, not a runner");

        // A pick the engine refuses does open the runner, showing the engine's own message, so a
        // command that needs a value the context cannot supply still has a way to be given one.
        NookApp::menu_pick(&mut session, &mut state, "app.setSetting");
        assert!(state.command_open);
        assert_eq!(state.command_id, "app.setSetting");
        // The runner's own wording for a failure (`dialogs.rs`): the window reads the same whether
        // the command came from a menu pick or from the Run button in it.
        assert_eq!(
            state.command_result.as_deref(),
            Some("error: config: missing parameter 'key'")
        );
    }

    #[test]
    fn preferences_opens_the_dialog_and_everything_else_the_runner() {
        let mut state = UiState::default();
        state.open_command("app.settings".to_string());
        assert!(
            state.settings_open,
            "File ▸ App ▸ Preferences opens the dialog"
        );
        assert!(
            !state.command_open,
            "Preferences is a form, not a JSON runner"
        );

        state.open_command("sql.run".to_string());
        assert!(state.command_open);
        assert_eq!(state.command_id, "sql.run");
    }
}
