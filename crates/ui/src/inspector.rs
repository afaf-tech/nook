//! The object inspector: the panel on the right that describes whatever the tree has selected.
//!
//! The module is a view model plus a painter and nothing else. `app.rs` owns the selection, runs the
//! command each tab asks for and stores the result, so everything below is a pure function of JSON —
//! no session, no database, and a test can call all of it.

use datacraft_engine::TableRef;
use datacraft_model::ResultSet;
use egui::{Align2, CornerRadius, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::app::UiState;
use crate::icons::{self, Glyph};
use crate::table::{TableColumn, TableState};
use crate::theme::{self as design, Tokens};
use crate::views::{self, List};

/// `.ih`: the object header. With the tab strip and the list head it makes the pane head, which
/// [`design::PANE_HEAD_H`] pins against the tree's.
pub const HEADER_H: f32 = 64.0;
/// `.lhead` and `.ghead`, the two heads a list can start with. The design uses the same 28 as the
/// grid's head, so this is `HEAD_H` rather than a second literal: the pane-head contract is asserted
/// against `HEAD_H`, and a private copy could drift while that test still passed.
const LIST_HEAD_H: f32 = design::HEAD_H;
/// `.crow`: the design's `--h-tree`, two lines of facts per object.
const CROW_H: f32 = design::TREE_H;
/// The inspector's `.chip`, which is 16px rather than the top bar's 24px state chip.
const CHIP_H: f32 = 16.0;

/// The four things the inspector can say about an object, in the design's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Columns,
    Indexes,
    Keys,
    Preview,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Columns, Tab::Indexes, Tab::Keys, Tab::Preview];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Columns => "Columns",
            Tab::Indexes => "Indexes",
            Tab::Keys => "Foreign keys",
            Tab::Preview => "Preview",
        }
    }

    /// The engine command whose result this tab shows. The preview has none: it is a page of rows
    /// read through [`datacraft_engine::cmd::preview_table`], not a schema command.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Tab::Columns => Some("schema.columns"),
            Tab::Indexes => Some("schema.indexes"),
            Tab::Keys => Some("schema.foreignKeys"),
            Tab::Preview => None,
        }
    }

    /// Where this tab's result lives in the panel's cache. The preview is not in it: its result is
    /// typed rows rather than JSON, so it has a field of its own.
    pub fn cache_slot(self) -> Option<usize> {
        match self {
            Tab::Columns => Some(0),
            Tab::Indexes => Some(1),
            Tab::Keys => Some(2),
            Tab::Preview => None,
        }
    }
}

/// The chip that qualifies a row: the design's `.chip`, in the three inks its model emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    /// `.chip.pk`: `accent` ink inside an `accent_line` border.
    Primary,
    /// `.chip.dw`: `warn` ink, which is how the design marks a referential action.
    Warn,
    /// A plain `.chip`: `text_dim` ink inside a `stroke_strong` border.
    Plain,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    pub text: String,
    pub kind: TagKind,
}

/// How one part of a row's second line is painted, from the design's `.l2` spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Plain,
    /// `.nul`: `text_faint` and italic, for a nullable column with no value.
    Null,
    /// `.cmt`: `text_dim` and italic, for a comment the schema carries.
    Comment,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub text: String,
    pub kind: PartKind,
}

impl Part {
    fn plain(text: impl Into<String>) -> Self {
        Part {
            text: text.into(),
            kind: PartKind::Plain,
        }
    }
}

/// One `.crow`.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    /// `.cname.mono`: an index or a key names an identifier, a column names a word.
    pub mono: bool,
    pub tag: Option<Tag>,
    pub detail: Vec<Part>,
}

/// A `.ghead` group with its rows. A list without a group title is simply the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub title: String,
    pub count: usize,
    /// `.gnote`: one line explaining an empty group.
    pub note: Option<String>,
    pub rows: Vec<Row>,
}

/// A tab's content, built from the command's JSON.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub head: String,
    pub legend: String,
    pub groups: Vec<Group>,
}

