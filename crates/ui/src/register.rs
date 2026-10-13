//! The results register: one tab per statement of the last run, the status bar over the active
//! statement's result, the read-only grid or its JSON view, and the footer — with the states that
//! have nothing to show (nothing has run, a zero-row completion, a failed statement, a void tab).
//!
//! The register is a view of what the engine recorded, never of the editor: it reads
//! `SqlState::results`, one `StatementRun` per statement, so every tab carries its own rows, its own
//! duration and its own failure. Everything it paints is built from those runs — the pill's copy,
//! the CSV the Copy button writes and the JSON the JSON view shows.

use datacraft_engine::{Session, SqlState, StatementRun};
use datacraft_model::ResultSet;
use egui::{Align2, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::app::UiState;
use crate::icons::Glyph;
use crate::table::{self, TableColumn, TableEvent, TableState, TableView};
use crate::theme::{self as design, Emphasis, Mark, Tokens};

/// What the register shows before anything has run — the editor's own invitation, so an empty query
/// tab says one thing in one wording rather than two.
pub const INVITATION: &str = "Write a statement and press Run, or Cmd+Enter.";

/// `.rbar`'s second pill: the script changed after the run that filled this register, so what is
/// on screen is no longer what the editor would run.
pub const STALE_NOTE: &str = "stale · the script changed since this run";

/// `.rfoot`'s right-hand pill: every result the register paints is read-only.
const READ_ONLY: &str = "read-only result";
/// `.rfoot`'s mono hint in the grid view.
const COPY_HINT: &str = "⌘C copies the selection";
/// `.rfoot`'s mono hint in the JSON view.
const JSON_HINT: &str = "same values as the grid view · read-only";
/// `.void`'s sentence, on a tab whose statement produced nothing.
const VOID_NOTE: &str = "no result set for this statement";
/// `.empty`'s heading.
const EMPTY_TITLE: &str = "No rows returned";
/// `.rfoot`'s height. The design sizes it as `--h-head`, the same 28 as the grid's head.
const FOOTER_H: f32 = design::HEAD_H;
/// `.jb pre`'s `line-height: 20px`, which is also the pitch the gutter's numbers are drawn on.
const JSON_ROW_H: f32 = design::EDITOR_ROW_H;

/// Which view of the active result the register shows.
///
/// The design's segment offers `Grid / JSON / Text`; `Text` is not shipped because no markup, note
/// or prose in the mockup defines what it renders — the ceiling is recorded in `docs/architecture.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Grid,
    Json,
}

impl View {
    fn label(self) -> &'static str {
        match self {
            View::Grid => "Grid",
            View::Json => "JSON",
        }
    }

    fn copy_label(self) -> &'static str {
        match self {
            View::Grid => "Copy",
            View::Json => "Copy JSON",
        }
    }

    fn all() -> [View; 2] {
        [View::Grid, View::Json]
    }
}

/// The register's own state, one per query tab: which statement is on screen, in which view, and the
/// script the results came from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Register {
    /// The statement whose result is on screen.
    pub active: usize,
    pub view: View,
    /// The editor's text at the run that filled this register. [`Register::stale`] compares it with
    /// the live text, so the register can say its results no longer match the editor.
    pub script: String,
}

impl Register {
    /// The register for a tab's SQL state, given the previous frame's register.
    ///
    /// Call this when the run has changed — after `sql.run`, or when the tab has no register yet.
    /// `script` records the text those results came from, so calling it every frame would erase what
    /// [`Register::stale`] is for.
    ///
    /// The active tab defaults to the statement that failed, because that is the one the user has to
    /// read, and to the last statement otherwise; a previous register keeps its own selection while
    /// that index still names a statement, so re-running a script does not move the tab under the
    /// pointer. The view is the previous one.
    pub fn from(state: &SqlState, previous: Option<&Register>) -> Register {
        let count = state.results.len();
        let failed = state.results.iter().rposition(|run| run.error.is_some());
        let fallback = failed.unwrap_or_else(|| count.saturating_sub(1));
        let active = match previous {
            Some(previous) if previous.active < count => previous.active,
            _ => fallback,
        };
        Register {
            active,
            view: previous.map(|previous| previous.view).unwrap_or_default(),
            script: state.text.clone(),
        }
    }

    /// True when the editor's text has moved on since the run that filled this register.
    pub fn stale(&self, script: &str) -> bool {
        self.script != script
    }

    /// What the Copy button puts on the clipboard: the active view's own text — the grid's CSV, or
    /// the JSON text. Both are the engine's serialisation of the same `ResultSet`, and a statement
    /// with no result set has nothing to copy.
    pub fn clipboard(&self, state: &SqlState) -> String {
        let Some(result) = state
            .results
            .get(self.active)
            .and_then(|run| run.result.as_ref())
        else {
            return String::new();
        };
        match self.view {
            View::Grid => result.to_csv(),
            View::Json => json_text(result),
        }
    }

    /// `.rbar .note`'s sentence: which statement this is, and what its result is. The statement
    /// count appears only when the run held more than one, so a single statement says what it has.
    ///
    /// `max_rows` is the preference every run is bounded by, which is the limit a cut page was cut at.
    pub fn note(&self, state: &SqlState, max_rows: usize) -> String {
        let prefix = statement_prefix(self.active, state.results.len());
        let suffix = match state
            .results
            .get(self.active)
            .and_then(|run| run.result.as_ref())
        {
            Some(result) if result.truncated => format!("bounded page · LIMIT {max_rows}"),
            Some(_) => "complete result".to_owned(),
            None => "no result set".to_owned(),
        };
        format!("{prefix}{suffix}")
    }

    /// `.rtabs .tot`: what the whole run cost. One statement's time is the whole run's, so the
    /// singular count drops the design's `total` — the same wording the mockup prints.
    pub fn total_note(state: &SqlState) -> String {
        let ms = millis(state.total_ms());
        match state.results.len() {
            1 => format!("1 statement · {ms} ms"),
            count => format!("{count} statements · {ms} ms total"),
        }
    }
}

/// `.jb`'s text and the JSON view's clipboard payload: the engine's own serialisation of the result,
/// the same one the `json` export format writes (`crates/engine/src/cmd/grid.rs`), so the two can
/// never describe the same rows differently.
pub fn json_text(result: &ResultSet) -> String {
    serde_json::to_string_pretty(&result.to_json()).unwrap_or_default()
}

