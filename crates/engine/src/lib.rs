#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L3: the session — every user-visible action is a command with an id, and the session owns
//! connections, tabs, the grid buffer, query history and the job store.

pub mod catalog;
pub mod cmd;
pub mod datagen;
pub mod jobs;
pub mod parity;
pub mod profiles;
pub mod report;
pub mod sqltext;
pub mod store;
pub mod tunnel;

use std::collections::BTreeMap;

use nook_core::{ConnectionConfig, Error, Result, Value};
use nook_drivers::Driver;
use nook_model::{Catalog, ResultSet, ServerInfo, Table};
use serde_json::json;

pub use catalog::{catalog as feature_catalog, CatalogEntry};
pub use jobs::{Job, JobKind, JobStore};
pub use profiles::ProfileStore;
pub use tunnel::{SshSecret, Tunnel, ASKPASS_CONN_ENV};

/// Command parameters: a JSON object, because the ribbon, the CLI, the control channel and MCP all
/// dispatch by id and must pass the same shapes.
#[derive(Debug, Clone, Default)]
pub struct Params(serde_json::Map<String, serde_json::Value>);

impl Params {
    /// The raw parameter map, for handlers that pass a whole subtree through.
    pub fn raw(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
    pub fn none() -> Self {
        Params(serde_json::Map::new())
    }

    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        match value {
            serde_json::Value::Null => Ok(Params::none()),
            serde_json::Value::Object(map) => Ok(Params(map)),
            other => Err(Error::config(format!(
                "command parameters must be a JSON object, got {}",
                json_kind(&other)
            ))),
        }
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.0.get(key)
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Required string parameter.
    pub fn str_arg(&self, key: &str) -> Result<String> {
        match self.0.get(key) {
            Some(serde_json::Value::String(s)) => Ok(s.clone()),
            Some(serde_json::Value::Number(n)) => Ok(n.to_string()),
            Some(serde_json::Value::Bool(b)) => Ok(b.to_string()),
            Some(other) => Err(Error::config(format!(
                "parameter '{key}' must be a string, got {}",
                json_kind(other)
            ))),
            None => Err(Error::config(format!("missing parameter '{key}'"))),
        }
    }

    pub fn str_opt(&self, key: &str) -> Option<String> {
        self.str_arg(key).ok()
    }

    pub fn int_arg(&self, key: &str) -> Result<i64> {
        match self.0.get(key) {
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .ok_or_else(|| Error::config(format!("parameter '{key}' is not an integer"))),
            Some(serde_json::Value::String(s)) => s
                .trim()
                .parse()
                .map_err(|_| Error::config(format!("parameter '{key}' is not an integer"))),
            Some(other) => Err(Error::config(format!(
                "parameter '{key}' must be an integer, got {}",
                json_kind(other)
            ))),
            None => Err(Error::config(format!("missing parameter '{key}'"))),
        }
    }

    pub fn int_opt(&self, key: &str, default: i64) -> i64 {
        self.int_arg(key).unwrap_or(default)
    }

    pub fn bool_opt(&self, key: &str, default: bool) -> bool {
        match self.0.get(key) {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::String(s)) => {
                matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
            }
            Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
            _ => default,
        }
    }

    /// A list of strings, accepting either a JSON array or a comma-separated string.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.0.get(key) {
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect(),
            Some(serde_json::Value::String(s)) => s
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
            _ => vec![],
        }
    }

    /// A cell payload, for grid edits and imports.
    pub fn cell(&self, key: &str) -> Option<Value> {
        self.0.get(key).map(Value::from_json)
    }

    /// Connection id, defaulting to the active connection.
    pub fn conn_id(&self) -> Option<String> {
        self.str_opt("conn").or_else(|| self.str_opt("connection"))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// One user-visible action. Every surface — ribbon, menu, shortcut, CLI, control channel, MCP —
/// resolves the same id through the same registry.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu/ribbon group, also the CLI's `commands --group` filter.
    pub group: &'static str,
    /// Keyboard shortcut as displayed, e.g. `cmd+n`.
    pub shortcut: &'static str,
    /// Parameter documentation printed by `nook-cli commands`.
    pub params: &'static str,
    pub help: &'static str,
    pub run: fn(&mut Session, &Params) -> Result<serde_json::Value>,
    /// Rough name shown when the command is unavailable and the user asks why.
    pub ready: fn(&Session) -> bool,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSpec")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("group", &self.group)
            .finish()
    }
}

impl CommandSpec {
    pub const fn new(
        id: &'static str,
        label: &'static str,
        group: &'static str,
        run: fn(&mut Session, &Params) -> Result<serde_json::Value>,
    ) -> Self {
        CommandSpec {
            id,
            label,
            group,
            shortcut: "",
            params: "",
            help: "",
            run,
            ready: |_| true,
        }
    }

    pub const fn shortcut(mut self, s: &'static str) -> Self {
        self.shortcut = s;
        self
    }

    pub const fn params(mut self, p: &'static str) -> Self {
        self.params = p;
        self
    }

    pub const fn help(mut self, h: &'static str) -> Self {
        self.help = h;
        self
    }

    pub const fn ready(mut self, r: fn(&Session) -> bool) -> Self {
        self.ready = r;
        self
    }
}

/// The whole registry, built once.
pub fn registry() -> &'static [CommandSpec] {
    static REGISTRY: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut all = Vec::new();
        all.extend(cmd::app::commands());
        all.extend(cmd::ai::commands());
        all.extend(cmd::conn::commands());
        all.extend(cmd::schema::commands());
        all.extend(cmd::grid::commands());
        all.extend(cmd::sql::commands());
        all.extend(cmd::ops::commands());
        all.extend(cmd::monitor::commands());
        all.extend(cmd::privs::commands());
        all.extend(cmd::tools::commands());
        all.extend(cmd::diagram::commands());
        all.sort_by_key(|c| (c.group, c.id));
        all
    })
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    registry().iter().find(|c| c.id == id)
}