/// The inspector's own state: which tab, which object, and the caches behind the two.
#[derive(Default)]
pub struct Inspector {
    pub tab: Tab,
    /// The described object and the connection holding it. The connection is named by its **id**
    /// rather than its index: `conn.delete` removes an element and shifts every later connection
    /// down, so a stored index would silently come to mean a different server.
    pub selected: Option<(String, TableRef)>,
    /// The engine's label for the selected connection, for the header's chip.
    pub engine: String,
    /// The facts line under the breadcrumb.
    pub meta: String,
    /// What the caches below were fetched for: the connection id and the table's key. A different
    /// selection replaces everything, so a stale row can never appear under a fresh name.
    pub key: String,
    /// One slot per schema tab, in `Tab::cache_slot` order.
    pub cache: [Option<Result<serde_json::Value, String>>; 3],
    /// The preview's page, typed, because the grid renders `datacraft_core::Value` and not JSON.
    pub preview: Option<Result<ResultSet, String>>,
    /// The row count the connection's catalog last reported, for the preview's footer.
    pub total_rows: Option<u64>,
    /// The preview grid's own selection and hover state.
    pub preview_state: TableState,
}

impl Inspector {
    /// Forget the selection and everything fetched for it: the panel is hidden until the tree
    /// selects something again, and no row of a closed connection stays on screen.
    pub fn clear(&mut self) {
        self.selected = None;
        self.key.clear();
        self.engine.clear();
        self.meta.clear();
        self.total_rows = None;
        self.cache = Default::default();
        self.preview = None;
    }
}

/// What the panel wants the app to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// `.btn.ba` in the header.
    OpenData,
}

/// The design's `.ih-meta`: what the panel knows about the object before it lists anything.
///
/// Rows and size are estimates the catalog may not have; what a driver does not report is left out
/// rather than shown as a zero.
pub fn meta(rows: Option<u64>, size: Option<u64>, columns: usize) -> String {
    use crate::table::{grouped, plural};
    let mut parts = Vec::new();
    if let Some(rows) = rows {
        parts.push(format!("{} {}", grouped(rows), plural(rows, "row", "rows")));
    }
    if let Some(size) = size {
        parts.push(bytes(size));
    }
    parts.push(format!(
        "{} {}",
        grouped(columns as u64),
        plural(columns as u64, "column", "columns")
    ));
    parts.join(" · ")
}

/// `1.8 MB`: the design's size, in the largest unit that keeps a digit before the point.
fn bytes(size: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1 << 30), ("MB", 1 << 20), ("KB", 1 << 10)];
    for (unit, scale) in UNITS {
        if size >= scale {
            return format!("{:.1} {unit}", size as f64 / scale as f64);
        }
    }
    format!("{size} B")
}

fn text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string().trim_matches('"').to_owned(),
    }
}

fn strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(text).collect())
        .unwrap_or_default()
}

/// `main.customers`, or just `customers` when the driver has no schemas.
fn qualified(schema: &str, table: &str) -> String {
    if schema.is_empty() {
        table.to_owned()
    } else {
        format!("{schema}.{table}")
    }
}

/// The object the command ran against, in the `"schema.table"` form the engine uses.
fn table_of(json: &serde_json::Value) -> String {
    match text(&json["table"]) {
        key if key.is_empty() => String::new(),
        key => key.split('.').next_back().unwrap_or(&key).to_owned(),
    }
}

/// The Columns tab. `json` is a `schema.columns` result.
pub fn columns(json: &serde_json::Value) -> Model {
    let entries = json["columns"].as_array().cloned().unwrap_or_default();
    let rows = entries
        .iter()
        .map(|column| {
            let mut detail = vec![Part::plain(text(&column["type"]))];
            detail.push(if column["nullable"].as_bool().unwrap_or(true) {
                Part {
                    text: "NULL".into(),
                    kind: PartKind::Null,
                }
            } else {
                Part::plain("NN")
            });
            let default = text(&column["default"]);
            if !default.is_empty() {
                detail.push(Part::plain(default));
            }
            if column["auto_increment"].as_bool().unwrap_or(false) {
                detail.push(Part::plain("rowid alias"));
            }
            let comment = text(&column["comment"]);
            if !comment.is_empty() {
                detail.push(Part {
                    text: comment,
                    kind: PartKind::Comment,
                });
            }
            Row {
                name: text(&column["name"]),
                mono: false,
                tag: column["primary_key"]
                    .as_bool()
                    .unwrap_or(false)
                    .then(|| Tag {
                        text: "PK".into(),
                        kind: TagKind::Primary,
                    }),
                detail,
            }
        })
        .collect::<Vec<_>>();
    Model {
        head: "Column".into(),
        // The design teaches the two words the rows use in the head's legend.
        legend: "NN not null · NULL nullable".into(),
        groups: vec![Group {
            title: String::new(),
            count: rows.len(),
            note: None,
            rows,
        }],
    }
}