/// Paint the register into the rest of `ui`, reading the tab's SQL state and dispatching what the
/// user asked for through the command registry.
pub fn show(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    register: &mut Register,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let Some(tab) = session.tabs.get(tab_index) else {
        return;
    };
    let density = session.settings.density;
    let max_rows = session.settings.max_rows;
    let id = egui::Id::new(("register", tab_index));

    let mut actions = Actions::default();
    let sql = &tab.sql;
    let _ = paint(
        ui,
        tokens,
        register,
        sql,
        density,
        max_rows,
        id,
        state.table(tab_index),
        &mut actions,
    );

    if let Some(text) = actions.copy {
        ui.ctx().copy_text(text);
    }
    if actions.export {
        state.open_export_grid("csv");
    }
    if actions.rerun {
        let text = session.tabs[tab_index].sql.text.clone();
        let mut params = serde_json::json!({ "sql": text });
        if let Some(conn) = session.tabs[tab_index].conn {
            params["conn"] = serde_json::json!(session.conns[conn].cfg.id);
        }
        if let Err(e) = session.run("sql.run", params) {
            session.log(datacraft_engine::Level::Error, format!("sql.run: {e}"));
        }
    }
}

/// What one painted frame asked the caller to do. The widget never touches the session or the
/// clipboard itself, so a test can drive it headlessly and read the request back.
#[derive(Debug, Clone, Default, PartialEq)]
struct Actions {
    /// Put this on the clipboard.
    copy: Option<String>,
    /// Open the export dialog for the active result.
    export: bool,
    /// Run the tab's script again.
    rerun: bool,
}

/// The bands one frame painted, so the caller — and the tests below — can measure the stack.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Layout {
    tabs: Option<Rect>,
    bar: Option<Rect>,
    grid: Option<Rect>,
    empty: Option<Rect>,
    error: Option<Rect>,
    void: Option<Rect>,
    json: Option<Rect>,
    footer: Option<Rect>,
    /// The row pitch the grid widget was handed, from the density preference.
    row_h: f32,
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &mut Ui,
    tokens: &Tokens,
    register: &mut Register,
    sql: &SqlState,
    density: datacraft_engine::Density,
    max_rows: usize,
    id: egui::Id,
    table: &mut TableState,
    actions: &mut Actions,
) -> Layout {
    // The design stacks its bands with no gap between them — each one's hairline is the next one's
    // top edge — so the pane's stray default spacing must not add one. Same as `grid::show`.
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut marks = Layout::default();

    if sql.results.is_empty() {
        invitation(ui, tokens);
        return marks;
    }
    // A re-run that produced fewer statements must not leave the register pointing past its own
    // tabs: the active index is clamped to the run in hand, which is also what `from` defaults to.
    register.active = register.active.min(sql.results.len() - 1);
    let run = &sql.results[register.active];
    let row_h = design::row_h(density);
    marks.row_h = row_h;

    marks.tabs = Some(tabs(ui, tokens, sql, register));
    marks.bar = Some(status_bar(
        ui, tokens, register, sql, run, max_rows, actions,
    ));

    match (run.result.as_ref(), run.error.as_deref()) {
        // A statement that produced nothing: the driver's own words, then the void. This is the
        // state a failed tab opens on.
        (None, error) => {
            if let Some(error) = error {
                marks.error = Some(error_band(ui, tokens, run, error, actions));
            }
            marks.void = Some(void(ui, tokens));
        }
        (Some(result), _) => match register.view {
            View::Json => {
                let rows = result.row_count();
                let height = if rows == 0 {
                    ui.available_height()
                } else {
                    (ui.available_height() - FOOTER_H).max(JSON_ROW_H)
                };
                marks.json = Some(json_view(ui, tokens, &json_text(result), height, id));
                if rows > 0 {
                    marks.footer = Some(footer(ui, tokens, View::Json, result, table.selected));
                }
            }
            // The design keeps the columns of a zero-row result so the layout does not jump: the
            // head stays, and the block below it says what the engine reported.
            View::Grid if result.rows.is_empty() => {
                if !result.columns.is_empty() {
                    marks.grid = Some(grid(ui, tokens, result, row_h, table, id, 0.0));
                }
                marks.empty = Some(empty_block(ui, tokens, run, result, actions));
            }
            View::Grid => {
                // The widget spends the head above the body it is handed, and the footer below it:
                // both come out of what is left, or the footer lands past the pane's last row.
                let height = (ui.available_height() - FOOTER_H - design::HEAD_H).max(row_h);
                marks.grid = Some(grid(ui, tokens, result, row_h, table, id, height));
                marks.footer = Some(footer(ui, tokens, View::Grid, result, table.selected));
            }
        },
    }
    marks
}

/// `.rtabs`: one tab per statement, the run's total pinned to the right, and the `stroke` hairline
/// the register below it starts from.
fn tabs(ui: &mut Ui, tokens: &Tokens, sql: &SqlState, register: &mut Register) -> Rect {
    let rect = design::band_fill(
        ui,
        design::REG_TABS_H,
        tokens.panel_alt,
        design::S4,
        egui::Align::Center,
        |ui| {
            for (index, run) in sql.results.iter().enumerate() {
                tab(ui, tokens, index, run, register);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(Register::total_note(sql))
                        .font(tokens.mono(design::T_XS))
                        .color(tokens.text_dim),
                );
            });
        },
    );
    design::band_hairline(ui, tokens, rect);
    rect
}

/// One `.rtab`: the statement's ordinal, its own first word, and its own rows and duration. The
/// active tab is filled with `panel` and carries the design's 2px `accent` bar along its **top**
/// edge — unlike the window tab strip, whose indicator is at the bottom.
fn tab(ui: &mut Ui, tokens: &Tokens, index: usize, run: &StatementRun, register: &mut Register) {
    let active = register.active == index;
    let ordinal = (index + 1).to_string();
    let label = statement_label(&run.statement);
    let facts = tab_facts(run);
    let mono = tokens.mono(design::T_XS);
    let width = f32::from(design::S5)
        + measured(ui, mono.clone(), &ordinal)
        + f32::from(design::S4)
        + measured(ui, tokens.font(design::T_SM), &label)
        + f32::from(design::S4)
        + measured(ui, mono.clone(), &facts)
        + f32::from(design::S4);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, design::TAB_H), Sense::click());
    {
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, CornerRadius::ZERO, tokens.panel);
            painter.rect_filled(
                Rect::from_min_max(
                    rect.left_top(),
                    egui::pos2(rect.right(), rect.top() + design::INDICATOR_W),
                ),
                CornerRadius::ZERO,
                tokens.accent,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, tokens.hover);
        }
        painter.vline(
            rect.right() - 0.5,
            rect.top()..=rect.bottom(),
            Stroke::new(1.0, tokens.stroke),
        );
        let failed = run.error.is_some();
        let ink = if failed {
            tokens.error
        } else if active {
            tokens.text
        } else {
            tokens.text_dim
        };
        let mut x = rect.left() + f32::from(design::S5);
        for (text, font, colour) in [
            (
                ordinal.as_str(),
                mono.clone(),
                if failed {
                    tokens.error
                } else {
                    tokens.text_dim
                },
            ),
            (label.as_str(), tokens.font(design::T_SM), ink),
            (facts.as_str(), mono.clone(), ink),
        ] {
            painter.text(
                egui::pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font.clone(),
                colour,
            );
            x += measured(ui, font, text) + f32::from(design::S4);
        }
    }
    if response.clicked() {
        register.active = index;
    }
}