// ------------------------------------------------------------------ session state

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    Connected,
    Failed,
}

/// A profile plus its live connection.
pub struct Conn {
    pub cfg: ConnectionConfig,
    pub driver: Option<Box<dyn Driver>>,
    pub server: Option<ServerInfo>,
    pub catalog: Option<Catalog>,
    pub error: Option<String>,
    pub state: ConnState,
    pub tunnel: Option<Tunnel>,
    /// Cached so the tree can show a timestamp without refetching.
    pub catalog_ms: u64,
}

impl Conn {
    pub fn new(cfg: ConnectionConfig) -> Self {
        Conn {
            cfg,
            driver: None,
            server: None,
            catalog: None,
            error: None,
            state: ConnState::Disconnected,
            tunnel: None,
            catalog_ms: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.driver.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Query,
    Data,
    Design,
    Diagram,
    Monitor,
    Transfer,
    Jobs,
    Report,
    Users,
    Welcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub table: String,
}

impl TableRef {
    pub fn key(&self) -> String {
        match &self.schema {
            Some(s) if !s.is_empty() => format!("{s}.{}", self.table),
            _ => self.table.clone(),
        }
    }

    pub fn parse(spec: &str) -> TableRef {
        match spec.rsplit_once('.') {
            Some((schema, table)) if !table.is_empty() => TableRef {
                schema: Some(schema.to_string()),
                table: table.to_string(),
            },
            _ => TableRef {
                schema: None,
                table: spec.to_string(),
            },
        }
    }
}

/// One pending change in the grid, shown as a badge until applied.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingEdit {
    pub kind: nook_model::EditKind,
    pub row: usize,
    pub column: String,
    pub value: Value,
    /// The columns an insert writes: kept so `grid.apply` can plan the statement again against the
    /// table as it is when the apply runs, rather than run a statement planned against a table that
    /// has since changed.
    pub insert: Vec<(String, Value)>,
    /// The primary-key values of the record this edit was made on, read when the edit was made and in
    /// the key columns' order. The apply pairs them with the key columns the table has *then*: the
    /// page under the tab can be sorted, filtered or paged between the two, and re-reading the row by
    /// index would key the statement to whichever record moved into that row. Keeping values rather
    /// than names is what lets a key column renamed in the meantime still resolve.
    ///
    /// The pairing is positional, so a key *set* replaced by another one of the same arity between the
    /// edit and the apply would pair an old value with a new name. Carrying the names instead would
    /// refuse the renamed key column above, which is the common case. The one door into the bad case is
    /// a key replacement typed straight into the SQL editor (`ALTER TABLE t DROP PRIMARY KEY, ADD
    /// PRIMARY KEY (other)` on MySQL) while a tab holds a pending edit; no Nook editor replaces a key,
    /// and SQLite cannot. The tradeoff is still the one worth making, and it is named here rather than
    /// left to be discovered.
    pub key: Vec<Value>,
    /// The statement this edit planned to. The grid shows it; the apply re-plans it.
    pub sql: String,
}

#[derive(Debug, Clone, Default)]
pub struct GridState {
    pub result: ResultSet,
    pub table: Option<TableRef>,
    pub columns: Vec<nook_model::Column>,
    pub primary_key: Vec<String>,
    pub page: usize,
    pub page_size: usize,
    pub order: String,
    pub filter: String,
    pub pending: Vec<PendingEdit>,
    pub cursor: Option<(usize, usize)>,
    pub total_rows: Option<u64>,
    pub error: Option<String>,
    pub elapsed_ms: f64,
}

impl GridState {
    /// Detach from any pending edits: switching table throws them away, which the UI warns about.
    pub fn clear(&mut self) {
        self.result = ResultSet::default();
        self.table = None;
        self.columns.clear();
        self.primary_key.clear();
        self.page = 0;
        self.order.clear();
        self.filter.clear();
        self.pending.clear();
        self.cursor = None;
        self.total_rows = None;
        self.error = None;
        self.elapsed_ms = 0.0;
    }

    pub fn dirty(&self) -> bool {
        !self.pending.is_empty()
    }

    /// True when `cmd::grid::edit_cell` would accept an edit: it refuses through `pk_values` an unbound or keyless table, so the UI must offer nothing the engine rejects.
    pub fn editable(&self) -> bool {
        self.error.is_none()
            && self.table.is_some()
            && !self.columns.is_empty()
            && !self.primary_key.is_empty()
    }

    /// Active filter clauses. `filter` holds one raw `WHERE` clause (`grid.filter`), so this counts 0 or 1.
    pub fn filter_count(&self) -> usize {
        usize::from(!self.filter.trim().is_empty())
    }

    /// Pending edits as (inserts, updates, deletes), so the UI prints `+n / ●n / −n` without re-deriving the kinds.
    pub fn pending_counts(&self) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        for edit in &self.pending {
            match edit.kind {
                nook_model::EditKind::InsertRow => counts.0 += 1,
                nook_model::EditKind::SetCell => counts.1 += 1,
                nook_model::EditKind::DeleteRow => counts.2 += 1,
            }
        }
        counts
    }
}

/// One statement of a run: what it was, what it produced, how long it took, and why it failed.
/// `sql.run` keeps one per statement, so the results register can show each statement's own rows.
#[derive(Debug, Clone)]
pub struct StatementRun {
    pub statement: String,
    pub result: Option<ResultSet>,
    pub elapsed_ms: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SqlState {
    pub text: String,
    pub last_result: Option<ResultSet>,
    pub elapsed_ms: f64,
    /// Statements of the last run, so `sql.explainStatement` and history can refer to them.
    pub statements: Vec<String>,
    /// The text the last run's request was issued against: the `sql` the command was given, or the
    /// editor's own text when it was not — never the statement some path reduced that text to. So a
    /// run from the control channel or the palette is stamped with what it ran rather than with
    /// whatever the editor happened to hold, and `Run statement` and `Run selection` are stamped with
    /// the editor's whole text. The results register records it as the source of the rows it shows and
    /// calls itself stale when the editor has moved on since.
    pub ran_script: String,
    /// One entry per statement of the last run, in run order. The next run replaces them.
    pub results: Vec<StatementRun>,
    pub history_cursor: Option<usize>,
}

impl SqlState {
    /// The failed run's message. At most one, because a run stops at the first failure.
    pub fn error(&self) -> Option<&str> {
        self.results.iter().find_map(|run| run.error.as_deref())
    }