/// The Indexes tab. `json` is a `schema.indexes` result.
pub fn indexes(json: &serde_json::Value) -> Model {
    let entries = json["indexes"].as_array().cloned().unwrap_or_default();
    let rows = entries
        .iter()
        .map(|index| {
            let tag = if index["primary"].as_bool().unwrap_or(false) {
                // A non-unique index has nothing to say about itself beyond its columns.
                Some(Tag {
                    text: "PRIMARY".into(),
                    kind: TagKind::Primary,
                })
            } else if index["unique"].as_bool().unwrap_or(false) {
                Some(Tag {
                    text: "UNIQUE".into(),
                    kind: TagKind::Plain,
                })
            } else {
                None
            };
            let mut detail = vec![Part::plain(strings(&index["columns"]).join(", "))];
            let kind = text(&index["kind"]);
            if !kind.is_empty() {
                detail.push(Part::plain(kind));
            }
            Row {
                name: text(&index["name"]),
                mono: true,
                tag,
                detail,
            }
        })
        .collect::<Vec<_>>();
    Model {
        head: "Index".into(),
        legend: format!("{} defined", rows.len()),
        groups: vec![Group {
            title: String::new(),
            count: rows.len(),
            note: None,
            rows,
        }],
    }
}

/// The Foreign keys tab. `json` is a `schema.foreignKeys` result: the keys this table defines.
///
/// The design's second group lists the tables that reference this one.
///
/// ponytail: only the outgoing half is shown. Listing what references a table needs a scan no command
/// performs, so the model states the outgoing keys and says so rather than faking a second group;
/// add a `schema.scanReferences` command and a second `Group` here when that is worth the round trip.
pub fn keys(json: &serde_json::Value) -> Model {
    let table = table_of(json);
    let entries = json["foreign_keys"].as_array().cloned().unwrap_or_default();
    let rows = entries
        .iter()
        .map(|key| {
            let target = qualified(&text(&key["ref_schema"]), &text(&key["ref_table"]));
            let columns = strings(&key["ref_columns"]).join(", ");
            let mut detail = vec![Part::plain(if columns.is_empty() {
                target
            } else {
                format!("{target}.{columns}")
            })];
            let on_update = text(&key["on_update"]);
            if !on_update.is_empty() {
                detail.push(Part::plain(format!("ON UPDATE {on_update}")));
            }
            let on_delete = text(&key["on_delete"]);
            Row {
                name: format!("{table}.{}", strings(&key["columns"]).join(", ")),
                mono: true,
                tag: (!on_delete.is_empty()).then(|| Tag {
                    text: format!("ON DELETE {on_delete}"),
                    kind: TagKind::Warn,
                }),
                detail,
            }
        })
        .collect::<Vec<_>>();
    // The design's `.gnote` names the object: "main.customers defines no foreign keys."
    let note = rows
        .is_empty()
        .then(|| format!("{} defines no foreign keys.", text(&json["table"])));
    Model {
        head: "Foreign key".into(),
        legend: format!("{} defined", rows.len()),
        groups: vec![Group {
            title: "Outgoing".into(),
            count: rows.len(),
            note,
            rows,
        }],
    }
}

/// Build the model for `tab` from the result the app fetched, or the message saying why not.
pub fn model(
    tab: Tab,
    result: Option<&Result<serde_json::Value, String>>,
) -> (Model, Option<String>) {
    match result {
        None => (Model::default(), None),
        Some(Err(error)) => (Model::default(), Some(error.clone())),
        Some(Ok(json)) => (
            match tab {
                Tab::Columns => columns(json),
                Tab::Indexes => indexes(json),
                Tab::Keys => keys(json),
                Tab::Preview => Model::default(),
            },
            None,
        ),
    }
}