/// `.rbar`: the active statement's status line and its bound, the view switch, and the two actions
/// that read the result — all of them facts the engine produced.
#[allow(clippy::too_many_arguments)]
fn status_bar(
    ui: &mut Ui,
    tokens: &Tokens,
    register: &mut Register,
    sql: &SqlState,
    run: &StatementRun,
    max_rows: usize,
    actions: &mut Actions,
) -> Rect {
    let ink = if run.error.is_some() {
        tokens.error
    } else {
        tokens.text
    };
    let rect = design::band_fill(
        ui,
        design::TOOLBAR_H,
        tokens.panel,
        design::S5,
        egui::Align::Center,
        |ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S5);
            ui.label(RichText::new(stat_text(run)).size(design::T_SM).color(ink));
            design::note_pill(ui, tokens, &register.note(sql, max_rows), Some(tokens.info));
            if register.stale(&sql.text) {
                design::note_pill(ui, tokens, STALE_NOTE, Some(tokens.warn));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = f32::from(design::S5);
                if design::toolbar_button(
                    ui,
                    tokens,
                    "Export",
                    Some(Mark::Glyph(Glyph::Export)),
                    tokens.panel_alt,
                    Emphasis::Plain,
                    true,
                    |_| {},
                )
                .on_hover_text("Export this result")
                .clicked()
                {
                    actions.export = true;
                }
                let copyable = run.result.is_some();
                if design::toolbar_button(
                    ui,
                    tokens,
                    register.view.copy_label(),
                    Some(Mark::Glyph(Glyph::Copy)),
                    tokens.panel_alt,
                    Emphasis::Plain,
                    copyable,
                    |_| {},
                )
                .on_hover_text("Copy this view as text")
                .clicked()
                {
                    actions.copy = Some(register.clipboard(sql));
                }
                let labels: Vec<(&str, bool)> = View::all()
                    .iter()
                    .map(|view| (view.label(), *view == register.view))
                    .collect();
                let width = segment_width(ui, tokens, &labels);
                if let Some(index) = design::pane_segmented(
                    ui,
                    tokens,
                    &labels,
                    width,
                    design::SEG_H,
                    2.0,
                    tokens.text,
                ) {
                    register.view = View::all()[index];
                }
            });
        },
    );
    design::band_hairline(ui, tokens, rect);
    rect
}

/// `.eband`: the driver's code, its message and the statement that failed, between the status bar
/// and the void. `Retry` runs the script again; `Copy error` writes the whole message out.
fn error_band(
    ui: &mut Ui,
    tokens: &Tokens,
    run: &StatementRun,
    error: &str,
    actions: &mut Actions,
) -> Rect {
    let (tag, message) = split_error(error);
    let mut retry = false;
    let rect = design::register_error_band(ui, tokens, tag, message, &run.statement, |ui| {
        ui.spacing_mut().item_spacing.x = f32::from(design::S4);
        if design::ghost_button(ui, tokens, "Copy error", true)
            .on_hover_text("Copy the driver's message and the statement")
            .clicked()
        {
            actions.copy = Some(error_text(error, &run.statement));
        }
        if design::ghost_button(ui, tokens, "Retry", true)
            .on_hover_text("Run the script again")
            .clicked()
        {
            retry = true;
        }
    });
    actions.rerun |= retry;
    rect
}

/// The grid itself: P4's widget, read-only. A result set is not a table — nothing behind it can be
/// written back — so neither the inline editor nor the header's sort is offered.
fn grid(
    ui: &mut Ui,
    tokens: &Tokens,
    result: &ResultSet,
    row_h: f32,
    table: &mut TableState,
    id: egui::Id,
    height: f32,
) -> Rect {
    let columns: Vec<TableColumn> = result
        .columns
        .iter()
        .enumerate()
        .map(|(index, name)| {
            TableColumn::new(
                name.clone(),
                result.column_types.get(index).cloned().unwrap_or_default(),
            )
        })
        .collect();
    let view = TableView {
        columns: &columns,
        rows: &result.rows,
        pending: Vec::new(),
        inserted: Vec::new(),
        selected: table.selected,
        sort: None,
        editable: false,
        sortable: false,
        gutter: true,
        height,
        row_h,
        empty_text: "no rows",
    };
    let before = ui.cursor();
    if let TableEvent::Select(row, column) = table::show(ui, tokens, &view, table, id) {
        table.selected = Some((row, column));
    }
    let after = ui.cursor().top();
    Rect::from_min_max(
        egui::pos2(before.left(), before.top()),
        egui::pos2(before.right(), after),
    )
}

/// `.rfoot`: only a result with rows has one — the zero-row and void states have no footer, which is
/// what the design's sheets show (6 footers against 10 frames).
fn footer(
    ui: &mut Ui,
    tokens: &Tokens,
    view: View,
    result: &ResultSet,
    selected: Option<(usize, usize)>,
) -> Rect {
    let rect = design::band_fill(
        ui,
        FOOTER_H,
        tokens.panel_alt,
        design::S5,
        egui::Align::Center,
        |ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S4);
            ui.label(
                RichText::new(rows_note(result.row_count()))
                    .size(design::T_XS)
                    .color(tokens.text_dim),
            );
            match view {
                View::Grid => {
                    ui.label(
                        RichText::new(selection_note(usize::from(selected.is_some())))
                            .size(design::T_XS)
                            .color(tokens.text_dim),
                    );
                    ui.label(
                        RichText::new(COPY_HINT)
                            .font(tokens.mono(design::T_XS))
                            .color(tokens.text_dim),
                    );
                }
                View::Json => {
                    ui.label(
                        RichText::new(JSON_HINT)
                            .font(tokens.mono(design::T_XS))
                            .color(tokens.text_dim),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                design::note_pill(ui, tokens, READ_ONLY, Some(tokens.text_faint));
            });
        },
    );
    design::band_top_hairline(ui, tokens, rect);
    rect
}

/// `.empty`, under the head of a result that came back with columns and no rows. The block is the
/// design's 196 tall with P4's padding, so a zero-row run shifts nothing else on the pane.
fn empty_block(
    ui: &mut Ui,
    tokens: &Tokens,
    run: &StatementRun,
    result: &ResultSet,
    actions: &mut Actions,
) -> Rect {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, design::EMPTY_BLOCK_H), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, tokens.panel);
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + 20.0, rect.top() + 28.0),
        egui::pos2(rect.right() - 20.0, rect.bottom() - 30.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = f32::from(design::S3);
    let (title, body) = empty_copy(run, result);
    child.label(RichText::new(title).size(design::T_MD).color(tokens.text));
    child.label(
        RichText::new(body)
            .size(design::T_SM)
            .color(tokens.text_dim),
    );
    child.add_space(f32::from(design::S2));
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = f32::from(design::S3);
        if design::ghost_button(ui, tokens, "Copy query", true)
            .on_hover_text("Copy the statement to the clipboard")
            .clicked()
        {
            actions.copy = Some(run.statement.clone());
        }
        if design::ghost_button(ui, tokens, "Re-run", true)
            .on_hover_text("Run the script again")
            .clicked()
        {
            actions.rerun = true;
        }
    });
    rect
}