    /// What the last run cost: the sum of its statements' own timings.
    pub fn total_ms(&self) -> f64 {
        self.results.iter().map(|run| run.elapsed_ms).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub conn: String,
    pub sql: String,
    pub ok: bool,
    pub elapsed_ms: f64,
    pub rows: usize,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snippet {
    pub name: String,
    pub sql: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: u64,
    pub kind: TabKind,
    pub title: String,
    pub conn: Option<usize>,
    pub grid: GridState,
    pub sql: SqlState,
    pub table: Option<Table>,
    /// Free-form payload for the diagram/report/monitor tabs (SQL, chart spec, …).
    pub payload: BTreeMap<String, serde_json::Value>,
}

impl Tab {
    pub fn new(id: u64, kind: TabKind, title: impl Into<String>) -> Self {
        Tab {
            id,
            kind,
            title: title.into(),
            conn: None,
            grid: GridState {
                page_size: 200,
                ..GridState::default()
            },
            sql: SqlState::default(),
            table: None,
            payload: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
    Success,
}

/// Row density, the design's Compact / Comfortable switch. `Compact` is `--h-row`, `Comfortable`
/// `--h-row-comfort`; the UI turns it into pixels, so this stays a preference and not a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Compact,
    Comfortable,
}

impl Density {
    pub fn parse(value: &str) -> Option<Density> {
        match value {
            "compact" => Some(Density::Compact),
            "comfortable" => Some(Density::Comfortable),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: String,
    pub density: Density,
    pub page_size: usize,
    pub max_rows: usize,
    pub font_size: f32,
    pub confirm_destructive: bool,
    pub auto_commit_edits: bool,
    pub keyword_case: sqltext::KeywordCase,
    pub keep_history: usize,
    pub show_system_objects: bool,
    /// The AI provider, model and endpoint behind `ai.generate`. The key is not here: it lives in
    /// the keychain, so the profile file never holds a secret.
    pub ai: nook_ai::AiConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "dark".into(),
            density: Density::Compact,
            page_size: 200,
            max_rows: nook_drivers::MAX_ROWS,
            font_size: 13.0,
            confirm_destructive: true,
            auto_commit_edits: false,
            keyword_case: sqltext::KeywordCase::Upper,
            keep_history: 500,
            show_system_objects: false,
            ai: nook_ai::AiConfig::default(),
        }
    }
}

/// The whole application state. Headless callers (CLI, MCP, control channel) use the same struct.
pub struct Session {
    pub conns: Vec<Conn>,
    pub active_conn: Option<usize>,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    pub profiles: ProfileStore,
    pub jobs: JobStore,
    pub history: Vec<HistoryEntry>,
    pub snippets: Vec<Snippet>,
    /// Connection ids pinned in the tree, first-painted first. Persisted in the store.
    pub pins: Vec<String>,
    pub messages: Vec<Message>,
    pub settings: Settings,
    next_tab_id: u64,
    /// Start directory for file dialogs and relative dumps.
    pub home: std::path::PathBuf,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let home = profiles::config_dir();
        let profiles = ProfileStore::load().unwrap_or_default();
        let jobs = JobStore::load().unwrap_or_default();
        // The store is the settings file too: preferences written by `app.setSetting` are read back
        // here on the next run.
        let mut settings = Settings::default();
        if let Some(density) = profiles
            .settings()
            .get("density")
            .and_then(|value| value.as_str())
            .and_then(Density::parse)
        {
            settings.density = density;
        }
        let mut session = Session {
            conns: Vec::new(),
            active_conn: None,
            tabs: vec![Tab::new(0, TabKind::Welcome, "Welcome")],
            active_tab: Some(0),
            profiles,
            jobs,
            history: Vec::new(),
            snippets: default_snippets(),
            pins: Vec::new(),
            messages: Vec::new(),
            settings,
            next_tab_id: 1,
            home,
        };
        // The AI configuration lives in the app's own database, not in the profile file. See
        // `load_ai_configuration` for how an install that predates it is carried over.
        session.load_ai_configuration();
        // Snippets, history and pins live in the same database. A start reads what a previous run
        // wrote; anything that fails leaves the in-memory defaults and says so, because a broken
        // state file is not worth refusing to start over.
        session.load_persisted_state();
        session.log(Level::Info, "Nook ready");
        session
    }

    /// Read snippets, history and pins from the store, replacing the in-memory defaults. A saved
    /// snippet with the same name as a built-in wins: the user edited it.
    fn load_persisted_state(&mut self) {
        match store::load_snippets() {
            Ok(saved) if !saved.is_empty() => {
                for snippet in saved {
                    match self.snippets.iter_mut().find(|s| s.name == snippet.name) {
                        Some(builtin) => *builtin = snippet,
                        None => self.snippets.push(snippet),
                    }
                }
            }
            Ok(_) => {}
            Err(e) => self.log(Level::Warn, format!("snippets: {}", e.message())),
        }
        match store::load_history() {
            Ok(saved) if !saved.is_empty() => {
                // Load directly, not through `remember`: remember is the write path for *new*
                // entries and pushes into the store, which would re-append every loaded row.
                self.history = saved;
                self.history.sort_by_key(|h| h.at);
            }
            Ok(_) => {}
            Err(e) => self.log(Level::Warn, format!("history: {}", e.message())),
        }
        match store::load_pins() {
            Ok(saved) => self.pins = saved,
            Err(e) => self.log(Level::Warn, format!("pins: {}", e.message())),
        }
    }

    /// Put the AI configuration in place at start-up.
    ///
    /// The database is where it is written (`store.rs`). An install that predates it still has the
    /// blob this feature used to write into `profiles.json`, so that is read here as a fallback — a
    /// start reads and never writes, so opening the app cannot rewrite the user's files. The first
    /// settings change is what moves the value across and drops the blob (`save_ai_configuration`).
    /// Anything that fails leaves the defaults and says so: a settings file is not worth refusing to
    /// start over.
    fn load_ai_configuration(&mut self) {
        match store::load_ai() {
            Ok(Some(cfg)) => self.settings.ai = cfg,
            Ok(None) => self.adopt_legacy_ai_blob(),
            Err(e) => self.log(
                Level::Warn,
                format!("the AI settings could not be read, so the defaults are in use: {e}"),
            ),
        }
    }

    /// Read the blob an earlier version wrote into the profile file, leaving the file alone.
    fn adopt_legacy_ai_blob(&mut self) {
        let Some(blob) = self.profiles.settings().get("ai") else {
            return;
        };
        match serde_json::from_value::<nook_ai::AiConfig>(blob.clone()) {
            Ok(cfg) => self.settings.ai = cfg,
            Err(e) => self.log(
                Level::Warn,
                format!("the saved AI settings could not be read, so the defaults are in use: {e}"),
            ),
        }
    }

    /// Write the AI configuration to the database, and — for an install that still carries the blob
    /// an earlier version wrote — drop that blob, so afterwards there is one place the settings can
    /// be. The database write happens first: a failure here must not cost the user the value.
    pub fn save_ai_configuration(&mut self) -> Result<()> {
        store::save_ai(&self.settings.ai)?;
        if self.profiles.remove_setting("ai") {
            if let Err(e) = self.profiles.save() {
                self.log(
                    Level::Warn,
                    format!("the old AI settings could not be removed from the profile file: {e}"),
                );
            }
        }
        Ok(())
    }

    /// Session with the saved profiles preloaded as connections.
    pub fn with_profiles() -> Self {
        let mut session = Session::new();
        let profiles = session.profiles.list().to_vec();
        for cfg in profiles {
            session.conns.push(Conn::new(cfg));
        }
        session
    }

    pub fn log(&mut self, level: Level, text: impl Into<String>) {
        self.messages.push(Message {
            level,
            text: text.into(),
        });
        if self.messages.len() > 2000 {
            let drop = self.messages.len() - 2000;
            self.messages.drain(0..drop);
        }
    }

    pub fn last_error(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.level == Level::Error)
            .map(|m| m.text.as_str())
    }

    pub fn add_tab(
        &mut self,
        kind: TabKind,
        title: impl Into<String>,
        conn: Option<usize>,
    ) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Tab::new(id, kind, title);
        tab.conn = conn;
        self.tabs.push(tab);
        self.active_tab = Some(self.tabs.len() - 1);
        self.tabs.len() - 1
    }