/// The design's `.tabs`: a 40px band holding the pane variant of the segmented control. `enabled` is
/// false for the empty pane, whose four labels are painted but inert and unringed.
fn tabs_band(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    options: &[(&str, bool)],
    enabled: bool,
) -> Option<usize> {
    let (band, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), design::TOOLBAR_H),
        Sense::hover(),
    );
    let inner = egui::Rect::from_min_max(
        egui::pos2(band.left() + f32::from(design::S5), band.top()),
        egui::pos2(band.right() - f32::from(design::S5), band.bottom()),
    );
    let mut band_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let clicked = design::pane_segmented(
        &mut band_ui,
        tokens,
        options,
        inner.width(),
        design::SEG_H,
        2.0,
        tokens.accent,
    );
    ui.painter().hline(
        band.left()..=band.right(),
        band.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    if enabled {
        clicked
    } else {
        None
    }
}

/// Paint the whole panel. `state.inspector` says what to paint and carries the caches.
pub fn show(ui: &mut egui::Ui, state: &mut UiState, tokens: &Tokens) -> Action {
    // The pane's bands are contiguous in the design — 64 + 40 + 28 with no gap — so the panel's
    // default `item_spacing` must not creep between them or the body lands below the 132 contract.
    ui.spacing_mut().item_spacing.y = 0.0;
    let inspector = &mut state.inspector;
    if inspector.selected.is_none() {
        // The design keeps the pane while nothing is selected: its header says so, the section switch
        // is greyed and inert, and the body explains where to pick an object. Hiding the pane instead
        // made the whole layout jump on the first click.
        let _ = header(ui, inspector, tokens);
        let options = Tab::ALL.map(|tab| (tab.label(), false));
        let _ = tabs_band(ui, tokens, &options, false);
        empty_pane(ui, tokens);
        return Action::None;
    }
    let action = if header(ui, inspector, tokens) {
        Action::OpenData
    } else {
        Action::None
    };

    let options = Tab::ALL.map(|tab| (tab.label(), tab == inspector.tab));
    if let Some(index) = tabs_band(ui, tokens, &options, true) {
        inspector.tab = Tab::ALL[index];
    }

    if inspector.tab == Tab::Preview {
        // The design puts the preview's page footer at the bottom of the pane — "5 of 12,481 rows"
        // describes the page, not the last row — so it takes its own band and the table scrolls
        // above it. A footer allocated after the rows would sit wherever the rows happen to end.
        let (shown, total) = match &inspector.preview {
            Some(Ok(result)) => (result.rows.len(), inspector.total_rows),
            _ => (0, None),
        };
        egui::Panel::bottom("inspector-preview-foot")
            .exact_size(design::ROW_H)
            .show_separator_line(false)
            .frame(design::bar_frame(tokens))
            .show(ui, |ui| preview_footer(ui, tokens, shown, total));
    }

    egui::ScrollArea::vertical()
        .id_salt("inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if inspector.tab == Tab::Preview {
                preview(ui, inspector, tokens);
            } else {
                let slot = inspector.tab.cache_slot().unwrap_or(0);
                let (model, error) = model(inspector.tab, inspector.cache[slot].as_ref());
                match error {
                    Some(error) => empty(ui, tokens, &error),
                    None => groups(ui, tokens, &model),
                }
            }
        });

    action
}