/// `.void`: the rest of the register, saying there is no result set for the statement on screen.
fn void(ui: &mut Ui, tokens: &Tokens) -> Rect {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ui.available_height()),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, tokens.panel);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        VOID_NOTE,
        tokens.mono(design::T_XS),
        tokens.text_dim,
    );
    rect
}

/// `.jb`: the JSON text beside its own line-number gutter, under a `ScrollArea` so a long value is
/// reachable rather than clipped.
fn json_view(ui: &mut Ui, tokens: &Tokens, text: &str, height: f32, id: egui::Id) -> Rect {
    let code = ui.painter().layout_job(json_job(tokens, text));
    let lines = code.rows.len().max(1);
    let numbers: String = (1..=lines).map(|line| format!("{line}\n")).collect();
    let mut numbers_job = egui::text::LayoutJob::simple(
        numbers,
        tokens.mono(design::T_XS),
        tokens.text_dim,
        f32::INFINITY,
    );
    numbers_job.halign = egui::Align::RIGHT;
    if let Some(section) = numbers_job.sections.first_mut() {
        section.format.line_height = Some(JSON_ROW_H);
    }
    let numbers = ui.painter().layout_job(numbers_job);

    let out = egui::ScrollArea::both()
        .id_salt(id.with("json"))
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let content = Vec2::new(
                (design::JSON_GUTTER_W
                    + f32::from(design::S6)
                    + code.size().x
                    + f32::from(design::S5))
                .max(ui.available_width()),
                lines as f32 * JSON_ROW_H + 2.0 * f32::from(design::S5),
            );
            let (rect, _) = ui.allocate_exact_size(content, Sense::hover());
            let gutter = design::JSON_GUTTER_W;
            let pad = f32::from(design::S5);
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::ZERO, tokens.panel);
            painter.galley(
                egui::pos2(rect.left() + gutter - pad, rect.top() + pad),
                numbers,
                tokens.text_dim,
            );
            painter.vline(
                rect.left() + gutter,
                rect.top()..=rect.bottom(),
                Stroke::new(1.0, tokens.stroke),
            );
            painter.galley(
                egui::pos2(
                    rect.left() + gutter + f32::from(design::S6),
                    rect.top() + pad,
                ),
                code,
                tokens.text,
            );
            rect
        });
    // The content, not the viewport: the band's own height is what its lines come to.
    out.inner
}

/// `.jb`'s text as one job: a 20px line pitch, and the design's inks — keys and `null` in
/// `text_dim` (`null` also italic), punctuation in `text_faint`, values in `text`.
fn json_job(tokens: &Tokens, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    for (run, ink) in json_runs(text) {
        job.append(
            &run,
            0.0,
            egui::TextFormat {
                font_id: tokens.mono(design::T_SM),
                color: match ink {
                    Ink::Key | Ink::Null => tokens.text_dim,
                    Ink::Punct => tokens.text_faint,
                    Ink::Value => tokens.text,
                },
                italics: ink == Ink::Null,
                line_height: Some(JSON_ROW_H),
                ..Default::default()
            },
        );
    }
    job
}

/// What one run of JSON text is, which is all the JSON view needs to ink it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    /// A quoted key: a string with a colon after it.
    Key,
    /// `{`, `}`, `[`, `]`, `,` and `:`.
    Punct,
    /// The `null` literal.
    Null,
    /// A number, a boolean, or a string used as a value.
    Value,
}

/// The JSON text in the runs the design inks, each carrying the whitespace that precedes it so the
/// pretty-printed layout survives. One pass, one look-ahead: whether a string is followed by a colon.
fn json_runs(text: &str) -> Vec<(String, Ink)> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<(String, Ink)> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() {
            push_run(&mut runs, chars[start..].iter().collect(), Ink::Value);
            break;
        }
        let (ink, end) = if chars[index] == '"' {
            let mut end = index + 1;
            while end < chars.len() {
                match chars[end] {
                    '\\' => end += 2,
                    '"' => {
                        end += 1;
                        break;
                    }
                    _ => end += 1,
                }
            }
            let is_key = chars[end.min(chars.len())..]
                .iter()
                .find(|ch| !ch.is_whitespace())
                .copied()
                == Some(':');
            (
                if is_key { Ink::Key } else { Ink::Value },
                end.min(chars.len()),
            )
        } else if "{}[],:".contains(chars[index]) {
            (Ink::Punct, index + 1)
        } else {
            let mut end = index;
            while end < chars.len() && !"{}[],:".contains(chars[end]) && !chars[end].is_whitespace()
            {
                end += 1;
            }
            let token: String = chars[index..end].iter().collect();
            (
                if token == "null" {
                    Ink::Null
                } else {
                    Ink::Value
                },
                end,
            )
        };
        push_run(&mut runs, chars[start..end].iter().collect(), ink);
        index = end.max(index + 1);
    }
    runs
}

/// Append a run, merging it into the previous one when the ink matches — fewer sections to lay out.
fn push_run(runs: &mut Vec<(String, Ink)>, text: String, ink: Ink) {
    match runs.last_mut() {
        Some((last, last_ink)) if *last_ink == ink => last.push_str(&text),
        _ => runs.push((text, ink)),
    }
}

/// The address of nothing has run yet — the editor's own invitation, centred where the register
/// would be. `Register::from` on an empty `SqlState` reaches this state and paints no bands at all.
fn invitation(ui: &mut Ui, tokens: &Tokens) -> Rect {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        INVITATION,
        tokens.font(design::T_SM),
        tokens.text_faint,
    );
    rect
}

/// `.rbar .stat`: the active statement's own facts, `failed` replacing the count when it did not
/// complete. The design's `· main.customers` is dropped — the register holds a result set, not the
/// table a query read, and the tree names the table.
fn stat_text(run: &StatementRun) -> String {
    let ms = millis(run.elapsed_ms);
    if run.error.is_some() {
        return format!("failed · {ms} ms");
    }
    let Some(result) = run.result.as_ref() else {
        return format!("no result · {ms} ms");
    };
    if result.columns.is_empty() {
        return format!("{} affected · {ms} ms", affected_note(result.affected));
    }
    let mut text = format!(
        "{} rows · {ms} ms",
        table::grouped(result.row_count() as u64)
    );
    if result.affected > 0 {
        text.push_str(&format!(" · {} affected", result.affected));
    }
    text
}

/// `.rtab .mt`: the same facts in the tab's own wording, where a statement with no columns reports
/// what it affected rather than how many rows it returned.
fn tab_facts(run: &StatementRun) -> String {
    let ms = millis(run.elapsed_ms);
    if run.error.is_some() {
        return format!("error · {ms} ms");
    }
    match run.result.as_ref() {
        Some(result) if result.columns.is_empty() => {
            format!("{} affected · {ms} ms", affected_note(result.affected))
        }
        Some(result) => format!(
            "{} rows · {ms} ms",
            table::grouped(result.row_count() as u64)
        ),
        None => format!("not run · {ms} ms"),
    }
}