    pub fn close_tab(&mut self, index: usize) -> Result<()> {
        if index >= self.tabs.len() {
            return Err(Error::not_found(format!("no tab at {index}")));
        }
        if self.tabs[index].grid.dirty() {
            return Err(Error::config(
                "the tab has unapplied grid edits; revert or apply them first",
            ));
        }
        if self.tabs.len() == 1 {
            return Err(Error::config("the last tab cannot be closed"));
        }
        self.tabs.remove(index);
        self.active_tab = Some(
            self.active_tab
                .unwrap_or(0)
                .min(self.tabs.len().saturating_sub(1)),
        );
        Ok(())
    }

    pub fn active_tab_mut(&mut self) -> Result<&mut Tab> {
        let idx = self
            .active_tab
            .ok_or_else(|| Error::internal("no active tab"))?;
        self.tabs
            .get_mut(idx)
            .ok_or_else(|| Error::internal(format!("active tab {idx} is gone")))
    }

    pub fn active_tab(&self) -> Result<&Tab> {
        let idx = self
            .active_tab
            .ok_or_else(|| Error::internal("no active tab"))?;
        self.tabs
            .get(idx)
            .ok_or_else(|| Error::internal(format!("active tab {idx} is gone")))
    }

    /// Index of the connection a command should act on: the explicit `conn` parameter, else the tab's, else the active one.
    pub fn resolve_conn(&self, params: &Params) -> Result<usize> {
        if let Some(id) = params.conn_id() {
            return self.conn_index(&id);
        }
        if let Ok(tab) = self.active_tab() {
            if let Some(idx) = tab.conn {
                if idx < self.conns.len() {
                    return Ok(idx);
                }
            }
        }
        self.active_conn
            .ok_or_else(|| Error::config("no connection is open; open a connection first"))
    }

    pub fn conn_index(&self, id: &str) -> Result<usize> {
        self.conns
            .iter()
            .position(|c| c.cfg.id == id || c.cfg.name == id)
            .ok_or_else(|| Error::not_found(format!("no connection named '{id}'")))
    }

    pub fn conn_mut(&mut self, index: usize) -> Result<&mut Conn> {
        self.conns
            .get_mut(index)
            .ok_or_else(|| Error::not_found(format!("no connection at index {index}")))
    }

    pub fn driver_for(&mut self, index: usize) -> Result<&mut dyn Driver> {
        let conn = self.conn_mut(index)?;
        match conn.driver.as_mut() {
            Some(driver) => Ok(driver.as_mut()),
            None => Err(Error::connect(format!(
                "{} is not connected",
                conn.cfg.name
            ))),
        }
    }