/// The `.ih` header: the glyph, the breadcrumb, the engine chip and the Open data button, then the
/// facts line. Returns whether the button was clicked.
fn header(ui: &mut egui::Ui, inspector: &Inspector, tokens: &Tokens) -> bool {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER_H), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, tokens.panel);
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(design::S5), rect.top()),
        egui::pos2(rect.right() - f32::from(design::S5), rect.bottom()),
    );
    // `.ih` centres its two lines; `.ih-top` is 24 tall and `.ih-meta` starts 4px below it.
    let content = 24.0 + f32::from(design::S2) + 13.0;
    let top = egui::Rect::from_min_size(
        egui::pos2(inner.left(), inner.top() + (HEADER_H - content) / 2.0),
        Vec2::new(inner.width(), 24.0),
    );

    let glyph = egui::Rect::from_min_size(top.min, Vec2::splat(design::GLYPH_BOX));
    icons::glyph(ui, glyph, Glyph::Table, tokens.text_dim);

    let (prefix, name) = match &inspector.selected {
        Some((_, reference)) => (
            reference
                .schema
                .as_ref()
                .map(|schema| format!("{schema}."))
                .unwrap_or_default(),
            reference.table.clone(),
        ),
        None => (String::new(), String::new()),
    };
    let font = tokens.mono(design::T_SM);
    let painter = ui.painter();
    let mut x = glyph.right() + f32::from(design::S3);
    for (text, colour) in [(prefix, tokens.text_faint), (name, tokens.text)] {
        if text.is_empty() {
            continue;
        }
        let width = painter
            .layout_no_wrap(text.clone(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x;
        painter.text(
            egui::pos2(x, top.center().y),
            Align2::LEFT_CENTER,
            &text,
            font.clone(),
            colour,
        );
        x += width;
    }

    // `.chip` in its plain ink: the same pill the rows use.
    let chip_text = inspector.engine.to_uppercase();
    let chip_width = painter
        .layout_no_wrap(
            chip_text.clone(),
            tokens.mono(design::T_XS),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
        + 2.0 * f32::from(design::S3)
        + 2.0;
    let chip = egui::Rect::from_min_size(
        egui::pos2(x + f32::from(design::S3), top.center().y - CHIP_H / 2.0),
        Vec2::new(chip_width, CHIP_H),
    );
    if chip.right() < inner.right() - 80.0 {
        paint_chip(ui, tokens, chip, &chip_text, TagKind::Plain);
    }

    let mut top_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(top)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let clicked = design::accent_button(&mut top_ui, tokens, "Open data", true)
        .on_hover_text("Open this object as a data tab")
        .clicked();

    ui.painter().text(
        egui::pos2(inner.left(), top.bottom() + f32::from(design::S2) + 6.5),
        Align2::LEFT_CENTER,
        &inspector.meta,
        tokens.font(design::T_XS),
        tokens.text_dim,
    );
    clicked
}

/// A tab's groups: the head, the optional note and the rows.
fn groups(ui: &mut egui::Ui, tokens: &Tokens, model: &Model) {
    // Guarding on every group being empty would swallow the note: the Foreign-keys model has one
    // group, whose whole content is the sentence explaining that there is nothing to list.
    if model.groups.is_empty() {
        empty(ui, tokens, "Nothing to show for this object.");
        return;
    }
    for group in &model.groups {
        if group.title.is_empty() {
            list_head(ui, tokens, &model.head, &model.legend);
        } else {
            group_head(ui, tokens, &group.title, group.count);
        }
        if let Some(note) = &group.note {
            note_line(ui, tokens, note);
        }
        for row in &group.rows {
            row_line(ui, tokens, row);
        }
    }
}

/// `.lhead`: an uppercase label with a legend on the right.
fn list_head(ui: &mut egui::Ui, tokens: &Tokens, label: &str, legend: &str) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), LIST_HEAD_H), Sense::hover());
    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.left() + f32::from(design::S5), rect.center().y),
        Align2::LEFT_CENTER,
        label.to_uppercase(),
        tokens.font(design::T_XS),
        tokens.text_dim,
    );
    painter.text(
        egui::pos2(rect.right() - f32::from(design::S5), rect.center().y),
        Align2::RIGHT_CENTER,
        legend,
        tokens.mono(design::T_XS),
        tokens.text_dim,
    );
    painter.hline(
        rect.left()..=rect.right(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
}

/// `.ghead`: a group's title with its count, on `surface-2`.
fn group_head(ui: &mut egui::Ui, tokens: &Tokens, title: &str, count: usize) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), LIST_HEAD_H), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, tokens.panel_alt);
    painter.text(
        egui::pos2(rect.left() + f32::from(design::S5), rect.center().y),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        tokens.font(design::T_XS),
        tokens.text_dim,
    );
    painter.text(
        egui::pos2(rect.right() - f32::from(design::S5), rect.center().y),
        Align2::RIGHT_CENTER,
        count.to_string(),
        tokens.mono(design::T_XS),
        tokens.text_dim,
    );
    painter.hline(
        rect.left()..=rect.right(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
}

/// `.gnote`: one line explaining why a group is empty.
fn note_line(ui: &mut egui::Ui, tokens: &Tokens, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), design::ROW_H),
        Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(rect.left() + f32::from(design::S5), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        tokens.font(design::T_XS),
        tokens.text_dim,
    );
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.grid_line),
    );
}