/// `.empty`'s heading and paragraph: what the engine reported, in the run's own numbers.
fn empty_copy(run: &StatementRun, result: &ResultSet) -> (String, String) {
    let ms = millis(run.elapsed_ms);
    let body = if result.columns.is_empty() {
        format!(
            "The statement completed in {ms} ms and affected {}.",
            affected_note(result.affected)
        )
    } else {
        format!("The statement completed in {ms} ms and matched 0 rows.")
    };
    (EMPTY_TITLE.to_owned(), body)
}

/// `2 statements · 14 ms total`'s prefix, left off a run of one statement: with a single statement
/// the tab already says which one it is.
fn statement_prefix(index: usize, count: usize) -> String {
    if count > 1 {
        format!("statement {} of {count} · ", index + 1)
    } else {
        String::new()
    }
}

/// `.rfoot`'s left-hand range: the rows the register is holding.
fn rows_note(count: usize) -> String {
    match count {
        0 => "No rows".to_owned(),
        1 => "Row 1 of 1".to_owned(),
        count => format!("Rows 1–{count} of {count}"),
    }
}

/// `.rfoot`'s selection count. The grid selects one cell at a time, so this reads `0 selected` or
/// `1 selected` today; the count is taken rather than assumed so a multi-row selection can arrive
/// without this line changing.
fn selection_note(count: usize) -> String {
    format!("{} selected", table::grouped(count as u64))
}

/// `1 row` / `3 rows`, for the statements that report an affected count rather than rows.
fn affected_note(affected: u64) -> String {
    format!(
        "{} {}",
        table::grouped(affected),
        table::plural(affected, "row", "rows")
    )
}

/// `.rtab`'s label: the statement's own first word, which is what the design prints — `SELECT`,
/// `UPDATE`. Clipped, so a one-word statement cannot push the tab's own metrics out of it.
fn statement_label(statement: &str) -> String {
    let word = statement.split_whitespace().next().unwrap_or_default();
    table::clip(&word.to_ascii_uppercase(), 12)
}

/// `.eband`'s tag and message, split out of the engine's single sentence: every error in the house
/// type displays as `kind: message`, so the code is the first segment and the message follows it.
fn split_error(error: &str) -> (&str, &str) {
    match error.split_once(": ") {
        Some((code, message)) if !code.is_empty() && !code.contains(' ') => (code, message),
        _ => ("error", error),
    }
}

/// What `Copy error` puts on the clipboard: the engine's own sentence, and the statement it was
/// about — the same two facts the band shows.
fn error_text(error: &str, statement: &str) -> String {
    if statement.trim().is_empty() {
        error.to_owned()
    } else {
        format!("{error}\n{statement}")
    }
}

/// A duration in whole milliseconds, which is how every count in these bands is set.
fn millis(elapsed_ms: f64) -> i64 {
    elapsed_ms.round() as i64
}

/// The width a string needs, for the callers that reserve a slot before painting into it.
fn measured(ui: &Ui, font: FontId, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x
}