    /// Two connections at once, for transfer and sync. Panics avoided: distinct indices are checked.
    pub fn two_drivers(
        &mut self,
        a: usize,
        b: usize,
    ) -> Result<(&mut dyn Driver, &mut dyn Driver)> {
        if a == b {
            return Err(Error::config(
                "source and target must be different connections",
            ));
        }
        if a >= self.conns.len() || b >= self.conns.len() {
            return Err(Error::not_found("connection index out of range"));
        }
        let (left, right) = if a < b {
            let (l, r) = self.conns.split_at_mut(b);
            (&mut l[a], &mut r[0])
        } else {
            let (l, r) = self.conns.split_at_mut(a);
            (&mut r[0], &mut l[b])
        };
        let da = left
            .driver
            .as_mut()
            .ok_or_else(|| Error::connect(format!("{} is not connected", left.cfg.name)))?;
        let db = right
            .driver
            .as_mut()
            .ok_or_else(|| Error::connect(format!("{} is not connected", right.cfg.name)))?;
        Ok((da.as_mut(), db.as_mut()))
    }

    /// Cached catalog of a connection, fetching it when asked.
    pub fn catalog_of(&self, index: usize) -> Result<&Catalog> {
        self.conns
            .get(index)
            .and_then(|c| c.catalog.as_ref())
            .ok_or_else(|| Error::config("no catalog yet; run conn.refresh first"))
    }

    pub fn table_of(&self, index: usize, schema: Option<&str>, table: &str) -> Result<Table> {
        self.catalog_of(index)?
            .find_table(schema, table)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("no such table: {table}")))
    }

    pub fn remember(&mut self, entry: HistoryEntry) {
        self.history.push(entry);
        if self.history.len() > self.settings.keep_history.max(10) {
            let drop = self.history.len() - self.settings.keep_history.max(10);
            self.history.drain(0..drop);
        }
        // The store is the durable twin of this vec. A failed write costs history rows, not data:
        // say so and keep going.
        if let Some(entry) = self.history.last() {
            if let Err(e) = store::push_history(entry) {
                self.log(Level::Warn, format!("history write: {}", e.message()));
            }
        }
        if let Err(e) = store::trim_history(self.settings.keep_history.max(10)) {
            self.log(Level::Warn, format!("history trim: {}", e.message()));
        }
    }

    /// The pinned connection ids, first-painted first.
    pub fn pinned(&self) -> &[String] {
        &self.pins
    }

    /// Snapshot the open tabs into the store: kind, title, editor text and the connection id.
    /// Deliberately not the grid edits, result rows or scroll position — a restored tab is a place
    /// to work, not a replay of a run. Returns the snapshot string so the caller can skip the
    /// write when nothing changed — an idle app must not touch the file.
    pub fn tab_snapshot(&self) -> String {
        #[derive(serde::Serialize)]
        struct SnapshotTab<'a> {
            kind: &'static str,
            title: &'a str,
            conn: Option<String>,
            sql: &'a str,
        }
        let kind_name = |kind: TabKind| match kind {
            TabKind::Query => "Query",
            TabKind::Data => "Data",
            TabKind::Design => "Design",
            TabKind::Diagram => "Diagram",
            TabKind::Monitor => "Monitor",
            TabKind::Transfer => "Transfer",
            TabKind::Jobs => "Jobs",
            TabKind::Report => "Report",
            TabKind::Users => "Users",
            TabKind::Welcome => "Welcome",
        };
        let tabs: Vec<SnapshotTab> = self
            .tabs
            .iter()
            .map(|t| SnapshotTab {
                kind: kind_name(t.kind),
                title: &t.title,
                conn: t.conn.and_then(|i| self.conns.get(i)).map(|c| c.cfg.id.clone()),
                sql: &t.sql.text,
            })
            .collect();
        json!({ "tabs": tabs, "active": self.active_tab }).to_string()
    }

    /// Write the tab snapshot unconditionally — the on-exit path, where the write must happen even
    /// if the content is unchanged (it also covers a first run whose store does not exist yet).
    pub fn persist_tabs(&mut self) {
        let snap = self.tab_snapshot();
        if let Err(e) = store::save_ui_state("tabs", &snap) {
            self.log(Level::Warn, format!("tab snapshot: {}", e.message()));
        }
    }

    /// Rebuild tabs from the last snapshot, replacing the default Welcome tab. Rules: at most 20
    /// tabs (the rest are logged away — a 400-tab snapshot is a bug in the caller); a tab whose
    /// connection id is unknown restores with no connection and names it, never a silent drop.
    pub fn restore_tabs(&mut self) {
        let Some(raw) = store::load_ui_state("tabs").unwrap_or(None) else {
            return;
        };
        let Ok(snap) = serde_json::from_str::<serde_json::Value>(&raw) else {
            self.log(Level::Warn, "tab snapshot is not valid JSON; starting fresh");
            return;
        };
        let Some(entries) = snap["tabs"].as_array() else {
            return;
        };
        let mut restored = 0usize;
        for entry in entries {
            if restored >= 20 {
                self.log(
                    Level::Warn,
                    format!(
                        "tab snapshot holds {} tabs; keeping the first 20",
                        entries.len()
                    ),
                );
                break;
            }
            let title = entry["title"].as_str().unwrap_or("Query").to_string();
            let conn_id = entry["conn"].as_str();
            let conn = conn_id.and_then(|id| {
                let found = self.conns.iter().position(|c| c.cfg.id == id);
                if found.is_none() {
                    self.log(
                        Level::Warn,
                        format!("tab '{title}': connection '{id}' no longer exists"),
                    );
                }
                found
            });
            let id = self.next_tab_id;
            self.next_tab_id += 1;
            let mut unknown_kind: Option<String> = None;
            let kind = match entry["kind"].as_str().unwrap_or("Query") {
                "Data" => TabKind::Data,
                "Design" => TabKind::Design,
                "Diagram" => TabKind::Diagram,
                "Monitor" => TabKind::Monitor,
                "Transfer" => TabKind::Transfer,
                "Jobs" => TabKind::Jobs,
                "Report" => TabKind::Report,
                "Users" => TabKind::Users,
                "Welcome" => TabKind::Welcome,
                other => {
                    unknown_kind = Some(other.to_string());
                    TabKind::Query
                }
            };
            if let Some(other) = unknown_kind {
                self.log(Level::Warn, format!("unknown tab kind '{other}'; restoring as Query"));
            }
            let mut tab = Tab::new(id, kind, title);
            tab.conn = conn;
            if let Some(sql) = entry["sql"].as_str() {
                tab.sql.text = sql.to_string();
            }
            self.tabs.push(tab);
            restored += 1;
        }
        // Replace the default Welcome tab when anything was restored.
        if restored > 0 {
            let first_welcome = self.tabs.iter().position(|t| t.kind == TabKind::Welcome);
            if let Some(i) = first_welcome {
                self.tabs.remove(i);
            }
            self.active_tab = Some(0);
        }
    }

    /// Run a command by id. Unknown ids and unavailable commands are errors, never panics.
    pub fn run(&mut self, id: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let spec = find(id).ok_or_else(|| Error::not_found(format!("unknown command '{id}'")))?;
        let params = Params::from_json(params)?;
        if !(spec.ready)(self) {
            return Err(Error::config(format!(
                "'{}' is not available right now",
                spec.label
            )));
        }
        match (spec.run)(self, &params) {
            Ok(value) => Ok(value),
            Err(e) => {
                self.log(Level::Error, format!("{id}: {e}"));
                Err(e)
            }
        }
    }

    /// Every command id that the registry knows, for the CLI and the MCP tool list.
    pub fn command_ids() -> Vec<&'static str> {
        registry().iter().map(|c| c.id).collect()
    }
}