/// One `.crow`: the name and its chip on the first line, the facts on the second.
fn row_line(ui: &mut egui::Ui, tokens: &Tokens, row: &Row) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), CROW_H), Sense::hover());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, tokens.panel_alt);
    }
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(design::S5), rect.top()),
        egui::pos2(rect.right() - f32::from(design::S5), rect.bottom()),
    );
    // Two lines, 15px and 13px, centred in the row with the design's 2px between them.
    let name_y = rect.center().y - 7.0;
    let detail_y = rect.center().y + 7.0;
    let name = crate::table::clip(&row.name, 34);
    let name_font = if row.mono {
        tokens.mono(design::T_SM)
    } else {
        tokens.font(design::T_MD)
    };
    ui.painter().text(
        egui::pos2(inner.left(), name_y),
        Align2::LEFT_CENTER,
        name,
        name_font,
        tokens.text,
    );

    if let Some(tag) = &row.tag {
        let width = ui
            .painter()
            .layout_no_wrap(
                tag.text.clone(),
                tokens.mono(design::T_XS),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .x
            + 2.0 * f32::from(design::S3)
            + 2.0;
        let chip = egui::Rect::from_min_size(
            egui::pos2(inner.right() - width, name_y - CHIP_H / 2.0),
            Vec2::new(width, CHIP_H),
        );
        paint_chip(ui, tokens, chip, &tag.text, tag.kind);
    }

    // `.l2`: the facts, separated by the design's faint middot.
    let font = tokens.mono(design::T_XS);
    let mut x = inner.left();
    for (index, part) in row.detail.iter().enumerate() {
        let colour = match part.kind {
            PartKind::Plain => tokens.text_dim,
            PartKind::Null => tokens.text_faint,
            PartKind::Comment => tokens.text_dim,
        };
        if index > 0 {
            let dot = ui
                .painter()
                .layout_no_wrap("·".to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                .size()
                .x;
            ui.painter().text(
                egui::pos2(x + f32::from(design::S3), detail_y),
                Align2::LEFT_CENTER,
                "·",
                font.clone(),
                tokens.text_faint,
            );
            x += dot + 2.0 * f32::from(design::S3);
        }
        let text = crate::table::clip(&part.text, 30);
        let width = ui
            .painter()
            .layout_no_wrap(text.clone(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x;
        ui.painter().text(
            egui::pos2(x, detail_y),
            Align2::LEFT_CENTER,
            text,
            font.clone(),
            colour,
        );
        x += width;
    }
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.grid_line),
    );
}

/// The design's `.chip`: a 16px pill in one of the four inks.
fn paint_chip(ui: &egui::Ui, tokens: &Tokens, rect: egui::Rect, text: &str, kind: TagKind) {
    let (ink, border) = match kind {
        TagKind::Primary => (tokens.accent, tokens.accent_line),
        TagKind::Warn => (tokens.warn, tokens.stroke_strong),
        TagKind::Plain => (tokens.text_dim, tokens.stroke_strong),
    };
    let radius = CornerRadius::same(design::R_CHIP);
    let painter = ui.painter();
    painter.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text.to_uppercase(),
        tokens.mono(design::T_XS),
        ink,
    );
}

/// The design's empty pane: `.empty` in the inspector with the `.et` headline and the `.eb` sentence
/// telling the reader where an object is chosen.
fn empty_pane(ui: &mut egui::Ui, tokens: &Tokens) {
    ui.add_space(f32::from(design::S8));
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
        icons::glyph(ui, rect, Glyph::Table, tokens.text_faint);
        ui.add_space(f32::from(design::S4));
        ui.label(
            RichText::new("Nothing selected")
                .size(design::T_MD)
                .color(tokens.text),
        );
        ui.add_space(f32::from(design::S2));
        ui.label(
            RichText::new(
                "Choose a table or view in the connection tree to inspect its columns, indexes, \
                 foreign keys and rows.",
            )
            .size(design::T_SM)
            .color(tokens.text_dim),
        );
    });
}

/// The design's `.empty`: a faint glyph and a line saying why there is nothing here.
fn empty(ui: &mut egui::Ui, tokens: &Tokens, text: &str) {
    ui.add_space(f32::from(design::S8));
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
        icons::glyph(ui, rect, Glyph::Table, tokens.text_faint);
        ui.add_space(f32::from(design::S3));
        ui.label(
            RichText::new(text)
                .size(design::T_XS)
                .color(tokens.text_dim),
        );
    });
}