/// The width `.seg` takes for `labels`: each item is the design's `0 var(--s5)` around its label,
/// and `pane_segmented` shares the container's inner width — its border, its inset and its gaps —
/// equally between them.
fn segment_width(ui: &Ui, tokens: &Tokens, labels: &[(&str, bool)]) -> f32 {
    let widest = labels
        .iter()
        .map(|(label, _)| measured(ui, tokens.font(design::T_XS), label))
        .fold(0.0_f32, f32::max);
    let count = labels.len() as f32;
    let inset = 2.0;
    count * (2.0 * f32::from(design::S5) + widest) + 2.0 * (1.0 + inset) + inset * (count - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datacraft_core::Value;

    fn result(columns: &[&str], rows: Vec<Vec<Value>>) -> ResultSet {
        ResultSet {
            columns: columns.iter().map(|name| (*name).to_owned()).collect(),
            column_types: columns.iter().map(|_| "TEXT".to_owned()).collect(),
            rows,
            ..Default::default()
        }
    }

    fn rows(count: usize) -> Vec<Vec<Value>> {
        (0..count)
            .map(|index| {
                vec![
                    Value::Int(index as i64),
                    Value::text(format!("row {index}")),
                ]
            })
            .collect()
    }

    /// A statement that returned `count` rows of two columns.
    fn ok(statement: &str, count: usize, elapsed_ms: f64) -> StatementRun {
        StatementRun {
            statement: statement.to_owned(),
            result: Some(result(&["id", "name"], rows(count))),
            elapsed_ms,
            error: None,
        }
    }

    /// A statement that changed rows and returned none.
    fn changed(statement: &str, affected: u64, elapsed_ms: f64) -> StatementRun {
        StatementRun {
            statement: statement.to_owned(),
            result: Some(ResultSet {
                affected,
                ..Default::default()
            }),
            elapsed_ms,
            error: None,
        }
    }

    /// The statement the run stopped at.
    fn failed(statement: &str, elapsed_ms: f64) -> StatementRun {
        StatementRun {
            statement: statement.to_owned(),
            result: None,
            elapsed_ms,
            error: Some("query: no such column: statuss".to_owned()),
        }
    }

    fn state(runs: Vec<StatementRun>) -> SqlState {
        SqlState {
            text: "select 1;".to_owned(),
            last_result: runs.last().and_then(|run| run.result.clone()),
            results: runs,
            ..Default::default()
        }
    }

    /// Every string a frame painted, so a test can assert copy the product itself renders.
    fn painted(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    fn shows(strings: &[String], needle: &str) -> bool {
        strings.iter().any(|text| text.contains(needle))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_frame(
        ctx: &egui::Context,
        size: Vec2,
        events: Vec<egui::Event>,
        register: &mut Register,
        sql: &SqlState,
        density: datacraft_engine::Density,
        table: &mut TableState,
        actions: &mut Actions,
    ) -> (Layout, Vec<String>) {
        let tokens = Tokens::of("dark");
        tokens.apply(ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), size)),
            events,
            ..Default::default()
        };
        let mut layout = Layout::default();
        let mut out = ctx.run_ui(input, |ui| {
            // The app's central pane: no margin, the panel fill, and bands that touch.
            egui::CentralPanel::default_margins()
                .frame(egui::Frame::new().fill(tokens.panel))
                .show(ui, |ui| {
                    layout = paint(
                        ui,
                        &tokens,
                        register,
                        sql,
                        density,
                        200,
                        egui::Id::new(("register", 0)),
                        table,
                        actions,
                    );
                });
        });
        // The frame paints into a texture delta the headless context never applies.
        out.textures_delta.clear();
        let copy = painted(&out.shapes);
        (layout, copy)
    }

    /// The pane the design's central column is: 776 logical pixels wide.
    fn pane() -> Vec2 {
        egui::vec2(776.0, 900.0)
    }

    #[test]
    fn the_active_tab_is_the_last_statement_or_the_one_that_failed() {
        let three = state(vec![
            ok("select 1", 6, 12.0),
            ok("select 2", 3, 4.0),
            changed("update t set a = 1", 1, 6.0),
        ]);
        assert_eq!(Register::from(&three, None).active, 2);

        let stopped = state(vec![ok("select 1", 6, 12.0), failed("select 2", 2.0)]);
        assert_eq!(Register::from(&stopped, None).active, 1);

        assert_eq!(Register::from(&SqlState::default(), None).active, 0);
    }

    #[test]
    fn a_rerun_keeps_the_tab_the_user_was_reading() {
        let three = state(vec![
            ok("select 1", 6, 12.0),
            ok("select 2", 3, 4.0),
            ok("select 3", 1, 1.0),
        ]);
        let previous = Register {
            active: 1,
            view: View::Json,
            script: "select 1; select 2; select 3;".to_owned(),
        };
        let next = Register::from(&three, Some(&previous));
        assert_eq!(next.active, 1, "the tab must not move under the pointer");
        assert_eq!(next.view, View::Json, "the view is the user's choice too");

        // A shorter run cannot name that tab: the fallback takes over.
        let one = state(vec![ok("select 1", 6, 12.0)]);
        assert_eq!(Register::from(&one, Some(&previous)).active, 0);
    }

    #[test]
    fn every_tab_carries_its_own_rows_and_milliseconds() {
        assert_eq!(
            tab_facts(&ok("select id from t", 6, 12.0)),
            "6 rows · 12 ms"
        );
        assert_eq!(tab_facts(&ok("select id from t", 1, 0.4)), "1 rows · 0 ms");
        assert_eq!(tab_facts(&failed("select 2", 2.0)), "error · 2 ms");
        assert_eq!(
            tab_facts(&changed("update t set a = 1", 1, 6.0)),
            "1 row affected · 6 ms"
        );
        assert_eq!(
            tab_facts(&changed("delete from t", 3, 5.0)),
            "3 rows affected · 5 ms"
        );
    }

    #[test]
    fn a_tab_is_labelled_with_the_statements_own_word() {
        assert_eq!(statement_label("select id from main.t"), "SELECT");
        assert_eq!(statement_label("  update t set a = 1"), "UPDATE");
        assert_eq!(statement_label(""), "");
    }

    #[test]
    fn the_total_names_the_run_and_says_total_only_when_there_is_one() {
        let three = state(vec![
            ok("select 1", 6, 12.0),
            ok("select 2", 3, 4.0),
            changed("update t", 1, 6.0),
        ]);
        assert_eq!(Register::total_note(&three), "3 statements · 22 ms total");

        let one = state(vec![ok("select 1", 6, 12.0)]);
        assert_eq!(Register::total_note(&one), "1 statement · 12 ms");
    }

    #[test]
    fn the_note_names_the_statement_the_result_and_the_bound() {
        let one = state(vec![ok("select 1", 6, 12.0)]);
        assert_eq!(
            Register::from(&one, None).note(&one, 200),
            "complete result",
            "a single statement needs no ordinal"
        );

        let three = state(vec![
            ok("select 1", 6, 12.0),
            ok("select 2", 3, 4.0),
            ok("select 3", 1, 1.0),
        ]);
        let mut register = Register::from(&three, None);
        register.active = 1;
        assert_eq!(
            register.note(&three, 200),
            "statement 2 of 3 · complete result"
        );

        let bounded = state(vec![
            ok("select 1", 6, 12.0),
            StatementRun {
                statement: "select 2".to_owned(),
                result: Some(ResultSet {
                    columns: vec!["id".to_owned()],
                    column_types: vec!["INTEGER".to_owned()],
                    rows: rows(200),
                    truncated: true,
                    ..Default::default()
                }),
                elapsed_ms: 9.0,
                error: None,
            },
            ok("select 3", 1, 1.0),
        ]);
        assert_eq!(
            register.note(&bounded, 200),
            "statement 2 of 3 · bounded page · LIMIT 200",
            "the note must report the bound the engine applied, not the rows in hand"
        );

        let stopped = state(vec![ok("select 1", 6, 12.0), failed("select 2", 2.0)]);
        let register = Register::from(&stopped, None);
        assert_eq!(
            register.note(&stopped, 200),
            "statement 2 of 2 · no result set"
        );
    }

    #[test]
    fn a_register_goes_stale_when_the_editor_text_moves_on() {
        let sql = state(vec![ok("select 1", 6, 12.0)]);
        let register = Register::from(&sql, None);
        assert!(!register.stale("select 1;"), "the same text is not stale");
        assert!(register.stale("select 2;"));
        assert!(STALE_NOTE.contains("changed since this run"));
    }

    #[test]
    fn the_clipboard_carries_the_active_views_own_text() {
        let rows = rows(2);
        let sql = state(vec![StatementRun {
            statement: "select id, name from t".to_owned(),
            result: Some(result(&["id", "name"], rows.clone())),
            elapsed_ms: 3.0,
            error: None,
        }]);
        let result = sql.results[0].result.clone().unwrap_or_default();

        let mut register = Register::from(&sql, None);
        assert_eq!(register.clipboard(&sql), result.to_csv());
        assert!(register.clipboard(&sql).starts_with("id,name\n"));

        register.view = View::Json;
        assert_eq!(register.clipboard(&sql), json_text(&result));

        let nothing = state(vec![failed("select 2", 2.0)]);
        assert_eq!(
            Register::from(&nothing, None).clipboard(&nothing),
            "",
            "a statement with no result set has nothing to copy"
        );
    }

    #[test]
    fn the_footer_counts_the_rows_and_the_selection() {
        assert_eq!(rows_note(0), "No rows");
        assert_eq!(rows_note(1), "Row 1 of 1");
        assert_eq!(rows_note(6), "Rows 1–6 of 6");
        assert_eq!(selection_note(0), "0 selected");
        assert_eq!(selection_note(1), "1 selected");
        assert_eq!(selection_note(6), "6 selected");
    }

    #[test]
    fn the_json_text_is_the_engines_own_serialisation_of_the_rows() {
        let result = result(&["id", "name"], rows(6));
        let text = json_text(&result);
        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("the JSON view's text is JSON");
        assert_eq!(parsed["rows"].as_array().map(Vec::len), Some(6));
        assert_eq!(parsed["rows"][0]["id"], serde_json::json!(0));
        assert_eq!(parsed["columns"], serde_json::json!(["id", "name"]));
        assert!(
            text.starts_with("{\n") && text.ends_with("\n}"),
            "the view is pretty-printed"
        );
        // The envelope's own line count: the seven keys, the two columns and two types, and six rows
        // of two scalars. A change here is a change to what `Copy JSON` puts on the clipboard.
        assert_eq!(text.lines().count(), 40);
    }

    #[test]
    fn the_json_runs_carry_the_designs_inks() {
        let runs = json_runs("{\n  \"id\": 1,\n  \"name\": null,\n  \"ok\": true\n}");
        let inks: Vec<Ink> = runs.iter().map(|(_, ink)| *ink).collect();
        assert_eq!(
            inks,
            vec![
                Ink::Punct,
                Ink::Key,
                Ink::Punct,
                Ink::Value,
                Ink::Punct,
                Ink::Key,
                Ink::Punct,
                Ink::Null,
                Ink::Punct,
                Ink::Key,
                Ink::Punct,
                Ink::Value,
                Ink::Punct,
            ]
        );
        // The whitespace stays with the run that follows it, so the layout keeps its newlines.
        assert!(runs.iter().all(|(text, _)| !text.is_empty()));
        assert_eq!(
            runs.iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            "{\n  \"id\": 1,\n  \"name\": null,\n  \"ok\": true\n}",
        );
    }

    #[test]
    fn the_json_job_sets_the_line_pitch_and_the_three_inks() {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        // The context has no fonts until it has run a frame, and the pitch below is measured from a
        // laid-out galley. The frame's texture delta is the headless context's to drop.
        let mut warm = ctx.run_ui(egui::RawInput::default(), |_| {});
        warm.textures_delta.clear();
        let job = json_job(&tokens, "{\"id\": 1, \"name\": null}");
        for section in &job.sections {
            assert_eq!(section.format.line_height, Some(JSON_ROW_H));
            assert_eq!(section.format.font_id, tokens.mono(design::T_SM));
        }
        let inks: Vec<egui::Color32> = job.sections.iter().map(|s| s.format.color).collect();
        assert!(inks.contains(&tokens.text_dim));
        assert!(inks.contains(&tokens.text_faint));
        assert!(inks.contains(&tokens.text));

        let galley =
            ctx.fonts_mut(|fonts| fonts.layout_job(json_job(&tokens, "{\n  \"id\": 1\n}")));
        assert_eq!(galley.rows.len(), 3, "one row per line of the pretty text");
        let pitch = galley.rows[1].pos.y - galley.rows[0].pos.y;
        assert!(
            (pitch - JSON_ROW_H).abs() < 0.5,
            "the JSON view's row pitch is {pitch}, not the design's {JSON_ROW_H}"
        );
    }

    /// One tab per run, each carrying its own statement's facts, and the total over them — the
    /// design's three-statement strip.
    #[test]
    fn the_tab_strip_carries_one_tab_per_statement() {
        let ctx = egui::Context::default();
        let sql = state(vec![
            ok("select id from t where plan = 'enterprise'", 6, 12.0),
            ok("select id from t where plan = 'pro'", 3, 4.0),
            changed("update t set plan = 'pro' where id = 6", 1, 6.0),
        ]);
        let mut register = Register::from(&sql, None);
        let mut table = TableState::default();
        let mut actions = Actions::default();
        let (layout, copy) = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );
        assert!(layout.tabs.is_some());
        for (ordinal, facts) in [
            ("1", "6 rows · 12 ms"),
            ("2", "3 rows · 4 ms"),
            ("3", "1 row affected · 6 ms"),
        ] {
            assert!(shows(&copy, ordinal), "tab {ordinal}'s ordinal is painted");
            assert!(
                shows(&copy, facts),
                "tab {ordinal} does not carry its own facts ({facts})"
            );
        }
        assert!(shows(&copy, "SELECT"));
        assert!(shows(&copy, "UPDATE"));
        assert!(shows(&copy, "3 statements · 22 ms total"));
    }

    /// The flush stack: the register's bands touch, the grid takes what is left, and the footer sits
    /// on the pane's floor. Every number here is the plan's own arithmetic.
    #[test]
    fn the_bands_are_flush_and_the_footer_lands_on_the_panes_floor() {
        let ctx = egui::Context::default();
        let sql = state(vec![ok("select id, name from main.customers", 6, 12.0)]);
        let mut register = Register::from(&sql, None);
        let mut table = TableState::default();
        let mut actions = Actions::default();
        let (layout, copy) = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );

        let tabs = layout.tabs.expect("the tab strip");
        let bar = layout.bar.expect("the status bar");
        let grid = layout.grid.expect("the grid");
        let footer = layout.footer.expect("a result with rows has a footer");
        assert_eq!(tabs.top(), 0.0, "the register starts at the pane's origin");
        assert!((tabs.height() - design::REG_TABS_H).abs() < 0.5);
        assert!((bar.height() - design::TOOLBAR_H).abs() < 0.5);
        assert!((bar.top() - tabs.bottom()).abs() < 0.5);
        assert!((grid.top() - bar.bottom()).abs() < 0.5);
        assert!((footer.height() - FOOTER_H).abs() < 0.5);
        assert!((footer.top() - grid.bottom()).abs() < 0.5);
        assert!(
            (footer.bottom() - pane().y).abs() < 0.5,
            "the footer's hairline is the pane's last row, not {}",
            footer.bottom()
        );
        assert!((layout.row_h - design::ROW_H).abs() < 0.5);
        assert_eq!(layout.empty, None);
        assert_eq!(layout.error, None);
        assert_eq!(layout.void, None);
        assert!(shows(&copy, "6 rows · 12 ms"));
        assert!(shows(&copy, "complete result"));
        assert!(shows(&copy, "Rows 1–6 of 6"));
        assert!(shows(&copy, READ_ONLY));
        assert!(
            shows(&copy, "SELECT"),
            "the tab is labelled with the statement"
        );
    }

    #[test]
    fn the_void_and_the_error_band_replace_the_grid_on_a_failed_statement() {
        let ctx = egui::Context::default();
        let sql = state(vec![
            ok("select 1", 6, 12.0),
            failed("select id, statuss from t", 2.0),
        ]);
        let mut register = Register::from(&sql, None);
        assert_eq!(register.active, 1, "a failure opens its own tab");
        let mut table = TableState::default();
        let mut actions = Actions::default();
        let (layout, copy) = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );

        let bar = layout.bar.expect("the status bar");
        let error = layout.error.expect("the failed statement's band");
        let void = layout.void.expect("the void under it");
        assert_eq!(layout.grid, None);
        assert_eq!(layout.footer, None, "a failed statement has no footer");
        assert!((error.top() - bar.bottom()).abs() < 0.5);
        assert!((error.height() - design::register_error_band_h()).abs() < 0.5);
        assert!((void.top() - error.bottom()).abs() < 0.5);
        assert!((void.bottom() - pane().y).abs() < 0.5);
        assert!(shows(&copy, "failed · 2 ms"));
        assert!(shows(&copy, "statement 2 of 2 · no result set"));
        assert!(shows(&copy, "query"), "the tag is the error's own code");
        assert!(shows(&copy, "no such column: statuss"));
        assert!(shows(&copy, VOID_NOTE));
        assert!(shows(&copy, "Retry"));
        assert!(shows(&copy, "Copy error"));
    }

    #[test]
    fn a_zero_row_result_keeps_its_head_and_puts_the_block_under_it() {
        let ctx = egui::Context::default();
        let sql = state(vec![StatementRun {
            statement: "select id, name from t where status = 'paused'".to_owned(),
            result: Some(result(&["id", "name"], Vec::new())),
            elapsed_ms: 4.0,
            error: None,
        }]);
        let mut register = Register::from(&sql, None);
        let mut table = TableState::default();
        let mut actions = Actions::default();
        let (layout, copy) = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );

        let grid = layout.grid.expect("the head of a zero-row result stays");
        let block = layout.empty.expect("the block");
        assert!(
            (grid.height() - design::HEAD_H).abs() < 0.5,
            "a zero-row result asks the widget for its head alone, measured {}",
            grid.height()
        );
        assert!((block.top() - grid.bottom()).abs() < 0.5);
        assert!((block.height() - design::EMPTY_BLOCK_H).abs() < 0.5);
        assert_eq!(layout.footer, None, "no rows, no footer");
        assert!(shows(&copy, EMPTY_TITLE));
        assert!(shows(
            &copy,
            "The statement completed in 4 ms and matched 0 rows."
        ));
        assert!(shows(&copy, "Copy query"));
        assert!(shows(&copy, "Re-run"));
    }

    /// A statement that changed rows returns no columns: the block reports what it affected rather
    /// than claiming it matched nothing.
    #[test]
    fn a_statement_with_no_columns_reports_what_it_affected() {
        let run = changed("update t set a = 1", 3, 6.0);
        let result = run.result.clone().unwrap_or_default();
        let (title, body) = empty_copy(&run, &result);
        assert_eq!(title, EMPTY_TITLE);
        assert_eq!(body, "The statement completed in 6 ms and affected 3 rows.");
        assert_eq!(stat_text(&run), "3 rows affected · 6 ms");
    }

    #[test]
    fn nothing_that_has_run_paints_the_invitation_and_no_bands() {
        let ctx = egui::Context::default();
        let sql = SqlState::default();
        let mut register = Register::from(&sql, None);
        let mut table = TableState::default();
        let mut actions = Actions::default();
        let (layout, copy) = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );
        assert_eq!(
            layout,
            Layout::default(),
            "no run means no tabs, no bar and no grid"
        );
        assert!(shows(&copy, INVITATION));
    }

    /// The design's zero-row frame keeps the columns; the widget must be handed the pitch the density
    /// setting asks for, in both densities.
    #[test]
    fn the_grid_follows_the_density_preference() {
        let ctx = egui::Context::default();
        let sql = state(vec![ok("select id, name from t", 6, 12.0)]);
        for (density, pitch) in [
            (datacraft_engine::Density::Compact, design::ROW_H),
            (
                datacraft_engine::Density::Comfortable,
                design::ROW_COMFORT_H,
            ),
        ] {
            let mut register = Register::from(&sql, None);
            let mut table = TableState::default();
            let mut actions = Actions::default();
            let (layout, _) = run_frame(
                &ctx,
                pane(),
                Vec::new(),
                &mut register,
                &sql,
                density,
                &mut table,
                &mut actions,
            );
            assert!(
                (layout.row_h - pitch).abs() < 0.5,
                "{density:?} asked for {pitch}, the grid was handed {}",
                layout.row_h
            );
        }
    }

    /// A read-only grid still selects, and never asks for an editor. The widget reports the event;
    /// the register has no `BeginEdit` path to take.
    #[test]
    fn a_double_click_selects_a_cell_and_opens_no_editor() {
        let ctx = egui::Context::default();
        let sql = state(vec![ok("select id, name from main.customers", 6, 12.0)]);
        let mut register = Register::from(&sql, None);
        let mut table = TableState::default();
        let mut actions = Actions::default();
        // The first frame lays the grid out so the second can be pointed at a real cell.
        let _ = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );
        let cell = ctx
            .read_response(egui::Id::new(("register", 0)).with(("cell", 0, 1)))
            .expect("the grid's cell registered its rect")
            .rect;
        let mut events = vec![egui::Event::PointerMoved(cell.center())];
        for _ in 0..2 {
            events.push(egui::Event::PointerButton {
                pos: cell.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            });
            events.push(egui::Event::PointerButton {
                pos: cell.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            });
        }
        let (layout, _) = run_frame(
            &ctx,
            pane(),
            events,
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );
        assert!(layout.grid.is_some());
        assert_eq!(
            table.selected,
            Some((0, 1)),
            "the click still selects the cell"
        );
        assert_eq!(table.editing, None, "a result set is not editable");
    }

    /// The actions the bands advertise are the product's own: the CSV of the active result, the
    /// statement itself, and a re-run of the script.
    #[test]
    fn the_registers_buttons_ask_for_the_view_they_are_about() {
        let ctx = egui::Context::default();
        let sql = state(vec![ok("select id, name from main.customers", 6, 12.0)]);
        let mut register = Register::from(&sql, None);
        register.view = View::Json;
        let mut table = TableState::default();
        let mut actions = Actions::default();
        let result = sql.results[0].result.clone().unwrap_or_default();

        // The Copy button is the design's `Copy JSON` in this view, and it copies this view's text.
        assert_eq!(register.clipboard(&sql), json_text(&result));
        assert_eq!(View::Json.copy_label(), "Copy JSON");
        assert_eq!(View::Grid.copy_label(), "Copy");

        let (layout, copy) = run_frame(
            &ctx,
            pane(),
            Vec::new(),
            &mut register,
            &sql,
            datacraft_engine::Density::Compact,
            &mut table,
            &mut actions,
        );
        assert!(shows(&copy, "Copy JSON"));
        assert!(shows(&copy, "Export"));
        assert!(shows(&copy, JSON_HINT));
        assert!(shows(&copy, "6 rows"));
        let json = layout.json.expect("the JSON view");
        let lines = json_text(&result).lines().count();
        assert!((json.top() - layout.bar.expect("the status bar").bottom()).abs() < 0.5);
        assert!(
            (json.height() - (lines as f32 * JSON_ROW_H + 2.0 * f32::from(design::S5))).abs() < 0.5,
            "the JSON band is {} tall for {lines} lines",
            json.height()
        );
        assert!(
            shows(&copy, "1\n2\n3"),
            "the gutter numbers every line of the pretty text"
        );
    }

    #[test]
    fn the_error_tag_is_the_engines_own_code() {
        assert_eq!(
            split_error("query: near \"FROMM\": syntax error"),
            ("query", "near \"FROMM\": syntax error")
        );
        assert_eq!(
            split_error("SQLITE_ERROR: no such column: statuss"),
            ("SQLITE_ERROR", "no such column: statuss")
        );
        assert_eq!(split_error("cancelled"), ("error", "cancelled"));
        assert_eq!(
            error_text("query: no such column: statuss", "select statuss from t"),
            "query: no such column: statuss\nselect statuss from t"
        );
        assert_eq!(error_text("cancelled", "  "), "cancelled");
    }
}