fn default_snippets() -> Vec<Snippet> {
    vec![
        Snippet {
            name: "Top 100 rows".into(),
            sql: "SELECT * FROM <table> LIMIT 100;".into(),
            tags: vec!["select".into()],
        },
        Snippet {
            name: "Row count per table".into(),
            sql:
                "SELECT table_name, (SELECT count(*) FROM <table>) FROM information_schema.tables;"
                    .into(),
            tags: vec!["select".into()],
        },
        Snippet {
            name: "Current activity".into(),
            sql: "SELECT * FROM pg_stat_activity ORDER BY query_start DESC;".into(),
            tags: vec!["select".into()],
        },
        Snippet {
            name: "Explain a query".into(),
            sql: "EXPLAIN ANALYZE <query>;".into(),
            tags: vec!["select".into()],
        },
        Snippet {
            name: "Create index".into(),
            sql: "CREATE INDEX ix_<table>_<column> ON <table> (<column>);".into(),
            tags: vec!["ddl".into()],
        },
    ]
}

/// Convenience for commands: the value of a JSON summary.
pub fn ok() -> serde_json::Value {
    json!({ "ok": true })
}

/// A lock serializing every test that points `NOOK_CONFIG_DIR` at a scratch directory — the
/// integration tests under `tests/` share the one static mutex with the unit tests. Public
/// because the integration tests cannot see `#[cfg(test)]` items.
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
    LOCK.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Helpers shared by the tests that touch the filesystem.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::MutexGuard;

    /// Point the config directory at a scratch path and hold the lock that keeps the job-store tests
    /// from writing over each other. The guard has to stay alive for the whole test.
    pub fn scratch_config() -> MutexGuard<'static, ()> {
        let guard = super::test_lock();
        let dir = std::env::temp_dir().join("nook-test-config");
        let _ = std::fs::create_dir_all(&dir);
        // Start from an empty store every time: a leftover jobs.json or nook.db from a previous run
        // would change what the test sees.
        for file in ["jobs.json", "profiles.json", "nook.db"] {
            let _ = std::fs::remove_file(dir.join(file));
        }
        std::env::set_var("NOOK_CONFIG_DIR", dir);
        guard
    }

    /// A fake `security` on `NOOK_SECURITY_BIN`: it records every call's arguments in a log and
    /// answers a lookup for `account` with `secret`, so the keychain paths can be exercised without
    /// the login keychain. It reads nothing on stdin and never prompts, and it succeeds on a store
    /// without writing anything, which is what lets a test assert the secret arrived as the store
    /// child's `-w` value and nowhere else it can observe. It takes the same lock as `scratch_config`
    /// — a process-wide override cannot be set while another test reads it — and returns the log the
    /// assertions read.
    pub fn fake_security(
        account: &str,
        secret: &str,
    ) -> (MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = scratch_config();
        let dir = std::env::temp_dir().join("nook-test-security");
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("calls.log");
        let _ = std::fs::remove_file(&log);
        let script = dir.join("security");
        // Logs the argv of every call, answers `find` with the secret, and succeeds on `add`: the store
        // passes the secret as `-w`'s value, so the fake never reads stdin — a `read` here would block
        // the moment the child inherits a terminal.
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\ncase \"$*\" in\n  *find-generic-password*{account}*) printf '%s\\n' '{secret}' ;;\nesac\nexit 0\n",
            log.display(),
        );
        let _ = std::fs::write(&script, body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("NOOK_SECURITY_BIN", &script);
        (guard, log)
    }

    /// A fake `curl` on `NOOK_CURL_BIN`: it records the whole command line plus the stdin body
    /// to a log and exits 0. Same lock discipline as `fake_security` — one process-wide override.
    pub fn fake_curl() -> (MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = scratch_config();
        let dir = std::env::temp_dir().join("nook-test-curl");
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("calls.log");
        let _ = std::fs::remove_file(&log);
        let script = dir.join("curl");
        let body = format!(
            "#!/bin/sh\n{{ printf '%s\\n' \"$*\"; cat; printf '\\n'; }} >> \"{}\"\nexit 0\n",
            log.display(),
        );
        let _ = std::fs::write(&script, body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("NOOK_CURL_BIN", &script);
        (guard, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_are_typed_and_forgiving() {
        let p = Params::from_json(json!({
            "name": "users",
            "limit": "50",
            "flag": true,
            "tables": "a, b,c",
            "list": ["x", 1]
        }))
        .expect("params");
        assert_eq!(p.str_arg("name").expect("name"), "users");
        assert_eq!(p.int_arg("limit").ok(), Some(50));
        assert!(p.bool_opt("flag", false));
        assert_eq!(p.list("tables"), vec!["a", "b", "c"]);
        assert_eq!(p.list("list").len(), 2);
        assert_eq!(p.int_opt("missing", 7), 7);
        assert!(p.str_arg("missing").is_err());
        assert!(Params::from_json(json!(5)).is_err());
        assert!(Params::from_json(serde_json::Value::Null).is_ok());
    }

    #[test]
    fn unknown_command_is_an_error() {
        let mut s = Session::new();
        let e = s.run("does.not.exist", json!({}));
        assert!(e.is_err());
    }

    #[test]
    fn a_legacy_blob_is_read_at_start_and_moved_on_the_first_change() {
        let _guard = test_support::scratch_config();
        let file = profiles::config_dir().join("profiles.json");
        let written = json!({
            "version": 1,
            "profiles": [],
            "settings": {
                "ai": {"provider": "moonshot", "model": "moonshot-v1-8k"},
                "density": "compact"
            }
        });
        std::fs::write(&file, written.to_string()).expect("write the settings file");

        // A start reads it — the alias is read the way the rest of the surface reads a provider
        // name — and writes nothing: opening the app does not rewrite the user's files.
        let mut session = Session::new();
        assert_eq!(session.settings.ai.provider, nook_ai::Provider::Kimi);
        assert_eq!(session.settings.ai.model, "moonshot-v1-8k");
        assert_eq!(
            store::load_ai().expect("the database"),
            None,
            "nothing written yet"
        );
        let untouched = profiles::ProfileStore::load().expect("reload the store");
        assert!(
            untouched.settings().get("ai").is_some(),
            "the blob was not touched"
        );

        // The first settings change is what moves it, and the old key goes with it.
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_model", "value": "moonshot-v1-128k"}),
            )
            .expect("set the model");
        assert_eq!(
            store::load_ai().expect("the database").map(|c| c.model),
            Some("moonshot-v1-128k".to_string()),
            "the value reached the database"
        );
        let after = profiles::ProfileStore::load().expect("reload the store");
        assert!(after.settings().get("ai").is_none(), "{}", after.settings());
        assert_eq!(
            after.settings().get("density"),
            Some(&json!("compact")),
            "everything beside it is still there"
        );

        // A later start reads the database, not the file.
        let again = Session::new();
        assert_eq!(again.settings.ai.model, "moonshot-v1-128k");
    }

    #[test]
    fn a_legacy_blob_that_cannot_be_read_is_said_out_loud_and_left_alone() {
        let _guard = test_support::scratch_config();
        let file = profiles::config_dir().join("profiles.json");
        let written = json!({
            "version": 1,
            "profiles": [],
            "settings": {"ai": {"provider": "opemai"}}
        });
        std::fs::write(&file, written.to_string()).expect("write the settings file");

        // The defaults stand in, but running on them silently would look like the app forgot the
        // settings rather than like the file naming a provider that does not exist.
        let session = Session::new();
        assert_eq!(session.settings.ai.provider, nook_ai::Provider::OpenAi);
        assert!(
            session
                .messages
                .iter()
                .any(|m| m.level == Level::Warn && m.text.contains("AI settings")),
            "the file named a provider that does not exist, and nothing was said: {:?}",
            session.messages
        );
        // Nothing was written, so nothing may be deleted: a bad read must not cost the user the blob.
        let after = profiles::ProfileStore::load().expect("reload the store");
        assert!(after.settings().get("ai").is_some(), "{}", after.settings());
        assert_eq!(store::load_ai().expect("the database"), None);
    }

    #[test]
    fn a_preference_change_outlives_the_session() {
        let _guard = test_support::scratch_config();
        let mut session = Session::new();
        // The provider first, then the model: a switch clears what belonged to the old provider, so
        // the other order would test the clearing rule rather than persistence.
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_provider", "value": "ollama"}),
            )
            .expect("set the provider");
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_model", "value": "llama3.1"}),
            )
            .expect("set the model");

        let stored = std::fs::read_to_string(profiles::config_dir().join("profiles.json"))
            .unwrap_or_default();
        assert!(
            !stored.contains("\"ai\""),
            "the profile file kept the AI settings: {stored}"
        );

        let restart = Session::new();
        assert_eq!(restart.settings.ai.provider, nook_ai::Provider::Ollama);
        assert_eq!(restart.settings.ai.model, "llama3.1");
    }

    #[test]
    fn switching_provider_drops_the_model_and_the_address_that_belonged_to_the_old_one() {
        let _guard = test_support::scratch_config();
        let mut session = Session::new();
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_model", "value": "gpt-4o"}),
            )
            .expect("set the model");
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_base_url", "value": "https://gateway.example/v1"}),
            )
            .expect("set the address");

        session
            .run(
                "app.setSetting",
                json!({"key": "ai_provider", "value": "anthropic"}),
            )
            .expect("switch provider");
        // A model id that belongs to another vendor is a request the server refuses, and the endpoint
        // is another vendor's host: both fall back to the new provider's own.
        assert_eq!(session.settings.ai.model, "");
        assert_eq!(session.settings.ai.base_url, "");
        assert_eq!(session.settings.ai.provider, nook_ai::Provider::Anthropic);

        // A restart agrees, so the rule reached the store and not only the running session.
        let restart = Session::new();
        assert_eq!(restart.settings.ai.model, "");
        assert_eq!(restart.settings.ai.base_url, "");

        // Re-picking the provider already chosen is not a switch, so it clears nothing.
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_model", "value": "claude-3-5-sonnet-latest"}),
            )
            .expect("set the model");
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_provider", "value": "anthropic"}),
            )
            .expect("re-pick the same provider");
        assert_eq!(session.settings.ai.model, "claude-3-5-sonnet-latest");
    }

    #[test]
    fn the_database_wins_over_a_blob_left_behind() {
        let _guard = test_support::scratch_config();
        let mut session = Session::new();
        session
            .run(
                "app.setSetting",
                json!({"key": "ai_model", "value": "stored-model"}),
            )
            .expect("set the model");

        // A hand-edited file that disagrees with the database.
        let mut store = profiles::ProfileStore::load().expect("load");
        store.set_setting("ai", json!({"provider": "openai", "model": "hand-written"}));
        store.save().expect("save");

        let mut restart = Session::new();
        assert_eq!(
            restart.settings.ai.model, "stored-model",
            "the database is the source"
        );

        // The blob is not touched by a start, and the next write is what clears it, so the two
        // cannot disagree from then on.
        assert!(profiles::ProfileStore::load()
            .expect("reload")
            .settings()
            .get("ai")
            .is_some());
        restart
            .run(
                "app.setSetting",
                json!({"key": "ai_model", "value": "next"}),
            )
            .expect("set the model again");
        let after = profiles::ProfileStore::load().expect("reload");
        assert!(after.settings().get("ai").is_none(), "{}", after.settings());
    }

    #[test]
    fn an_unreadable_row_warns_and_leaves_the_defaults() {
        let _guard = test_support::scratch_config();
        store::save_ai(&nook_ai::AiConfig {
            model: "stored".into(),
            ..nook_ai::AiConfig::default()
        })
        .expect("save");
        // A row the app did not write: the right shape, a provider this build does not know.
        let conn = rusqlite::Connection::open(profiles::ai_db_path()).expect("open the database");
        conn.execute("UPDATE ai_config SET provider = 'opemai' WHERE id = 1", [])
            .expect("break the row");
        drop(conn);

        let session = Session::new();
        assert_eq!(session.settings.ai.provider, nook_ai::Provider::OpenAi);
        assert!(
            session
                .messages
                .iter()
                .any(|m| m.level == Level::Warn && m.text.contains("AI settings")),
            "{:?}",
            session.messages
        );
    }

    #[test]
    fn a_normal_start_says_nothing_about_the_ai_configuration() {
        let _guard = test_support::scratch_config();
        let session = Session::new();
        assert!(
            !session
                .messages
                .iter()
                .any(|m| m.level == Level::Warn && m.text.contains("AI settings")),
            "a first run is not a problem: {:?}",
            session.messages
        );
    }

    #[test]
    fn commands_run_with_junk_parameters_never_panic() {
        // The WordCraft-style hostile-input sweep: every command, on an empty session, with nonsense.
        let junk = [
            json!({}),
            json!({"conn": "nope"}),
            json!({"conn": 42, "table": [], "sql": {}, "value": "x"}),
            json!({"index": -1, "limit": "many", "page": "NaN"}),
        ];
        for id in Session::command_ids() {
            for params in &junk {
                let mut session = Session::new();
                let _ = session.run(id, params.clone());
            }
        }
    }

    #[test]
    fn table_ref_parses_qualified_names() {
        assert_eq!(
            TableRef::parse("public.users"),
            TableRef {
                schema: Some("public".into()),
                table: "users".into()
            }
        );
        assert_eq!(TableRef::parse("users").schema, None);
        assert_eq!(TableRef::parse("users").key(), "users");
    }

    #[test]
    fn registry_ids_are_unique() {
        let mut ids: Vec<&str> = registry().iter().map(|c| c.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate command ids in the registry");
        assert!(before > 80, "expected a real command surface, got {before}");
    }

    /// A grid the engine would accept a cell edit for: bound to a table, with a key column.
    fn editable_grid() -> GridState {
        GridState {
            table: Some(TableRef::parse("users")),
            columns: vec![nook_model::Column {
                name: "id".into(),
                ..Default::default()
            }],
            primary_key: vec!["id".into()],
            ..GridState::default()
        }
    }

    fn pending(kind: nook_model::EditKind) -> PendingEdit {
        PendingEdit {
            kind,
            row: 0,
            column: String::new(),
            value: Value::Null,
            insert: Vec::new(),
            key: vec![Value::Int(1)],
            sql: String::new(),
        }
    }

    #[test]
    fn editable_follows_the_primary_key() {
        let mut grid = editable_grid();
        assert!(grid.editable());
        grid.primary_key.clear();
        assert!(!grid.editable());
    }

    #[test]
    fn editable_is_false_when_the_page_failed_or_is_unusable() {
        let mut grid = editable_grid();
        grid.error = Some("connection reset".into());
        assert!(!grid.editable());
        let mut grid = editable_grid();
        grid.columns.clear();
        assert!(!grid.editable());
        let mut grid = editable_grid();
        grid.table = None;
        assert!(!grid.editable());
    }

    #[test]
    fn pending_counts_start_empty_and_tally_each_kind() {
        let mut grid = GridState::default();
        assert_eq!(grid.pending_counts(), (0, 0, 0));
        grid.pending.push(pending(nook_model::EditKind::InsertRow));
        grid.pending.push(pending(nook_model::EditKind::SetCell));
        grid.pending.push(pending(nook_model::EditKind::DeleteRow));
        assert_eq!(grid.pending_counts(), (1, 1, 1));
    }

    #[test]
    fn pending_counts_tally_a_mixed_buffer() {
        let mut grid = GridState::default();
        grid.pending.push(pending(nook_model::EditKind::SetCell));
        grid.pending.push(pending(nook_model::EditKind::SetCell));
        grid.pending.push(pending(nook_model::EditKind::DeleteRow));
        assert_eq!(grid.pending_counts(), (0, 2, 1));
    }

    #[test]
    fn filter_count_is_zero_or_one() {
        let mut grid = GridState::default();
        assert_eq!(grid.filter_count(), 0);
        grid.filter = "status = 'live'".into();
        assert_eq!(grid.filter_count(), 1);
        grid.filter = "   ".into();
        assert_eq!(grid.filter_count(), 0);
    }
}