/// The Preview tab: the object's first rows in the shared read-only grid, with the design's footer.
fn preview(ui: &mut egui::Ui, inspector: &mut Inspector, tokens: &Tokens) {
    let Inspector {
        preview,
        preview_state,
        ..
    } = inspector;
    let rows = match preview {
        None => {
            empty(ui, tokens, "Fetching the first rows…");
            return;
        }
        Some(Err(error)) => {
            empty(ui, tokens, error);
            return;
        }
        Some(Ok(result)) => result,
    };
    let columns = rows
        .columns
        .iter()
        .zip(rows.column_types.iter())
        .map(|(name, kind)| TableColumn::new(name.clone(), kind.clone()))
        .collect::<Vec<_>>();
    // The table fills the body the footer left. `views::list` reports its own `HEAD_H` on top of the
    // height it is given, so the body's height is what is asked for, minus that head.
    let body = ui.clip_rect().height();
    views::list(
        ui,
        tokens,
        List {
            columns: &columns,
            rows: &rows.rows,
            height: (body - design::HEAD_H).max(design::ROW_H),
            id: egui::Id::new("inspector-preview"),
            empty: "This object has no rows.",
            gutter: false,
        },
        preview_state,
    );
}

/// `.pfoot`: how much of the object this page shows, and that nothing here writes.
fn preview_footer(ui: &egui::Ui, tokens: &Tokens, shown: usize, total: Option<u64>) {
    let rect = ui.max_rect();
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.top() + 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    let left = match total {
        Some(total) => format!("{shown} of {} rows", crate::table::grouped(total)),
        None => format!(
            "{shown} {}",
            crate::table::plural(shown as u64, "row", "rows")
        ),
    };
    ui.painter().text(
        egui::pos2(rect.left() + f32::from(design::S5), rect.center().y),
        Align2::LEFT_CENTER,
        left,
        tokens.font(design::T_XS),
        tokens.text_dim,
    );
    ui.painter().text(
        egui::pos2(rect.right() - f32::from(design::S5), rect.center().y),
        Align2::RIGHT_CENTER,
        "read-only",
        tokens.mono(design::T_XS),
        tokens.text_faint,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_tab_has_its_command_and_the_preview_has_none() {
        assert_eq!(
            Tab::ALL.map(Tab::label),
            ["Columns", "Indexes", "Foreign keys", "Preview"]
        );
        assert_eq!(Tab::Columns.command(), Some("schema.columns"));
        assert_eq!(Tab::Indexes.command(), Some("schema.indexes"));
        assert_eq!(Tab::Keys.command(), Some("schema.foreignKeys"));
        assert_eq!(
            Tab::Preview.command(),
            None,
            "the preview is a page of rows, not a schema command"
        );
        // Every schema tab needs a cache slot, and only those.
        for tab in Tab::ALL {
            assert_eq!(tab.command().is_some(), tab.cache_slot().is_some());
        }
        assert_eq!(Tab::default(), Tab::Columns);
    }

    #[test]
    fn the_meta_line_counts_rows_size_and_columns() {
        assert_eq!(
            meta(Some(12_481), Some(1_887_437), 7),
            "12,481 rows · 1.8 MB · 7 columns"
        );
        assert_eq!(meta(Some(1), Some(512), 1), "1 row · 512 B · 1 column");
        assert_eq!(meta(Some(0), Some(0), 0), "0 rows · 0 B · 0 columns");
        assert_eq!(
            meta(Some(2), Some(5_368_709_120), 3),
            "2 rows · 5.0 GB · 3 columns"
        );
        // A catalog that reports neither estimate still knows how many columns there are.
        assert_eq!(meta(None, None, 7), "7 columns");
    }

    #[test]
    fn columns_become_rows_with_chips_and_facts() {
        let model = columns(&json!({
            "table": "main.customers",
            "columns": [
                {"name":"id","type":"INTEGER","nullable":false,"primary_key":true,
                 "auto_increment":true,"default":null,"comment":""},
                {"name":"plan","type":"TEXT","nullable":false,"primary_key":false,
                 "auto_increment":false,"default":"'starter'","comment":"billing tier"},
                {"name":"lifetime_value","type":"NUMERIC","nullable":true,"primary_key":false,
                 "auto_increment":false,"default":"0.00","comment":""}
            ]
        }));
        assert_eq!(model.head, "Column");
        assert_eq!(model.legend, "NN not null · NULL nullable");
        let rows = &model.groups[0].rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "id");
        assert_eq!(rows[0].tag.as_ref().map(|t| t.kind), Some(TagKind::Primary));
        assert_eq!(
            rows[0]
                .detail
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            ["INTEGER", "NN", "rowid alias"],
            "a rowid alias says so, and a null default says nothing"
        );
        assert_eq!(
            rows[1]
                .detail
                .iter()
                .map(|p| (p.text.as_str(), p.kind))
                .collect::<Vec<_>>(),
            [
                ("TEXT", PartKind::Plain),
                ("NN", PartKind::Plain),
                ("'starter'", PartKind::Plain),
                ("billing tier", PartKind::Comment),
            ]
        );
        assert_eq!(rows[2].detail[1].kind, PartKind::Null);
        assert!(rows[1].tag.is_none(), "only a key earns a chip");
    }

    #[test]
    fn indexes_become_rows_with_their_columns_and_method() {
        let model = indexes(&json!({
            "indexes": [
                {"name":"sqlite_autoindex_customers_1","columns":["id ASC"],"unique":true,
                 "primary":true,"kind":"B-tree"},
                {"name":"idx_customers_email","columns":["email ASC"],"unique":true,
                 "primary":false,"kind":"B-tree"},
                {"name":"idx_customers_status_signup","columns":["status ASC","signup_date DESC"],
                 "unique":false,"primary":false,"kind":"B-tree"}
            ]
        }));
        assert_eq!(model.head, "Index");
        assert_eq!(model.legend, "3 defined");
        let rows = &model.groups[0].rows;
        assert!(rows[0].mono, "an index name is an identifier");
        assert_eq!(
            rows[0].tag.as_ref().map(|t| (t.text.as_str(), t.kind)),
            Some(("PRIMARY", TagKind::Primary))
        );
        assert_eq!(
            rows[1].tag.as_ref().map(|t| t.text.as_str()),
            Some("UNIQUE")
        );
        assert!(rows[2].tag.is_none(), "a plain index has no chip");
        assert_eq!(
            rows[2]
                .detail
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            ["status ASC, signup_date DESC", "B-tree"]
        );
    }

    #[test]
    fn foreign_keys_name_the_column_that_points_and_where_it_points() {
        let model = keys(&json!({
            "table": "main.orders",
            "foreign_keys": [
                {"name":"orders_customer_id_fkey","columns":["customer_id"],
                 "ref_schema":"main","ref_table":"customers","ref_columns":["id"],
                 "on_delete":"SET NULL","on_update":"NO ACTION"}
            ]
        }));
        let group = &model.groups[0];
        assert_eq!(group.title, "Outgoing");
        assert_eq!(group.count, 1);
        assert!(group.note.is_none());
        assert_eq!(group.rows[0].name, "orders.customer_id");
        assert!(group.rows[0].mono);
        assert_eq!(
            group.rows[0]
                .tag
                .as_ref()
                .map(|t| (t.text.as_str(), t.kind)),
            Some(("ON DELETE SET NULL", TagKind::Warn))
        );
        assert_eq!(
            group.rows[0]
                .detail
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            ["main.customers.id", "ON UPDATE NO ACTION"]
        );
    }

    #[test]
    fn an_object_without_keys_says_so_instead_of_showing_nothing() {
        let model = keys(&json!({"table": "main.customers", "foreign_keys": []}));
        assert_eq!(model.legend, "0 defined");
        assert!(model.groups[0].rows.is_empty());
        // The design's `.gnote` names the object, so the sentence has to carry the qualified name.
        assert_eq!(
            model.groups[0].note.as_deref(),
            Some("main.customers defines no foreign keys.")
        );
    }

    #[test]
    fn a_failed_fetch_becomes_a_message_rather_than_an_empty_list() {
        let (view, error) = model(Tab::Columns, Some(&Err("no such table".into())));
        assert_eq!(error.as_deref(), Some("no such table"));
        assert!(view.groups.is_empty());
        let (view, error) = model(Tab::Columns, None);
        assert!(error.is_none() && view.groups.is_empty());
    }
}
