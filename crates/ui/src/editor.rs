//! The query editor register: the toolbar, the SQL area with its gutter, statement markers and row
//! fills, the statement strip, the statement error band, the catalog-aware completion popup — and,
//! under them, the results register [`crate::register`] paints.
//!
//! Everything here reads the tab's own `SqlState`: the script it holds, the runs the engine recorded
//! and the error the failed one carries. A single statement runs in the background — `dispatch`
//! routes it to `sql.runAsync` and the frame keeps drawing while `pump_runner` advances it — so while
//! one is in flight the editor paints the design's running state from `runner.running_for_tab`: a
//! `.prog` hairline, an `Executing…` pill, and a live `Stop` that cancels the job. A multi-statement
//! script stays blocking on the UI thread (the runner streams one statement), so there is no running
//! state to show and the `Stop` is painted disabled with the engine's own sentence as its hover text.

use egui::text::LayoutJob;
use egui::{
    Align2, CornerRadius, Key, Margin, Modifiers, Rect, RichText, Sense, Stroke, TextFormat, Ui,
    Vec2,
};
use nook_engine::{Level, Session};

use crate::app::UiState;
use crate::register::{self, Register};
use crate::table;
use crate::theme::{self as design, Emphasis, Mark, Tokens};

/// The code's left inset inside the SQL area: the design's 52px `.gut` and its `--s5` before `.code`.
const CODE_X: f32 = design::GUTTER_W + design::S5 as f32;
/// How many characters of a statement a strip chip prints before it is clipped.
const CHIP_CHARS: usize = 28;
/// The completion popup's shortest prefix: the design opens it on `st`, and one letter would offer
/// the whole catalog.
const POP_MIN: usize = 2;
/// How many popup items are painted at most, so its height is bounded on a wide catalog.
const POP_ITEMS: usize = 8;
/// `.pop .grp`'s height: 10px of type in 6px above and 2px below.
const POP_GROUP_H: f32 = 18.0;

/// The caret's place in the script: 1-based line and column, and the byte offset they are counted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub line: usize,
    pub column: usize,
    pub index: usize,
}

/// The bands one frame painted, so the caller — and the tests below — can measure the stack.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub toolbar: Option<Rect>,
    pub sql: Option<Rect>,
    pub strip: Option<Rect>,
    /// `.band`: present only while the run's failed statement has something to say.
    pub band: Option<Rect>,
    /// The strip below the band — the design's failed frame keeps the script's own row under it.
    pub strip_lower: Option<Rect>,
    /// Where the register starts: the pane's bottom border is part of the editor.
    pub register_top: Option<f32>,
    pub popup: Option<Rect>,
    /// The SQL area's rows, one per line, in screen coordinates: the gutter, the current row's fill
    /// and the statement markers are all placed from them.
    pub rows: Vec<Rect>,
}

/// The SQL area's text edit id, one per tab: the editor's own cursor and the completion popup both
/// hang off it, so a second query tab cannot inherit the first one's.
pub fn editor_id(tab_index: usize) -> egui::Id {
    egui::Id::new(("editor", tab_index))
}

/// The caret at a byte offset, clamped to the text and snapped to a character boundary: a stale
/// offset — the text shrank under it — must not split the character the user just typed.
pub fn caret_in(text: &str, index: usize) -> Caret {
    let limit = index.min(text.len());
    let index = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .take_while(|i| *i <= limit)
        .last()
        .unwrap_or(0);
    let line_start = text[..index].rfind('\n').map(|i| i + 1).unwrap_or(0);
    Caret {
        line: text[..index].matches('\n').count() + 1,
        column: text[line_start..index].chars().count() + 1,
        index,
    }
}

/// The caret a text edit reported. `None` when the widget has no cursor — the editor then keeps the
/// one it already had, because the read-out is a fact about the script, not about the focus.
pub fn caret_line_col(text: &str, range: Option<egui::text::CCursorRange>) -> Option<Caret> {
    let primary = range?.primary;
    let index = text
        .char_indices()
        .nth(primary.index.0)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    Some(caret_in(text, index))
}

/// The statement the caret sits in, for "run statement at cursor". `None` is honest — the caret in
/// trailing whitespace, or past a comment the splitter dropped — and every caller takes
/// `unwrap_or(0)`: that button must always run something, and the first statement is that something.
pub fn caret_statement(text: &str, caret: Option<Caret>) -> Option<usize> {
    nook_engine::sqltext::statement_index_at(text, caret?.index)
}

/// The lines one statement owns, 1-based and inclusive, counted from the script itself. `split` trims
/// and drops comments, so a comment or blank line above a statement belongs to no statement and
/// carries neither a marker nor a row fill.
pub fn statement_lines(text: &str, index: usize) -> Option<(usize, usize)> {
    let statements = nook_engine::cmd::sql::split(text);
    let mut cursor = 0usize;
    for (nth, statement) in statements.iter().enumerate() {
        let start = text[cursor..].find(statement).map(|i| cursor + i)?;
        let end = start + statement.len();
        if nth == index {
            return Some((
                text[..start].matches('\n').count() + 1,
                text[..end].matches('\n').count() + 1,
            ));
        }
        cursor = end;
    }
    None
}

/// The strip's right-hand facts, which are the text's own: its line count, its line ending and its
/// encoding. UTF-8 is a fact rather than a default — the editor's buffer is a `String`.
pub fn text_facts(text: &str) -> String {
    format!(
        "{} lines · {} · UTF-8",
        text.lines().count(),
        if text.contains("\r\n") { "CRLF" } else { "LF" }
    )
}

/// The SQL area's syntax job: one section per token, at the design's 20px line pitch, and never
/// wrapped — the design's `.code` is `white-space:pre`, so one line is one row and the gutter's
/// numbers stay in step with the code.
pub fn sql_job(tokens: &Tokens, text: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    job.keep_trailing_whitespace = true;
    for token in nook_engine::sqltext::tokenize(text) {
        let (color, italics) = match token.kind {
            nook_engine::sqltext::TokenKind::Keyword => (tokens.accent, false),
            nook_engine::sqltext::TokenKind::Str => (tokens.ok, false),
            nook_engine::sqltext::TokenKind::Number => (tokens.info, false),
            nook_engine::sqltext::TokenKind::Comment => (tokens.text_faint, true),
            nook_engine::sqltext::TokenKind::Punct => (tokens.text_dim, false),
            nook_engine::sqltext::TokenKind::Word => (tokens.text, false),
        };
        job.append(
            &token.text,
            0.0,
            TextFormat {
                font_id: tokens.mono(design::T_MONO),
                color,
                italics,
                line_height: Some(design::EDITOR_ROW_H),
                ..Default::default()
            },
        );
    }
    job
}

/// `Ln x, Col y`, as the toolbar and the status bar print it: one wording, two places.
pub fn caret_note(caret: Caret) -> String {
    format!("Ln {}, Col {}", caret.line, caret.column)
}

/// The engine's own sentence for `sql.stop`, which is what a Stop with nothing to stop says when the
/// pointer rests on it: the button cannot describe the engine differently from the engine.
pub fn stop_reason() -> &'static str {
    nook_engine::find("sql.stop")
        .map(|spec| spec.help)
        .unwrap_or("Stopping a run is not possible")
}

/// Paint the editor register and, under it, the results register.
pub fn show(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) -> Layout {
    // The design stacks its bands hairline to hairline, so the pane's stray default spacing must not
    // put a gap between them. Same as `grid::show` and the register.
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut actions = Actions::default();
    let mut layout = Layout::default();
    let mut acted = false;

    let text = session.tabs[tab_index].sql.text.clone();
    let caret = caret_in(&text, state.caret.map_or(0, |caret| caret.index));

    let tab_id = session.tabs[tab_index].id;
    let running_job = session.runner.running_for_tab(tab_id).cloned();
    layout.toolbar = Some(toolbar(
        ui,
        caret,
        &text,
        tokens,
        &mut actions,
        running_job.as_ref(),
    ));
    // The find bar sits above the script when it is open: it edits the text below it, so it belongs
    // between the toolbar and the editor rather than floating over the code.
    find_bar(ui, session, tab_index, state, tokens, &mut actions);
    // Parameters are chips, not a form: what the script needs, and whether this tab has a value.
    parameter_chips(ui, session, tab_index, tokens);
    // The band sits between the toolbar and the script: it drafts *into* this tab, so a modal would
    // cover the editor it exists to fill.
    crate::ai::band(ui, session, state, tokens);
    layout.popup = sql_area(ui, session, tab_index, state, tokens, &mut layout);
    // `.prog`: a 2px accent rule along the editor's top edge while a single statement runs in the
    // background. Gated on the tab's own running job — the same one `toolbar` reads to enable Stop —
    // so a blocking script or a settled run paints none.
    if running_job.is_some() {
        if let Some(sql) = layout.sql {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(sql.min, egui::vec2(sql.width(), 2.0)),
                0.0,
                tokens.accent,
            );
        }
    }
    // The toolbar's and the editor's own controls have recorded what they want by now. Running them
    // before the outcome surfaces are measured is what makes a click on Run paint its own result —
    // the strips, the band, the register — in the frame that asked for it.
    acted |= dispatch(ui, session, tab_index, state, &mut actions);

    let text = session.tabs[tab_index].sql.text.clone();
    let caret = caret_in(&text, state.caret.map_or(0, |caret| caret.index));
    let statements = nook_engine::cmd::sql::split(&text);
    let failed = session.tabs[tab_index]
        .sql
        .results
        .iter()
        .position(|run| run.error.is_some());
    let parts = Parts {
        statements: &statements,
        caret: caret_statement(&text, Some(caret)),
        failed,
    };
    layout.strip = Some(strip(ui, tokens, &parts, &text_facts(&text), &mut actions));

    if let Some((title, message)) = band_copy(session, tab_index, &text, parts.failed) {
        layout.band = Some(design::statement_band(ui, tokens, &title, &message, |ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S4);
            if design::toolbar_button(
                ui,
                tokens,
                "Copy message",
                None,
                tokens.error_fill,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .on_hover_text("Copy the driver's message")
            .clicked()
            {
                actions.copy = Some(message.clone());
            }
            if design::toolbar_button(
                ui,
                tokens,
                "Retry",
                None,
                tokens.error_fill,
                Emphasis::Primary,
                true,
                |_| {},
            )
            .on_hover_text("Run the script again")
            .clicked()
            {
                actions.command = Some(("sql.run", serde_json::json!({})));
            }
        }));
        // The design's failed frame keeps a strip under the band, carrying the statement that
        // stopped the run.
        let note = failed_note(parts.failed, parts.statements.len());
        layout.strip_lower = Some(strip(ui, tokens, &parts, &note, &mut actions));
    }
    // `.pane`'s `border-bottom`: the only separator between the editor and the register, and part of
    // the pane — so the register starts one pixel below the last strip.
    border_row(ui, tokens);
    // The strips' and the band's own controls are painted after the first dispatch, so theirs run
    // here.
    acted |= dispatch(ui, session, tab_index, state, &mut actions);
    if acted {
        // A control that ran something has to be drawn again with its own outcome, and the frame that
        // asked is not necessarily the frame that shows it. egui does not repaint on its own once the
        // click that started this has been consumed.
        ui.ctx().request_repaint();
    }
    layout.register_top = Some(ui.cursor().top());

    // A register is a view of the last run, so it follows the engine: when the run's own statements no
    // longer match the script the register recorded, a run has landed — from this toolbar or from the
    // control channel — and the register is rebuilt around it. Re-running the same script keeps the
    // tab the user was reading, which is what `Register::from` does with a previous register.
    let mut register = match state.registers.remove(&tab_index) {
        Some(register) if !run_moved_on(&register, session, tab_index) => register,
        previous => Register::from(&session.tabs[tab_index].sql, previous.as_ref()),
    };
    register::show(ui, session, tab_index, &mut register, state, tokens);
    state.registers.insert(tab_index, register);

    layout
}

/// The right-hand editor of a split tab.
///
/// Deliberately thin: the pane has its own text and its own Run, and nothing else. The register, the
/// strips and the bands all belong to the left pane, because a second copy of the run bookkeeping is
/// a second set of rules to keep in sync for a feature whose whole point is two queries side by side.
pub fn show_pane(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) -> Layout {
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut layout = Layout::default();
    let mut actions = Actions::default();
    let pane_id = egui::Id::new(("editor-split", tab_index));

    let split = session.tabs[tab_index].sql.split.take();
    let Some(mut split) = split else {
        return layout;
    };
    let caret = caret_in(&split.text, 0);
    // The right pane has its own editor; a background run belongs to the tab, so the same job is
    // what Stop would cancel from either side.
    let tab_id = session.tabs[tab_index].id;
    let running_job = session.runner.running_for_tab(tab_id).cloned();
    layout.toolbar = Some(toolbar(
        ui,
        caret,
        &split.text,
        tokens,
        &mut actions,
        running_job.as_ref(),
    ));
    // The pane's Run runs *its* text: `sql.run` is handed the script explicitly, so the engine does
    // not fall back to the left pane's editor text.
    if let Some((command, mut params)) = actions.command.take() {
        if command == "sql.run" {
            params["sql"] = serde_json::Value::String(split.text.clone());
            // `pane: right` is what keeps this run's rows in `SplitState.results` instead of
            // replacing the left pane's.
            params["pane"] = serde_json::Value::String("right".into());
        }
        // The right pane stays blocking: its rows live in `SplitState`, and `pump` appends to the
        // tab's own run, so an async run here would land in the wrong pane. Named in `docs/gaps.md`.
        if let Err(e) = session.run(command, params) {
            session.log(Level::Warn, e.to_string());
        }
    }
    actions.command = None;
    design::band_fill(
        ui,
        design::EDITOR_H,
        tokens.panel,
        0,
        egui::Align::Min,
        |ui| {
            let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
                ui.fonts_mut(|fonts| fonts.layout_job(sql_job(tokens, buffer.as_str())))
            };
            egui::TextEdit::multiline(&mut split.text)
                .id(pane_id)
                .code_editor()
                .frame(egui::Frame::new().inner_margin(Margin {
                    left: CODE_X as i8,
                    right: design::S5,
                    top: 0,
                    bottom: 0,
                }))
                .desired_width(f32::INFINITY)
                .desired_rows(design::EDITOR_H as usize / design::EDITOR_ROW_H as usize)
                .layouter(&mut layouter)
                .show(ui);
        },
    );
    border_row(ui, tokens);
    // The right pane's own result: a plain read-only table of the last run's rows. The full register
    // (views, copy, selection) stays on the left pane — this is two queries side by side, not two
    // result workbenches, and a second register is a second set of rules for no extra question.
    let pane_rows = session.tabs[tab_index]
        .sql
        .split
        .as_ref()
        .and_then(|split| {
            split
                .results
                .iter()
                .rev()
                .find_map(|run| run.result.clone())
        });
    if let Some(rs) = pane_rows.as_ref() {
        let _ = state;
        ui.spacing_mut().item_spacing = egui::vec2(f32::from(design::S4), 2.0);
        egui::ScrollArea::both().id_salt(pane_id).show(ui, |ui| {
            egui::Grid::new(("split-grid", tab_index))
                .striped(true)
                .show(ui, |ui| {
                    for name in &rs.columns {
                        ui.label(RichText::new(name).strong().color(tokens.text));
                    }
                    ui.end_row();
                    for row in rs.rows.iter().take(200) {
                        for cell in row {
                            ui.label(
                                RichText::new(cell.display())
                                    .font(tokens.mono(design::T_XS))
                                    .color(if cell.is_null() {
                                        tokens.text_dim
                                    } else {
                                        tokens.text
                                    }),
                            );
                        }
                        ui.end_row();
                    }
                });
        });
    }
    session.tabs[tab_index].sql.split = Some(split);
    layout
}

/// What one painted frame asked the caller to do. The widget never touches the run itself, so a test
/// can drive it headlessly and read the request back.
#[derive(Debug, Default)]
struct Actions {
    /// Run this command with these parameters, through the registry.
    command: Option<(&'static str, serde_json::Value)>,
    /// Put this on the clipboard.
    copy: Option<String>,
    /// Empty the script.
    clear: bool,
    /// Open the AI band, or close it if it is already open.
    toggle_ai: bool,
}

/// The strip's contents: the script's statements, the one the caret is in, and the one that failed.
struct Parts<'a> {
    statements: &'a [String],
    caret: Option<usize>,
    failed: Option<usize>,
}

/// True when the engine has run a script the register has not seen. The register records the script
/// its rows came from and the engine records the one it ran, so the two disagree exactly when a run
/// landed that the register predates — a run whose split happens to match the previous one included.
/// Comparing the splits instead missed the rerun whose text had moved on inside a statement.
fn run_moved_on(register: &Register, session: &Session, tab_index: usize) -> bool {
    let sql = &session.tabs[tab_index].sql;
    sql.results.is_empty() || register.script != sql.ran_script
}

/// `.tb`: the design's order, cut to the width the pane has — `Run`, `Run statement`, `Stop`,
/// `Explain`, `Format`, `Clear`, then the caret read-out and the statement count. The connection chip
/// is cut because it does not fit (the fit is a test below); `Chart`, `Pivot`, `Save snippet` and
/// `Export` are cut because the Query menu, the palette and the register's own `.rbar` reach them.
/// The find bar: a field, the replace field when it is open, and the two toggles.
///
/// Replaces through `sql.findReplace`, so the caret rules and the whole-word rule live in one place
/// (the engine) instead of being re-implemented by the widget that happens to be painting.
fn find_bar(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
    actions: &mut Actions,
) {
    let Some(mut bar) = state.find.take() else {
        return;
    };
    let mut close = false;
    // Set when this frame walked to a hit: the editor's selection is moved after the bar paints, so
    // "find next" is visible as a selection instead of only as a number nobody sees.
    let mut walked = false;
    design::band_fill(
        ui,
        design::TOOLBAR_H,
        tokens.panel,
        design::S4,
        egui::Align::Center,
        |ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S3);
            ui.label(
                RichText::new("Find")
                    .font(tokens.mono(design::T_XS))
                    .color(tokens.text_dim),
            );
            let field = egui::TextEdit::singleline(&mut bar.find)
                .desired_width(180.0)
                .hint_text("find");
            let response = ui.add(field);
            if bar.replace_open {
                ui.label(
                    RichText::new("Replace")
                        .font(tokens.mono(design::T_XS))
                        .color(tokens.text_dim),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut bar.replace)
                        .desired_width(180.0)
                        .hint_text("replace"),
                );
            }
            for (label, hint, on) in [
                ("Aa", "Match case", &mut bar.case),
                ("|ab|", "Whole word", &mut bar.word),
            ] {
                if ui.toggle_value(on, label).on_hover_text(hint).clicked() {
                    *on = !*on;
                }
            }
            if design::toolbar_button(
                ui,
                tokens,
                if bar.replace_open {
                    "Replace all"
                } else {
                    "Find next"
                },
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .clicked()
            {
                if bar.replace_open {
                    actions.command = Some((
                        "sql.findReplace",
                        serde_json::json!({
                            "find": bar.find, "replace": bar.replace,
                            "case": bar.case, "word": bar.word, "all": true,
                        }),
                    ));
                } else {
                    // Find next only *walks*: it moves the selection onto the next hit and writes
                    // nothing. Going through `sql.findReplace` with an empty replacement would
                    // delete the hit it claims to find.
                    bar.current = next_hit(&session.tabs[tab_index].sql.text, &bar, bar.current);
                    walked = true;
                }
            }
            if ui.button("✕").on_hover_text("Close (Esc)").clicked() {
                close = true;
            }
            if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                // Enter walks too, and `Cmd+Enter` replaces the hit it is sitting on.
                if bar.replace_open && ui.input(|i| i.modifiers.command) {
                    if let Some(index) = bar.current {
                        let text = &session.tabs[tab_index].sql.text;
                        let hits =
                            nook_engine::sqltext::find_hits(text, &bar.find, bar.case, bar.word);
                        if let Some((start, end)) = hits.get(index).copied() {
                            actions.command = Some((
                                "sql.findReplace",
                                serde_json::json!({
                                    "find": bar.find, "replace": bar.replace,
                                    "case": bar.case, "word": bar.word,
                                    "at": [start, end],
                                }),
                            ));
                        }
                    }
                } else {
                    bar.current = next_hit(&session.tabs[tab_index].sql.text, &bar, bar.current);
                    walked = true;
                }
            }
            if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        },
    );
    if walked {
        if let Some(index) = bar.current {
            let text = &session.tabs[tab_index].sql.text;
            let hits = nook_engine::sqltext::find_hits(text, &bar.find, bar.case, bar.word);
            if let Some(&(start, end)) = hits.get(index) {
                // Byte offsets to character offsets: `CCursor` counts chars, and a hit after a
                // multi-byte character would land in the wrong place if the two were conflated.
                let from = text[..start].chars().count();
                let to = from + text[start..end].chars().count();
                let mut edit = egui::text_edit::TextEditState::load(ui.ctx(), editor_id(tab_index))
                    .unwrap_or_default();
                edit.cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(from),
                        egui::text::CCursor::new(to),
                    )));
                edit.store(ui.ctx(), editor_id(tab_index));
            }
        }
    }
    if !close {
        state.find = Some(bar);
    }
}

/// The hit after `current`, wrapping at the end. `None` when the text has no hits at all, so the
/// bar can say so instead of pretending it moved.
fn next_hit(text: &str, bar: &crate::app::FindBar, current: Option<usize>) -> Option<usize> {
    let hits = nook_engine::sqltext::find_hits(text, &bar.find, bar.case, bar.word);
    if hits.is_empty() {
        return None;
    }
    Some(current.map_or(0, |index| (index + 1) % hits.len()))
}

/// The parameter chips: one per placeholder the script needs, filled or not.
fn parameter_chips(ui: &mut Ui, session: &mut Session, tab_index: usize, tokens: &Tokens) {
    let sql = session.tabs[tab_index].sql.text.clone();
    let found = nook_engine::sqltext::parameters(&sql);
    if found.is_empty() {
        return;
    }
    let bound = session.tabs[tab_index].sql.params.clone();
    design::band_fill(
        ui,
        design::TOOLBAR_H,
        tokens.panel_alt,
        design::S4,
        egui::Align::Center,
        |ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S2);
            ui.label(
                RichText::new(format!(
                    "{} parameter(s)",
                    found.named.len().max(found.positional)
                ))
                .font(tokens.mono(design::T_XS))
                .color(tokens.text_dim),
            );
            for name in found.names() {
                let value = bound.get(&name).cloned().flatten();
                let (label, colour) = match &value {
                    Some(v) => (format!(":{name} = {v}"), tokens.text),
                    None => (format!(":{name}"), tokens.error_fill),
                };
                ui.label(
                    RichText::new(label)
                        .font(tokens.mono(design::T_XS))
                        .color(colour),
                );
            }
            if found.positional > 0 {
                ui.label(
                    RichText::new(format!("{} positional slot(s)", found.positional))
                        .font(tokens.mono(design::T_XS))
                        .color(tokens.text_dim),
                );
            }
        },
    );
}

fn toolbar(
    ui: &mut Ui,
    caret: Caret,
    text: &str,
    tokens: &Tokens,
    actions: &mut Actions,
    running_job: Option<&nook_engine::runner::Job>,
) -> Rect {
    let index = caret_statement(text, Some(caret)).unwrap_or(0);
    let rect = design::band_fill(
        ui,
        design::TOOLBAR_H,
        tokens.panel_alt,
        design::S4,
        egui::Align::Center,
        |ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S4);
            if design::toolbar_button(
                ui,
                tokens,
                "Run",
                Some(Mark::Play),
                tokens.panel_high,
                Emphasis::Primary,
                true,
                |_| {},
            )
            .on_hover_text("Run the whole script (Cmd+Enter)")
            .clicked()
            {
                actions.command = Some(("sql.run", serde_json::json!({})));
            }
            if design::toolbar_button(
                ui,
                tokens,
                "Run statement",
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .on_hover_text("Run the statement the cursor sits in")
            .clicked()
            {
                actions.command = Some(("sql.runStatement", serde_json::json!({"index": index})));
            }
            // A single statement is running in the background: say so beside the Stop that ends it.
            if running_job.is_some() {
                design::chip(ui, tokens, "Executing…", tokens.text_faint);
            }
            // Enabled exactly when there is a background run to stop: the button's own state is the
            // answer to "can I stop this", and it carries the registry's help when it cannot.
            let running = running_job.map(|job| job.id);
            if design::toolbar_button(
                ui,
                tokens,
                "Stop",
                Some(Mark::Stop),
                tokens.panel_high,
                Emphasis::Plain,
                running.is_some(),
                |_| {},
            )
            .on_hover_text(match running_job {
                Some(job) => format!(
                    "Cancel background run {} ({} rows so far)",
                    job.id, job.rows
                ),
                None => stop_reason().to_string(),
            })
            .clicked()
            {
                if let Some(id) = running {
                    actions.command = Some(("sql.cancel", serde_json::json!({ "job": id })));
                }
            }
            vdiv(ui, tokens);
            if design::toolbar_button(
                ui,
                tokens,
                "Explain",
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .on_hover_text("Ask the server for the plan")
            .clicked()
            {
                // The visual form, which also fills the tab's plan so the register can draw it.
                actions.command = Some(("sql.explainVisual", serde_json::json!({})));
            }
            if design::toolbar_button(
                ui,
                tokens,
                "Format",
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .on_hover_text("Pretty-print the script (Cmd+Shift+F)")
            .clicked()
            {
                actions.command = Some(("sql.format", serde_json::json!({})));
            }
            if design::toolbar_button(
                ui,
                tokens,
                "Clear",
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .on_hover_text("Empty the script")
            .clicked()
            {
                actions.clear = true;
            }
            vdiv(ui, tokens);
            if design::toolbar_button(
                ui,
                tokens,
                "Generate SQL",
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |_| {},
            )
            .on_hover_text("Draft SQL from a sentence, with @table context (Cmd+Shift+G)")
            .clicked()
            {
                actions.toggle_ai = true;
            }
            // The design's right cluster: the caret read-out, a divider, then the trailing fact.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = f32::from(design::S4);
                ui.label(
                    RichText::new(statements_note(text))
                        .font(tokens.mono(design::T_XS))
                        .color(tokens.text_dim),
                );
                vdiv(ui, tokens);
                ui.label(
                    RichText::new(caret_note(caret))
                        .font(tokens.mono(design::T_XS))
                        .color(tokens.text_dim),
                );
            });
        },
    );
    design::band_hairline(ui, tokens, rect);
    rect
}

/// The `.vdiv` between two groups of controls: a 1px line, 18 tall, in a 17px slot (1 + 8 + 8).
fn vdiv(ui: &mut Ui, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top()..=rect.bottom(),
        Stroke::new(1.0, tokens.stroke),
    );
}

/// `N statements`, the toolbar's trailing fact: the script's own length, which the strip's chips then
/// name one by one.
fn statements_note(text: &str) -> String {
    let count = nook_engine::cmd::sql::split(text).len();
    format!(
        "{count} {}",
        table::plural(count as u64, "statement", "statements")
    )
}

/// `.ed`: the 320px SQL area — the gutter, the tokenised code, the statement markers, the current and
/// failed row fills, and the completion popup over it. Writes the row rects and the caret read-out
/// into `layout` and `state`, and returns the popup's rect when it painted one.
fn sql_area(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
    layout: &mut Layout,
) -> Option<Rect> {
    let id = editor_id(tab_index);
    let caret = caret_in(
        &session.tabs[tab_index].sql.text,
        state.caret.map_or(0, |caret| caret.index),
    );
    let active = caret_statement(&session.tabs[tab_index].sql.text, Some(caret));
    let failed = session.tabs[tab_index]
        .sql
        .results
        .iter()
        .position(|run| run.error.is_some());
    let mut popup = None;
    let rect = design::band_fill(
        ui,
        design::EDITOR_H,
        tokens.panel,
        0,
        egui::Align::Min,
        |ui| {
            // The band's own rect, which is the child's whole area at an inset of zero.
            let area = ui.max_rect();
            // The row fills have to land *under* the glyphs, and egui does not paint twice: the slot
            // is reserved here and set once the galley has said where the rows are.
            let fill_slot = ui.painter().add(egui::Shape::Noop);
            // The popup takes the keys it wants before the text edit reads them, or Enter would
            // insert a newline and accept a completion in the same press.
            popup = completion(ui, session, tab_index, caret, tokens);
            let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
                ui.fonts_mut(|fonts| fonts.layout_job(sql_job(tokens, buffer.as_str())))
            };
            let output = egui::TextEdit::multiline(&mut session.tabs[tab_index].sql.text)
                .id(id)
                .code_editor()
                .frame(egui::Frame::new().inner_margin(Margin {
                    left: CODE_X as i8,
                    right: design::S5,
                    top: 0,
                    bottom: 0,
                }))
                .desired_width(f32::INFINITY)
                .desired_rows(design::EDITOR_H as usize / design::EDITOR_ROW_H as usize)
                .layouter(&mut layouter)
                .show(ui);
            if output.response.response.changed() {
                state.editor_dirty = true;
            }
            let text = session.tabs[tab_index].sql.text.as_str();
            let owned = statement_lines(text, failed.unwrap_or(usize::MAX));
            let marked = active.and_then(|index| statement_lines(text, index));
            layout.rows.clear();
            let mut fills = Vec::new();
            for (line, row) in output.galley.rows.iter().enumerate() {
                let line = line + 1;
                let row_rect = Rect::from_min_size(
                    egui::pos2(area.left(), output.galley_pos.y + row.pos.y),
                    Vec2::new(area.width(), design::EDITOR_ROW_H),
                );
                if caret.line == line {
                    fills.push(egui::Shape::rect_filled(
                        row_rect,
                        CornerRadius::ZERO,
                        tokens.panel_alt,
                    ));
                }
                if owned.is_some_and(|(first, last)| (first..=last).contains(&line)) {
                    fills.push(egui::Shape::rect_filled(
                        row_rect,
                        CornerRadius::ZERO,
                        tokens.error_fill,
                    ));
                }
                if marked.is_some_and(|(first, last)| (first..=last).contains(&line)) {
                    // `.mk`: 2px at the pane's left edge, over every line of the statement the caret
                    // is in — `error` when that statement is the one that failed.
                    ui.painter().rect_filled(
                        Rect::from_min_size(
                            egui::pos2(area.left(), row_rect.top()),
                            Vec2::new(design::INDICATOR_W, design::EDITOR_ROW_H),
                        ),
                        CornerRadius::ZERO,
                        if failed == active {
                            tokens.error
                        } else {
                            tokens.accent
                        },
                    );
                }
                let ink = if owned.is_some_and(|(first, last)| (first..=last).contains(&line)) {
                    tokens.error
                } else if caret.line == line {
                    tokens.accent
                } else {
                    tokens.text_dim
                };
                ui.painter().text(
                    egui::pos2(
                        area.left() + design::GUTTER_W - f32::from(design::S5),
                        row_rect.center().y,
                    ),
                    Align2::RIGHT_CENTER,
                    line.to_string(),
                    tokens.mono(design::T_XS),
                    ink,
                );
                layout.rows.push(row_rect);
            }
            ui.painter().set(fill_slot, egui::Shape::Vec(fills));
            state.caret = Some(
                caret_line_col(text, output.cursor_range)
                    .unwrap_or_else(|| caret_in(text, caret.index)),
            );
        },
    );
    layout.sql = Some(rect);
    popup
}

/// `.strip`: one 20px `.st` chip per statement — the caret's own filled and ringed, the failed one in
/// `error` — and the row's own fact on the right. The chips are bounded to what the fact leaves them
/// (the design's row is a `nowrap` flex row and paints over it), and the caret's own statement stays
/// in the window, so the strip never hides the statement the editor is showing.
fn strip(
    ui: &mut Ui,
    tokens: &Tokens,
    parts: &Parts<'_>,
    fact: &str,
    actions: &mut Actions,
) -> Rect {
    let rect = design::band_fill(
        ui,
        design::STATUS_H,
        tokens.panel_alt,
        design::S4,
        egui::Align::Center,
        |ui| {
            let gap = f32::from(design::S3);
            ui.spacing_mut().item_spacing.x = gap;
            let band = ui.available_width();
            let labels: Vec<String> = parts
                .statements
                .iter()
                .enumerate()
                .map(|(index, statement)| {
                    let dot = parts.caret == Some(index);
                    // A chip is never wider than the band it is painted in: a clipped label plus the
                    // chrome around it can already be, and the strip's `.fchips`-sized row has to hold
                    // on the 236px pane the app's own minimum window leaves.
                    table::ellipsize(
                        ui,
                        &chip_label(index, statement),
                        tokens.mono(design::T_XS),
                        (band - chip_chrome(dot)).max(0.0),
                    )
                })
                .collect();
            let widths: Vec<f32> = labels
                .iter()
                .enumerate()
                .map(|(index, label)| chip_width(ui, tokens, label, parts.caret == Some(index)))
                .collect();
            // The fact is measured before the chips are painted, so the row's own width is reserved
            // rather than overrun — and it gives way when reserving it would leave the chips less than
            // they need, which is the case on the 236px pane the app's own minimum window leaves.
            let fact_w = design::text_width(ui, tokens.mono(design::T_XS), fact);
            let row = design::fit_row_with_fact(
                band,
                gap,
                &widths,
                design::more_pill_width(ui, tokens, labels.len()),
                fact_w,
                parts.caret,
            );
            let fitted = row.fitted;
            if !widths.is_empty() {
                if fitted.head > 0 {
                    design::more_pill(ui, tokens, fitted.head, design::STMT_H, tokens.panel_high)
                        .on_hover_text("More statements than the strip can show");
                }
                for index in fitted.start..=fitted.end {
                    let label = &labels[index];
                    let state = chip_state(parts, index);
                    if chip(
                        ui,
                        tokens,
                        label,
                        &parts.statements[index],
                        state,
                        widths[index],
                    )
                    .clicked()
                    {
                        actions.command =
                            Some(("sql.runStatement", serde_json::json!({ "index": index })));
                    }
                }
                if fitted.tail > 0 {
                    design::more_pill(ui, tokens, fitted.tail, design::STMT_H, tokens.panel_high)
                        .on_hover_text("More statements than the strip can show");
                }
            }
            if row.fact {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(fact)
                                .font(tokens.mono(design::T_XS))
                                .color(tokens.text_dim),
                        )
                        .truncate(),
                    );
                });
            }
        },
    );
    // `.strip`'s own `border-top`, drawn on its top edge: 435 with the pane's code above it, 517 with
    // the statement band.
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.top() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    rect
}

/// What a `.st` chip says about its statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chip {
    /// The statement the caret sits in.
    Active,
    /// The statement that stopped the run.
    Failed,
    Resting,
}

/// Which of the three states a statement's chip is in.
fn chip_state(parts: &Parts<'_>, index: usize) -> Chip {
    match (parts.caret == Some(index), parts.failed == Some(index)) {
        (true, _) => Chip::Active,
        (false, true) => Chip::Failed,
        _ => Chip::Resting,
    }
}

/// The label a `.st` chip carries: the design's `1 · SELECT …`, clipped.
fn chip_label(index: usize, statement: &str) -> String {
    format!(
        "{} · {}",
        index + 1,
        table::clip(statement.trim(), CHIP_CHARS)
    )
}

/// What that label costs in a chip besides the label itself: two `--s3` paddings, the 1px border pair
/// and the 5px dot when the chip is the caret's.
fn chip_chrome(dot: bool) -> f32 {
    2.0 * f32::from(design::S3)
        + 2.0
        + if dot {
            5.0 + f32::from(design::S3)
        } else {
            0.0
        }
}

/// What that label costs in a chip — the mono line and the chrome around it. The strip measures with
/// this and `chip` paints with it, so the budget the row plans against is the width it actually spends.
fn chip_width(ui: &Ui, tokens: &Tokens, label: &str, dot: bool) -> f32 {
    design::text_width(ui, tokens.mono(design::T_XS), label) + chip_chrome(dot)
}

/// One `.st` chip: 20 tall, pill-radius, with its own fill, ring and ink, and a 5px dot when it is the
/// caret's, at the width its label measured. The whole statement is on hover.
fn chip(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    statement: &str,
    state: Chip,
    width: f32,
) -> egui::Response {
    let dot = state == Chip::Active;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, design::STMT_H), Sense::click());
    let hovered = response.hovered();
    let response = response.on_hover_text(statement.trim());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let (fill, border, ink) = match state {
        Chip::Active => (tokens.accent_dim, tokens.accent_line, tokens.text),
        Chip::Failed => (tokens.error_fill, tokens.error, tokens.error),
        Chip::Resting => (tokens.panel_high, tokens.stroke, tokens.text),
    };
    let radius = CornerRadius::same(design::R_CHIP);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, if hovered { tokens.hover } else { fill });
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + f32::from(design::S3);
    if dot {
        painter.circle_filled(egui::pos2(x + 2.5, rect.center().y), 2.5, tokens.accent);
        x += 5.0 + f32::from(design::S3);
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        tokens.mono(design::T_XS),
        ink,
    );
    response
}

/// The pane's last pixel row: `.pane`'s `border-bottom`. It is allocated, not just painted, so the
/// register below starts under it.
fn border_row(ui: &mut Ui, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.center().y,
        Stroke::new(1.0, tokens.stroke),
    );
}

/// The band's two lines: the driver's own message, and the code, the position and what the failure
/// cost. The position is the failed statement's start — the one place in the script the editor can
/// name without guessing what the driver meant.
fn band_copy(
    session: &Session,
    tab_index: usize,
    text: &str,
    failed: Option<usize>,
) -> Option<(String, String)> {
    let (code, message) = register::split_error(session.tabs[tab_index].sql.error()?);
    let index = failed?;
    let line = statement_lines(text, index)
        .map(|(first, _)| first)
        .unwrap_or(1);
    Some((
        message.to_owned(),
        format!(
            "{code} · Ln {line}, Col 1 · statement {} · nothing was executed",
            index + 1
        ),
    ))
}

/// The lower strip's fact: which statement the run stopped at.
fn failed_note(failed: Option<usize>, count: usize) -> String {
    match failed {
        Some(index) if count > 1 => format!("statement {} of {count} not run", index + 1),
        Some(index) => format!("statement {} not run", index + 1),
        None => String::new(),
    }
}

/// Run what the frame collected, through the same registry the CLI, MCP and the control channel use.
fn dispatch(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    actions: &mut Actions,
) -> bool {
    let mut acted = false;
    if actions.clear {
        actions.clear = false;
        session.tabs[tab_index].sql.text.clear();
        state.editor_dirty = true;
        acted = true;
    }
    if actions.toggle_ai {
        actions.toggle_ai = false;
        state.ai_band = !state.ai_band;
        if state.ai_band {
            // Seed once, on open: the form's own copy is the working one from here, so a frame's
            // typing is never overwritten by the session's stored values.
            crate::ai::seed(session, &mut state.ai_form);
        }
        acted = true;
    }
    if let Some(text) = actions.copy.take() {
        ui.ctx().copy_text(text);
        acted = true;
    }
    let Some((id, mut params)) = actions.command.take() else {
        return acted;
    };
    if let Some(map) = params.as_object_mut() {
        if let Some(conn) = session.tabs[tab_index].conn {
            map.insert(
                "conn".into(),
                serde_json::json!(session.conns[conn].cfg.id.clone()),
            );
        }
        if !map.contains_key("sql") {
            let text = session.tabs[tab_index].sql.text.clone();
            if !text.trim().is_empty() {
                map.insert("sql".into(), serde_json::json!(text));
            }
        }
    }
    // A one-statement run goes to the background: the frame keeps drawing, rows land as they
    // arrive, and Stop has a job to cancel. A script stays blocking, because the runner streams one
    // statement — pretending otherwise would drop the rest of it.
    let id = match id {
        "sql.run"
            if params["sql"]
                .as_str()
                .map(|sql| nook_engine::cmd::sql::split(sql).len() == 1)
                .unwrap_or(false) =>
        {
            "sql.runAsync"
        }
        id => id,
    };
    match session.run(id, params) {
        // `sql.explain` records its own plan as the tab's run, the way `sql.run` does: the engine owns
        // what a run leaves behind, so the register shows a plan without the UI re-shaping it. Only
        // `sql.format` has an answer the editor has to take back.
        Ok(value) if id == "sql.format" => {
            if let Some(formatted) = value["sql"].as_str() {
                if let Some(tab) = session.tabs.get_mut(tab_index) {
                    tab.sql.text = formatted.to_string();
                }
            }
        }
        Ok(_) => {}
        Err(e) => session.log(Level::Error, format!("{id}: {e}")),
    }
    true
}

/// What the popup remembers between frames. It is a transient of the editor, not a fact of the
/// session, so it lives in egui's memory keyed by the tab's own id.
#[derive(Debug, Clone, Default, PartialEq)]
struct PopupState {
    selected: usize,
    /// The script at the moment Escape dismissed the popup: it reopens when the text moves on.
    dismissed: Option<String>,
}

/// One completion candidate: what would be inserted, what it is, and which group it is shown in.
struct Candidate {
    text: String,
    detail: String,
    group: &'static str,
}

/// `.pop`: the catalog-aware completion list, opened from the word before the caret. `sql.complete`
/// supplies the candidates and the session's snippets join them, so the popup offers what the engine
/// and the user's own library hold. While it is open it takes `Enter`, `Tab`, `Escape` and the arrow
/// keys and nothing else: every other key belongs to the text.
fn completion(
    ui: &mut Ui,
    session: &mut Session,
    tab_index: usize,
    caret: Caret,
    tokens: &Tokens,
) -> Option<Rect> {
    let id = editor_id(tab_index).with("popup");
    let text = session.tabs[tab_index].sql.text.clone();
    let mut state = ui
        .memory(|memory| memory.data.get_temp::<PopupState>(id))
        .unwrap_or_default();
    let prefix = word_before(&text, caret.index);
    let open = state.dismissed.as_deref() != Some(text.as_str())
        && prefix.chars().count() >= POP_MIN
        && !text.is_empty();
    let mut candidates = if open {
        candidates(session, &text, &prefix)
    } else {
        Vec::new()
    };
    candidates.truncate(POP_ITEMS);
    if candidates.is_empty() {
        ui.memory_mut(|memory| memory.data.insert_temp(id, state));
        return None;
    }
    state.selected = state.selected.min(candidates.len() - 1);

    let mut accept = false;
    let mut close = false;
    let mut step = 0i32;
    ui.input_mut(|input| {
        accept = input.consume_key(Modifiers::NONE, Key::Enter)
            || input.consume_key(Modifiers::NONE, Key::Tab);
        close = input.consume_key(Modifiers::NONE, Key::Escape);
        step += i32::from(input.consume_key(Modifiers::NONE, Key::ArrowDown));
        step -= i32::from(input.consume_key(Modifiers::NONE, Key::ArrowUp));
    });
    if step != 0 {
        let count = candidates.len() as i32;
        state.selected = (state.selected as i32 + step).rem_euclid(count) as usize;
    }
    if close {
        state.dismissed = Some(text.clone());
        state.selected = 0;
        ui.memory_mut(|memory| memory.data.insert_temp(id, state));
        // The key was the popup's, so the editor must not read it as its own: a text edit that sees
        // Escape gives up the focus, and the caret would go with the list.
        ui.memory_mut(|memory| memory.request_focus(editor_id(tab_index)));
        return None;
    }

    let rect = Rect::from_min_size(
        egui::pos2(ui.max_rect().left() + 150.0, ui.min_rect().top() + 120.0),
        Vec2::new(design::POP_W, popup_height(&candidates)),
    );
    let mut clicked = None;
    egui::Area::new(id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let (outer, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            let radius = CornerRadius::same(design::R_BOX);
            let painter = ui.painter();
            painter.rect_filled(outer, radius, tokens.panel_alt);
            painter.rect_stroke(
                outer,
                radius,
                Stroke::new(1.0, tokens.stroke_strong),
                egui::StrokeKind::Inside,
            );
            let header = Rect::from_min_size(outer.min, Vec2::new(outer.width(), design::POP_HD_H));
            painter.rect_filled(
                header,
                CornerRadius {
                    nw: design::R_BOX,
                    ne: design::R_BOX,
                    sw: 0,
                    se: 0,
                },
                tokens.panel_high,
            );
            painter.hline(
                outer.left()..=outer.right(),
                header.bottom() - 0.5,
                Stroke::new(1.0, tokens.stroke),
            );
            painter.text(
                egui::pos2(header.left() + f32::from(design::S4), header.center().y),
                Align2::LEFT_CENTER,
                format!("Completion · \"{prefix}\""),
                tokens.mono(design::T_XS),
                tokens.text_dim,
            );
            let footer = Rect::from_min_max(
                egui::pos2(outer.left(), outer.bottom() - design::POP_FT_H),
                outer.max,
            );
            painter.rect_filled(
                footer,
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: design::R_BOX,
                    se: design::R_BOX,
                },
                tokens.panel_high,
            );
            painter.hline(
                footer.left()..=footer.right(),
                footer.top() + 0.5,
                Stroke::new(1.0, tokens.stroke),
            );
            painter.text(
                egui::pos2(footer.left() + f32::from(design::S4), footer.center().y),
                Align2::LEFT_CENTER,
                "Enter inserts · Tab replaces · Esc dismisses",
                tokens.font(design::T_XS),
                tokens.text_dim,
            );
            painter.text(
                egui::pos2(footer.right() - f32::from(design::S4), footer.center().y),
                Align2::RIGHT_CENTER,
                format!("{} matches", candidates.len()),
                tokens.mono(design::T_XS),
                tokens.text_dim,
            );

            let mut y = header.bottom();
            let mut group = "";
            for (index, candidate) in candidates.iter().enumerate() {
                if candidate.group != group {
                    group = candidate.group;
                    painter.text(
                        egui::pos2(outer.left() + f32::from(design::S4), y + POP_GROUP_H / 2.0),
                        Align2::LEFT_CENTER,
                        group.to_uppercase(),
                        tokens.mono(design::T_XS),
                        tokens.text_dim,
                    );
                    y += POP_GROUP_H;
                }
                let item = Rect::from_min_size(
                    egui::pos2(outer.left(), y),
                    Vec2::new(outer.width(), design::POP_ITEM_H),
                );
                let response = ui.interact(item, id.with(("item", index)), Sense::click());
                if index == state.selected {
                    painter.rect_filled(item, CornerRadius::ZERO, tokens.accent_dim);
                    painter.rect_stroke(
                        item,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, tokens.accent_line),
                        egui::StrokeKind::Inside,
                    );
                }
                painter.text(
                    egui::pos2(item.left() + f32::from(design::S4), item.center().y),
                    Align2::LEFT_CENTER,
                    table::clip(&candidate.text, 40),
                    tokens.mono(design::T_SM),
                    tokens.text,
                );
                painter.text(
                    egui::pos2(item.right() - f32::from(design::S4), item.center().y),
                    Align2::RIGHT_CENTER,
                    &candidate.detail,
                    tokens.mono(design::T_XS),
                    tokens.text_dim,
                );
                if response.clicked() {
                    clicked = Some(index);
                }
                y += design::POP_ITEM_H;
            }
        });

    if let Some(index) = clicked {
        state.selected = index;
        accept = true;
    }
    if accept {
        let candidate = &candidates[state.selected];
        let mut edit = egui::text_edit::TextEditState::load(ui.ctx(), editor_id(tab_index))
            .unwrap_or_default();
        if let Some(tab) = session.tabs.get_mut(tab_index) {
            tab.sql.text = replace_prefix(&tab.sql.text, caret.index, &prefix, &candidate.text);
            edit.cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(
                        tab.sql.text
                            [..cursor_after(&tab.sql.text, caret.index, &prefix, &candidate.text)]
                            .chars()
                            .count(),
                    ),
                )));
        }
        edit.store(ui.ctx(), editor_id(tab_index));
        state.dismissed = Some(session.tabs[tab_index].sql.text.clone());
        ui.memory_mut(|memory| memory.data.insert_temp(id, state));
        // The list goes with the keystroke that took the candidate: painting it in this frame would
        // put the candidates beside the text they were replaced by.
        return None;
    }
    ui.memory_mut(|memory| memory.data.insert_temp(id, state));
    Some(rect)
}

/// The candidate list's heights: the header, one group label per distinct group, one row each, and
/// the footer.
fn popup_height(candidates: &[Candidate]) -> f32 {
    let groups = candidates
        .iter()
        .enumerate()
        .filter(|(index, candidate)| *index == 0 || candidates[index - 1].group != candidate.group)
        .count() as f32;
    design::POP_HD_H
        + design::POP_FT_H
        + groups * POP_GROUP_H
        + candidates.len() as f32 * design::POP_ITEM_H
}

/// The candidates for `prefix`: the engine's own list for the statement, then the session's snippets.
fn candidates(session: &mut Session, text: &str, prefix: &str) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    if let Ok(value) = session.run(
        "sql.complete",
        serde_json::json!({"prefix": prefix, "sql": text, "limit": POP_ITEMS * 4}),
    ) {
        for item in value["candidates"].as_array().into_iter().flatten() {
            out.push(Candidate {
                text: item["text"].as_str().unwrap_or_default().to_owned(),
                detail: item["detail"].as_str().unwrap_or_default().to_owned(),
                group: match item["kind"].as_str().unwrap_or_default() {
                    "column" => "Columns",
                    "table" => "Tables",
                    "function" => "Functions",
                    "snippet" => "Snippets",
                    _ => "Keywords",
                },
            });
        }
    }
    let needle = prefix.to_ascii_lowercase();
    for snippet in &session.snippets {
        if snippet.name.to_ascii_lowercase().starts_with(&needle) {
            out.push(Candidate {
                text: snippet.sql.clone(),
                detail: snippet.name.clone(),
                group: "Snippets",
            });
        }
    }
    out
}

/// The word the caret has typed: the run of word characters immediately before `index`.
fn word_before(text: &str, index: usize) -> String {
    let head = &text[..index.min(text.len())];
    let start = head
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    head[start..].to_owned()
}

/// The byte offset just past the word [`replace_prefix`] writes.
fn cursor_after(text: &str, index: usize, prefix: &str, word: &str) -> usize {
    index.min(text.len()).saturating_sub(prefix.len()) + word.len()
}

/// `text` with the word before `index` replaced by `word`, from the same word helper the popup opened
/// with — so the replacement cannot land beside what was typed.
fn replace_prefix(text: &str, index: usize, prefix: &str, word: &str) -> String {
    let end = index.min(text.len());
    let start = end.saturating_sub(prefix.len());
    format!("{}{word}{}", &text[..start], &text[end..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ClippedShape;
    use egui::{pos2, Color32};
    use nook_engine::TabKind;

    /// Everything one frame painted, plus the layout the editor measured.
    #[derive(Default)]
    struct Frame {
        layout: Layout,
        strings: Vec<String>,
        texts: Vec<(Rect, String)>,
        fills: Vec<(Rect, Color32)>,
        strokes: Vec<(Rect, Color32)>,
        lines: Vec<(f32, Color32)>,
    }

    impl Frame {
        fn shows(&self, needle: &str) -> bool {
            self.strings.iter().any(|text| text.contains(needle))
        }

        /// The painted rect of one exact label: the real control, not a guess at its geometry.
        fn label(&self, label: &str) -> Rect {
            self.texts
                .iter()
                .find(|(_, text)| text == label)
                .map(|(rect, _)| *rect)
                .unwrap_or_else(|| panic!("no painted {label:?}: {:#?}", self.strings))
        }

        /// The centre of one painted label, for pointer input.
        fn hit(&self, label: &str) -> egui::Pos2 {
            self.label(label).center()
        }

        /// The wide band whose top edge is at `top`, if the frame painted one.
        fn band(&self, top: f32) -> Option<Rect> {
            self.fills
                .iter()
                .map(|(rect, _)| *rect)
                .find(|rect| (rect.top() - top).abs() < 0.5 && rect.width() > 700.0)
        }
    }

    fn collect(shapes: &[ClippedShape]) -> Frame {
        fn walk(shape: &egui::Shape, frame: &mut Frame) {
            match shape {
                egui::Shape::Text(text) => {
                    let string = text.galley.text().to_owned();
                    frame
                        .texts
                        .push((text.visual_bounding_rect(), string.clone()));
                    frame.strings.push(string);
                }
                egui::Shape::Rect(rect) => {
                    frame.fills.push((rect.rect, rect.fill));
                    if rect.stroke.width > 0.0 {
                        frame.strokes.push((rect.rect, rect.stroke.color));
                    }
                }
                egui::Shape::LineSegment { points, stroke } => {
                    frame.lines.push((points[0].y, stroke.color));
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, frame);
                    }
                }
                _ => {}
            }
        }
        let mut frame = Frame::default();
        for clipped in shapes {
            walk(&clipped.shape, &mut frame);
        }
        frame
    }

    /// The pane the design's central column is: 776 wide under the app bar, whole to the status bar.
    fn pane() -> Rect {
        Rect::from_min_max(pos2(344.0, 75.0), pos2(1120.0, 874.0))
    }

    /// The pane the app's own minimum window leaves the central column: 900px less the rail and the
    /// tree (44 + 300) and the always-shown inspector (320). It is the narrowest the shell's own window
    /// permits — a panel dragged past it leaves less — and the rows that bound themselves have to hold
    /// in it.
    fn narrow_pane() -> Rect {
        pane_of_width(236.0)
    }

    /// [`narrow_pane`] at another width, for the sweeps that check a bounded row holds below it too.
    fn pane_of_width(width: f32) -> Rect {
        Rect::from_min_max(pos2(344.0, 75.0), pos2(344.0 + width, 874.0))
    }

    /// A live SQLite connection with a two-column `customers` table, a refreshed catalog, and one
    /// query tab holding `script`.
    fn session(name: &str, script: &str) -> Session {
        let path =
            std::env::temp_dir().join(format!("nook-editor-{}-{name}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut session = Session::new();
        session
            .conns
            .push(nook_engine::Conn::new(nook_core::ConnectionConfig::sqlite(
                path.to_string_lossy(),
            )));
        session.active_conn = Some(0);
        nook_engine::cmd::open_now(&mut session, 0).expect("the fixture database opens");
        nook_engine::cmd::run_sql(
            &mut session,
            0,
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, city TEXT NOT NULL);\
             INSERT INTO customers (city) VALUES ('London'), ('Lima');",
            Some(10),
        )
        .expect("the fixture schema is valid");
        nook_engine::cmd::refresh_catalog(&mut session, 0).expect("the catalog refreshes");
        let index = session.add_tab(TabKind::Query, "Query", Some(0));
        session.tabs[index].sql.text = script.to_owned();
        session.active_tab = Some(index);
        session
    }

    /// One real frame of the app's central pane, at the real rects, with real input.
    fn frame(
        ctx: &egui::Context,
        pane_state: &mut (Session, UiState, Tokens),
        events: Vec<egui::Event>,
        time: f64,
    ) -> Frame {
        frame_in(ctx, pane_state, events, time, pane())
    }

    /// The same frame in a pane of another width: the row tests push the band the shell's own
    /// minimum window leaves.
    fn frame_in(
        ctx: &egui::Context,
        pane_state: &mut (Session, UiState, Tokens),
        events: Vec<egui::Event>,
        time: f64,
        rect: Rect,
    ) -> Frame {
        // A one-statement run goes to the background, so a test frame has to pump the way the app's
        // frame does: take whatever the worker sent, and wait for a run that is still going so a
        // frame after a click paints the result the click asked for.
        let input = {
            let (session, _, _) = pane_state;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                session.pump();
                if !session.runner.jobs().any(|j| j.state.is_running())
                    || std::time::Instant::now() > deadline
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1440.0, 900.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            }
        };
        paint_run(ctx, pane_state, input, rect)
    }

    /// One frame with no wait-for-done pump: for asserting a run's *running* state, which by
    /// definition has not settled. [`frame`] drains the worker's messages before painting, which
    /// would flip the job to `Done`; here the background run a click just started is still `Running`
    /// — nothing has drained it — so the hairline and the `Executing…` pill are on screen. No engine
    /// seam: the job's state only changes in `drain`, so not calling it is what keeps it running.
    fn paint(
        ctx: &egui::Context,
        pane_state: &mut (Session, UiState, Tokens),
        events: Vec<egui::Event>,
        time: f64,
    ) -> Frame {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1440.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        paint_run(ctx, pane_state, input, pane())
    }

    /// Apply the tokens, run one UI frame at `rect`, and collect what it painted. Shared by
    /// [`frame_in`] (after its pump) and [`paint`] (with none), so the two render paths never drift.
    fn paint_run(
        ctx: &egui::Context,
        pane_state: &mut (Session, UiState, Tokens),
        input: egui::RawInput,
        rect: Rect,
    ) -> Frame {
        let (session, state, tokens) = pane_state;
        tokens.apply(ctx);
        let mut layout = Layout::default();
        let mut out = ctx.run_ui(input, |ui| {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let index = session.active_tab.unwrap_or(0);
            layout = show(&mut child, session, index, state, tokens);
        });
        out.textures_delta.clear();
        let mut frame = collect(&out.shapes);
        frame.layout = layout;
        frame
    }

    fn world(name: &str, script: &str) -> (Session, UiState, Tokens) {
        (
            session(name, script),
            UiState::default(),
            Tokens::of("dark"),
        )
    }

    /// The query tab's state: the session starts on its Welcome tab, so the tab under test is the
    /// one the app would paint — the active one.
    fn tab(world: &(Session, UiState, Tokens)) -> &nook_engine::Tab {
        &world.0.tabs[world.0.active_tab.unwrap()]
    }

    /// A click: the pointer arrives and the button goes down and up on the same spot.
    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]
    }

    fn hover(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![egui::Event::PointerMoved(pos)]
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// A cursor on line `line` (1-based) at the pane's right edge: past every glyph on a short line,
    /// so the caret lands at the text's end instead of mid-word.
    fn line_at(f: &Frame, line: usize) -> egui::Pos2 {
        let sql = f.layout.sql.unwrap();
        let row = f.layout.rows[line - 1];
        pos2(sql.right() - f32::from(design::S5), row.center().y)
    }

    fn line_1(f: &Frame) -> egui::Pos2 {
        line_at(f, 1)
    }

    fn line_2(f: &Frame) -> egui::Pos2 {
        line_at(f, 2)
    }

    /// The frames egui needs after a click before the editor shows its consequences. A click grants
    /// the focus at the end of its own frame; the widget reports the cursor it was given on the next;
    /// and whatever hangs off the caret — the row fill, the marker, the completion list — paints on
    /// the one after that. Three frames, and the last one is the frame to measure.
    fn settle(ctx: &egui::Context, world: &mut (Session, UiState, Tokens), time: f64) -> Frame {
        let _ = frame(ctx, world, Vec::new(), time + 0.02);
        let _ = frame(ctx, world, Vec::new(), time + 0.04);
        frame(ctx, world, Vec::new(), time + 0.06)
    }

    #[test]
    fn the_read_out_counts_lines_columns_and_bytes_over_a_multi_byte_script() {
        let text = "select 'é';\nselect 1;";
        // Line 1 is 11 characters and 12 bytes, so line 2 starts at character 12 and byte 13.
        let caret = caret_in(text, 20);
        assert_eq!((caret.line, caret.column, caret.index), (2, 8, 20));
        // A byte offset in the middle of `é` snaps back to its start rather than splitting it.
        assert_eq!(
            caret_in(text, 9),
            Caret {
                line: 1,
                column: 9,
                index: 8
            }
        );
        assert_eq!(caret_in(text, 999), caret_in(text, text.len()));
        assert_eq!(caret_note(caret_in(text, 20)), "Ln 2, Col 8");
        // The widget counts characters; the read-out counts bytes.
        assert_eq!(
            caret_line_col(
                text,
                Some(egui::text::CCursorRange::one(egui::text::CCursor::new(13)))
            ),
            Some(Caret {
                line: 2,
                column: 2,
                index: 14
            })
        );
        assert_eq!(caret_line_col(text, None), None);
    }

    #[test]
    fn a_statement_owns_the_lines_the_engine_runs_and_nothing_else() {
        // The splitter keeps a leading comment inside the statement it precedes, so the marker and
        // the fill follow the engine's statement rather than the eye's idea of one.
        let text = "-- the pilot's own note\nselect 1;\nselect 2;\nselect 3;\n";
        assert_eq!(statement_lines(text, 0), Some((1, 2)));
        assert_eq!(statement_lines(text, 2), Some((4, 4)));
        assert_eq!(statement_lines(text, 3), None);
        assert_eq!(statement_lines("-- only a note\n", 0), None);
        // The caret in a statement runs that statement; in the trailing whitespace there is none.
        let second = text.find("select 2").unwrap() + 3;
        assert_eq!(caret_statement(text, Some(caret_in(text, second))), Some(1));
        assert_eq!(
            caret_statement(text, Some(caret_in(text, text.len()))),
            None
        );
        assert_eq!(caret_statement(text, None), None);
    }

    #[test]
    fn the_strip_facts_are_the_texts_own() {
        assert_eq!(text_facts("a\nb\nc"), "3 lines · LF · UTF-8");
        assert_eq!(text_facts("a\r\nb"), "2 lines · CRLF · UTF-8");
    }

    #[test]
    fn the_job_inks_every_token_kind_and_carries_the_whole_buffer() {
        let tokens = Tokens::of("dark");
        let text = "-- note\nselect 'x', 42 from customers;";
        let job = sql_job(&tokens, text);
        assert_eq!(job.text, text, "the job lays out the buffer itself");
        assert_eq!(job.wrap.max_width, f32::INFINITY, "the design never wraps");
        let ink = |needle: &str| {
            let section = job
                .sections
                .iter()
                .find(|section| {
                    job.text[section.byte_range.start.0..section.byte_range.end.0].contains(needle)
                })
                .unwrap_or_else(|| panic!("no section {needle:?}"));
            (section.format.color, section.format.italics)
        };
        assert_eq!(ink("-- note"), (tokens.text_faint, true));
        assert_eq!(ink("select"), (tokens.accent, false));
        assert_eq!(ink("'x'"), (tokens.ok, false));
        assert_eq!(ink("42"), (tokens.info, false));
        assert_eq!(ink("customers"), (tokens.text, false));
    }

    #[test]
    fn the_bands_are_flush_and_the_register_starts_under_the_pane_border() {
        let ctx = egui::Context::default();
        let mut world = world("flush", "select 1 as a;\nselect 2 as b;\n");
        // The register's own bands only exist once something has run, and this test measures them.
        let _ = frame(&ctx, &mut world, Vec::new(), 1.0);
        let run = click_hit(&ctx, &mut world, "Run", 1.02);
        let f = frame(&ctx, &mut world, run, 1.05);
        let toolbar = f.layout.toolbar.unwrap();
        let sql = f.layout.sql.unwrap();
        let strip = f.layout.strip.unwrap();
        assert!((toolbar.top() - 75.0).abs() < 0.5 && (toolbar.bottom() - 115.0).abs() < 0.5);
        assert!((sql.top() - 115.0).abs() < 0.5 && (sql.bottom() - 435.0).abs() < 0.5);
        assert!((strip.top() - 435.0).abs() < 0.5 && (strip.bottom() - 461.0).abs() < 0.5);
        assert_eq!(f.layout.band, None, "a run that has not failed has no band");
        assert_eq!(f.layout.strip_lower, None);
        let register_top = f.layout.register_top.unwrap();
        assert!(
            (register_top - 462.0).abs() < 0.5,
            "the pane border is the last pixel row: the register starts at {register_top}"
        );
        // The register's own tab row starts exactly there, and its footer ends on the pane's floor.
        assert!(
            f.band(register_top).is_some(),
            "the register paints nothing at {register_top}"
        );
        assert!(
            f.band(874.0 - crate::theme::HEAD_H).is_some(),
            "the footer is anchored to the pane's bottom"
        );
    }

    #[test]
    fn a_failed_statement_paints_the_band_between_the_strips_and_keeps_the_earlier_result() {
        let ctx = egui::Context::default();
        let mut world = world("failed", "select 1 as a;\nselect statuss from customers;\n");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let run = f.hit("Run");
        let f = frame(&ctx, &mut world, click(run), 1.1);
        let band = f.layout.band.unwrap_or_else(|| {
            panic!(
                "the failure did not paint the band: {:#?}",
                tab(&world).sql.results
            )
        });
        assert!((band.top() - 461.0).abs() < 0.5 && (band.bottom() - 517.0).abs() < 0.5);
        let lower = f.layout.strip_lower.unwrap();
        assert!((lower.top() - 517.0).abs() < 0.5 && (lower.bottom() - 543.0).abs() < 0.5);
        assert!((f.layout.register_top.unwrap() - 544.0).abs() < 0.5);
        // The band's own paint: the error fill, the error rule on its top edge, the driver's message.
        let tokens = Tokens::of("dark");
        assert!(f
            .fills
            .iter()
            .any(|(rect, fill)| *fill == tokens.error_fill && (rect.top() - 461.0).abs() < 0.5));
        assert!(f
            .lines
            .iter()
            .any(|(y, ink)| *ink == tokens.error && (y - 461.5).abs() < 0.6));
        assert!(
            f.shows("no such column: statuss"),
            "the driver's own message"
        );
        assert!(f.shows("· Ln 2, Col 1 · statement 2 · nothing was executed"));
        // The strip names the statement that stopped the run, and the register opens its own tab.
        assert!(f.shows("statement 2 of 2 not run"));
        // The register is the run's own view: its tab row names the statement that stopped it, and
        // the tab the user was on keeps its grid.
        // The failed tab's facts are `error · {ms} ms`, and the ms is a real measurement, so the
        // assertion stops at the wording rather than pinning a number that varies by run.
        assert!(
            f.shows("error · "),
            "the register's tab for the failed statement"
        );
        assert!(f.shows("2 of 2"), "the register's bar counts the run");
        // The earlier statement's result survives the failure.
        let results = &tab(&world).sql.results;
        assert_eq!(results.len(), 2);
        assert!(
            results[0].result.is_some(),
            "the first statement's rows are kept"
        );
        assert!(results[1].error.is_some());
        assert!(results[0].result.as_ref().unwrap().row_count() > 0);
    }

    #[test]
    fn the_gutter_numbers_and_the_code_share_the_designs_columns() {
        let ctx = egui::Context::default();
        let mut world = world("gutter", "select 1 as a;\nselect 2 as b;\nselect 3 as c;\n");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        assert_eq!(f.layout.rows.len(), 4, "three lines and the empty fourth");
        assert!(
            (f.layout.rows[1].top() - f.layout.rows[0].top() - 20.0).abs() < 0.5,
            "the design's row pitch is 20: {:?}",
            f.layout.rows
        );
        assert!((f.layout.rows[0].left() - 344.0).abs() < 0.5);
        // The numbers end at 384 and every row has one.
        for (row, number) in f.layout.rows.iter().zip(1..=4) {
            let rect = f.label(&number.to_string());
            // `--s5` before the code, so the column's right edge is the design's 384. epaint's
            // visual box carries the glyphs' own bearings, so this is a pixel's tolerance.
            assert!(
                (rect.right() - 384.0).abs() < 1.5,
                "number {number} ends at {}",
                rect.right()
            );
            assert!((rect.center().y - row.center().y).abs() < 1.0);
        }
        // The code starts at 408, one `--s5` past the gutter, and never wraps: the editor paints one
        // galley for the whole buffer.
        let code = f.label("select 1 as a;\nselect 2 as b;\nselect 3 as c;\n");
        assert!((code.left() - 408.0).abs() < 1.5, "code at {}", code.left());
        assert!((code.top() - 115.0).abs() < 1.5);
    }

    #[test]
    fn the_current_row_and_the_caret_statement_are_filled_and_marked() {
        let ctx = egui::Context::default();
        let mut world = world(
            "rows",
            "select 1 as a;\nselect 2 as b,\n       statuss from customers;\n",
        );
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let _ = frame(&ctx, &mut world, click(line_2(&f)), 1.1);
        let f = settle(&ctx, &mut world, 1.1);
        let tokens = Tokens::of("dark");
        let row = |line: usize| f.layout.rows[line - 1];
        // The caret's row is filled; the rows of no other statement are.
        let fill = |rect: Rect, ink: Color32| {
            f.fills.iter().any(|(r, fill)| {
                *fill == ink
                    && (r.top() - rect.top()).abs() < 0.5
                    && (r.left() - rect.left()).abs() < 0.5
            })
        };
        assert_eq!(world.1.caret.map(|caret| caret.line), Some(2));
        assert!(fill(row(2), tokens.panel_alt), "the current row");
        assert!(
            !fill(row(1), tokens.panel_alt),
            "the row above it is not the caret's"
        );
        // `.mk`: the caret's statement (lines 2..3) is marked, in accent, and no other row is.
        let mark = |rect: Rect| {
            f.fills.iter().any(|(r, ink)| {
                *ink == tokens.accent
                    && (r.left() - rect.left()).abs() < 0.5
                    && (r.top() - rect.top()).abs() < 0.5
                    && (r.width() - crate::theme::INDICATOR_W).abs() < 0.5
            })
        };
        assert!(mark(row(2)) && mark(row(3)), "the statement's own lines");
        assert!(!mark(row(1)), "the statement above carries no marker");
    }

    /// A multi-statement script runs blocking on the UI thread (the runner streams one statement),
    /// so it never creates a background job and the running state stays off — the contrast to
    /// [`a_background_run_paints_the_running_state`].
    #[test]
    fn a_blocking_script_shows_no_running_state() {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        let mut world = world(
            "running",
            "select 1 as a;\nselect statuss from customers;\n",
        );
        let _ = frame(&ctx, &mut world, Vec::new(), 1.0);
        let run = click_hit(&ctx, &mut world, "Run", 1.05);
        let f = frame(&ctx, &mut world, run, 1.2);
        // No `.prog` hairline: a wide, 2px accent rule along the editor's top edge.
        let sql = f.layout.sql.unwrap();
        assert!(
            !f.fills.iter().any(|(rect, fill)| *fill == tokens.accent
                && rect.height() <= 2.0
                && rect.width() > 100.0
                && rect.top() < sql.top() + 3.0),
            "the editor paints a progress hairline"
        );
        // No running chip, no Executing… pill, no running footer.
        assert!(!f.shows("Executing"));
        assert!(!f.shows("running"));
        assert!(!f.shows("Cancel"));
        // And no error ink anywhere in the toolbar band.
        let toolbar = f.layout.toolbar.unwrap();
        assert!(
            !f.fills.iter().any(|(rect, fill)| *fill == tokens.error_fill
                && rect.top() > toolbar.top() - 1.0
                && rect.bottom() < toolbar.bottom() + 1.0),
            "the toolbar is painted as a failure"
        );
    }

    /// A single statement goes to the background, so while it is in flight the editor paints the
    /// design's running state: the `.prog` hairline, the `Executing…` pill, and a Stop that can end
    /// the run. Deterministic without any engine seam — the click starts the job, and because this
    /// paint does not pump, `drain` never flips it to `Done`, so `running_for_tab` stays `Some`.
    #[test]
    fn a_background_run_paints_the_running_state() {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        let mut world = world("bg", "select 1 as a;");
        // Start the background run: the click routes the single statement to sql.runAsync, which
        // creates a Running job on the calling thread.
        let run = click_hit(&ctx, &mut world, "Run", 1.0);
        let _ = frame(&ctx, &mut world, run, 1.1);
        // Paint one frame without pumping, so the job is still running.
        let f = paint(&ctx, &mut world, Vec::new(), 1.2);
        // The `.prog` hairline: a wide, 2px accent rule along the editor's top edge.
        let sql = f.layout.sql.unwrap();
        assert!(
            f.fills.iter().any(|(rect, fill)| *fill == tokens.accent
                && rect.height() <= 2.0
                && rect.width() > 100.0
                && rect.top() < sql.top() + 3.0),
            "the editor paints a progress hairline while a run is in flight"
        );
        // The `Executing…` pill, and a Stop that is enabled to end the run.
        assert!(f.shows("Executing"), "the editor shows an Executing… pill");
        let stop = f.label("Stop");
        assert!(
            f.fills.iter().any(|(rect, fill)| *fill == tokens.panel_high
                && rect.contains(stop.center())),
            "Stop is enabled (filled) while a job runs"
        );
    }

    /// Lay a frame out, then click the control painted with `label`.
    fn click_hit(
        ctx: &egui::Context,
        world: &mut (Session, UiState, Tokens),
        label: &str,
        time: f64,
    ) -> Vec<egui::Event> {
        let f = frame(ctx, world, Vec::new(), time);
        click(f.hit(label))
    }

    #[test]
    fn the_caret_runs_the_statement_it_sits_in_not_the_first() {
        let ctx = egui::Context::default();
        let mut world = world("caret", "select 1 as a;\nselect 2 as b;\n");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let _ = frame(&ctx, &mut world, click(line_2(&f)), 1.1);
        let f = settle(&ctx, &mut world, 1.1);
        assert_eq!(world.1.caret.map(|caret| caret.line), Some(2));
        // The toolbar prints the caret the editor reports, and only that: the old assertion looked for
        // an *absent* position, so it passed with the read-out gone and pinned a column the frame
        // never painted.
        let caret = world.1.caret.expect("the editor reported its caret");
        let read_out: Vec<&String> = f
            .texts
            .iter()
            .map(|(_, text)| text)
            .filter(|text| text.starts_with("Ln "))
            .collect();
        assert_eq!(
            read_out,
            vec![&caret_note(caret)],
            "the toolbar's read-out is the caret the editor sits on"
        );
        let run = click_hit(&ctx, &mut world, "Run statement", 1.2);
        let _ = frame(&ctx, &mut world, run, 1.4);
        // The engine runs the statement as it stands, semicolon and all.
        assert_eq!(
            tab(&world).sql.statements,
            vec!["select 2 as b;".to_string()],
            "the caret's statement ran"
        );
        assert_eq!(tab(&world).sql.results.len(), 1);
        // And the other way: the first statement, from the first line.
        let f = frame(&ctx, &mut world, Vec::new(), 1.5);
        let _ = frame(&ctx, &mut world, click(line_1(&f)), 1.6);
        let _ = settle(&ctx, &mut world, 1.6);
        assert_eq!(world.1.caret.map(|caret| caret.line), Some(1));
        let run = click_hit(&ctx, &mut world, "Run statement", 1.7);
        let _ = frame(&ctx, &mut world, run, 1.9);
        assert_eq!(
            tab(&world).sql.statements,
            vec!["select 1 as a;".to_string()]
        );
    }

    #[test]
    fn the_toolbar_fits_the_pane_and_the_designs_full_set_would_not() {
        let ctx = egui::Context::default();
        let mut world = world("fit", "select 1;\nselect 2;\nselect 3;\n");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let toolbar = f.layout.toolbar.unwrap();
        let kept = [
            "Run",
            "Run statement",
            "Stop",
            "Explain",
            "Format",
            "Clear",
            "Generate SQL",
        ];
        let buttons = kept
            .iter()
            .map(|label| f.label(label))
            .fold(f32::MIN, |right, rect| right.max(rect.right()));
        let cluster = f
            .texts
            .iter()
            .filter(|(_, text)| text.starts_with("Ln ") || text.ends_with("statements"))
            .map(|(rect, _)| rect.left())
            .fold(f32::MAX, f32::min);
        assert!(
            cluster - buttons >= f32::from(design::S4),
            "the read-outs collide with the controls: buttons to {buttons}, read-outs from {cluster}"
        );
        assert!(cluster >= toolbar.left());
        let inner = 776.0 - 2.0 * f32::from(design::S5);
        // The design's own toolbar, at the same sizes, does not fit: that is why the connection
        // chip and the four report actions are cut.
        let mut measured = (0.0, 0.0);
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1440.0, 900.0),
                )),
                time: Some(1.1),
                ..Default::default()
            },
            |ui| {
                let kept: f32 = [
                    ("Run", Some(Mark::Play)),
                    ("Run statement", None),
                    ("Stop", Some(Mark::Stop)),
                    ("Explain", None),
                    ("Format", None),
                    ("Clear", None),
                    ("Generate SQL", None),
                ]
                .iter()
                .map(|(label, mark)| design::toolbar_button_width(ui, &world.2, label, *mark))
                .sum();
                let cut: f32 = ["Chart", "Pivot", "Save snippet", "Export ▾"]
                    .iter()
                    .map(|label| design::toolbar_button_width(ui, &world.2, label, None))
                    .sum();
                let chip = design::chip_width(ui, &world.2, "nook-demo.db");
                // Two `.vdiv`s: one before Explain, one before Generate SQL.
                let gaps = 13.0 * f32::from(design::S4);
                measured = (kept + gaps, kept + chip + cut + gaps);
            },
        );
        out.textures_delta.clear();
        assert!(
            measured.0 <= inner,
            "the kept toolbar is {} wide against {inner}",
            measured.0
        );
        assert!(
            measured.1 > inner,
            "the design's toolbar is {} wide against {inner}",
            measured.1
        );
    }

    #[test]
    fn stop_is_disabled_with_the_engines_own_reason() {
        let ctx = egui::Context::default();
        let mut world = world("stop", "select 1 as a;\n");
        assert_eq!(
            stop_reason(),
            nook_engine::find("sql.stop").unwrap().help,
            "the button says what the registry says"
        );
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let stop = f.hit("Stop");
        let before = (
            world.0.messages.len(),
            tab(&world).sql.text.clone(),
            world.1.caret,
            tab(&world).sql.results.len(),
        );
        // Hover it long enough for the tooltip — egui shows one only once the pointer has stopped
        // moving — then click it.
        let mut time = 1.1;
        let _ = frame(&ctx, &mut world, hover(stop), time);
        let mut seen = false;
        for _ in 0..4 {
            time += 0.4;
            let f = frame(&ctx, &mut world, Vec::new(), time);
            seen |= f.shows(stop_reason());
        }
        assert!(
            seen,
            "the engine's own reason is the Stop control's hover text"
        );
        // Clicking it does nothing at all: the pointer leaving the control takes the tooltip with it,
        // and the engine is not asked to do anything.
        let f = frame(&ctx, &mut world, click(stop), time + 0.1);
        assert!(
            !f.shows(stop_reason()),
            "the tooltip goes when the pointer moves: {:#?}",
            f.strings
        );
        assert_eq!(
            (
                world.0.messages.len(),
                tab(&world).sql.text.clone(),
                world.1.caret,
                tab(&world).sql.results.len(),
            ),
            before,
            "a disabled Stop runs nothing and logs nothing"
        );
    }

    #[test]
    fn enter_inserts_a_newline_when_no_popup_is_open() {
        let ctx = egui::Context::default();
        let mut world = world("enter", "select 1;");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let _ = frame(&ctx, &mut world, click(line_1(&f)), 1.1);
        let f = frame(&ctx, &mut world, Vec::new(), 1.2);
        assert_eq!(f.layout.popup, None, "no prefix is being typed");
        let f = frame(&ctx, &mut world, vec![key(Key::Enter)], 1.3);
        assert_eq!(tab(&world).sql.text, "select 1;\n");
        assert_eq!(f.layout.popup, None);
    }

    #[test]
    fn escape_dismisses_the_popup_and_takes_no_other_key() {
        let ctx = egui::Context::default();
        let mut world = world("escape", "select 1;\ncus");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let _ = frame(&ctx, &mut world, click(line_2(&f)), 1.1);
        let f = settle(&ctx, &mut world, 1.1);
        assert!(f.layout.popup.is_some(), "the prefix opened the popup");
        assert!(
            f.shows("TABLES"),
            "the candidate's group is named: {:#?}",
            f.strings
        );
        assert!(f.shows("customers"), "the candidate is named");
        // Escape closes it, and the next letter still reaches the text.
        let f = frame(&ctx, &mut world, vec![key(Key::Escape)], 1.3);
        assert_eq!(f.layout.popup, None);
        assert_eq!(tab(&world).sql.text, "select 1;\ncus");
        let _ = frame(&ctx, &mut world, vec![egui::Event::Text("t".into())], 1.4);
        assert_eq!(tab(&world).sql.text, "select 1;\ncust");
    }

    #[test]
    fn accepting_a_candidate_replaces_the_typed_prefix_and_keeps_the_rest() {
        let ctx = egui::Context::default();
        let mut world = world("accept", "select 1;\ncus");
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        let _ = frame(&ctx, &mut world, click(line_2(&f)), 1.1);
        let f = settle(&ctx, &mut world, 1.1);
        assert!(f.layout.popup.is_some(), "the prefix opened the popup");
        let expected = candidates(&mut world.0, "select 1;\ncus", "cus")
            .into_iter()
            .next()
            .expect("sql.complete offers the table")
            .text;
        let f = frame(&ctx, &mut world, vec![key(Key::Enter)], 1.3);
        assert_eq!(tab(&world).sql.text, format!("select 1;\n{expected}"));
        assert_eq!(f.layout.popup, None, "the accepted text does not reopen it");
    }

    /// The strip holds at every pane width the shell can leave it: the 236px the minimum window gives,
    /// and the narrower bands a panel dragged over the pane leaves, down to the 60px where the chips are
    /// all that is left of the row. The chips elide to the band, and a band too narrow for even one
    /// paints none of them rather than painting past the pane.
    #[test]
    fn the_strip_holds_at_the_narrowest_pane_the_app_permits() {
        for count in [1usize, 3, 6, 100] {
            let ctx = egui::Context::default();
            let script: String = (1..=count)
                .map(|index| {
                    format!("select id, city from customers where city = 'city {index}';\n")
                })
                .collect();
            let mut world = world(&format!("narrow{count}"), &script);
            for width in [narrow_pane().width(), 170.0, 120.0, 60.0] {
                let pane = pane_of_width(width);
                let f = frame_in(&ctx, &mut world, Vec::new(), 1.0, pane);
                let strip = f.layout.strip.expect("the strip");
                assert!(
                    (strip.width() - width).abs() < 0.5,
                    "the band is the pane's width, not {}",
                    strip.width()
                );
                let mut painted = 0;
                for (rect, text) in &f.texts {
                    if !(strip.top()..strip.bottom()).contains(&rect.center().y) {
                        continue;
                    }
                    painted += 1;
                    assert!(
                        rect.left() >= strip.left() - 0.5 && rect.right() <= strip.right() + 0.5,
                        "{count} statements on the {width}px pane painted {text:?} at {rect:?}, outside the strip {strip:?}"
                    );
                }
                if width == narrow_pane().width() {
                    assert!(
                        painted > 0,
                        "{count} statements on the {}px pane painted no chip at all",
                        narrow_pane().width()
                    );
                }
            }
        }
    }

    /// `.strip` is the design's `nowrap` flex row with the script's own facts pinned to the right, so
    /// three ordinary statements paint their chips over the facts' glyphs, four push the facts outside
    /// the pane and six drop them. The chips are bounded now — the facts keep the width they measured
    /// and the statements that do not fit are counted — which is what the pane's 776px can hold.
    #[test]
    fn the_strip_keeps_its_facts_and_bounds_the_chips() {
        for count in [3usize, 6] {
            let ctx = egui::Context::default();
            let script: String = (1..=count)
                .map(|index| {
                    format!("select id, city from customers where city = 'city {index}';\n")
                })
                .collect();
            let mut world = world(&format!("strip{count}"), &script);
            let _ = frame(&ctx, &mut world, Vec::new(), 1.0);
            let run = click_hit(&ctx, &mut world, "Run", 1.02);
            let f = frame(&ctx, &mut world, run, 1.05);
            let strip = f.layout.strip.expect("the strip");
            let facts = f
                .texts
                .iter()
                .find(|(_, text)| text.ends_with("UTF-8"))
                .expect("the strip's own facts")
                .0;
            assert!(
                facts.right() <= strip.right() && facts.right() <= pane().right(),
                "{count} statements painted the facts at {} outside the pane",
                facts.right()
            );
            // Every chip stays left of the facts. The chips are the only `N · …` runs in the band —
            // the register below has its own.
            let chips: Vec<&(Rect, String)> = f
                .texts
                .iter()
                .filter(|(rect, text)| {
                    strip.contains(rect.center())
                        && text.contains(" · ")
                        && !text.ends_with("UTF-8")
                })
                .collect();
            for (rect, text) in &chips {
                assert!(
                    rect.right() <= facts.left(),
                    "chip {text:?} at {} runs into the facts at {}",
                    rect.right(),
                    facts.left()
                );
                assert!(rect.left() >= strip.left());
            }
            // Nothing is dropped silently: a chip per statement, or a count for it.
            let counted: usize = f
                .texts
                .iter()
                .filter(|(rect, text)| strip.contains(rect.center()) && text.starts_with('+'))
                .map(|(_, text)| {
                    text.trim_start_matches('+')
                        .split_whitespace()
                        .next()
                        .and_then(|count| count.parse::<usize>().ok())
                        .unwrap_or(0)
                })
                .sum();
            assert_eq!(
                chips.len() + counted,
                count,
                "{count} statements: {} chips and {counted} counted",
                chips.len()
            );
        }
    }

    /// The register records the script the run *used* rather than the editor's text at the moment of
    /// the run. A rerun of what is on screen therefore has to clear the stale pill even when its split
    /// is unchanged — the old rule compared the splits, so the register kept the previous text and
    /// painted "the script changed since this run" against results from exactly that script.
    #[test]
    fn a_rerun_whose_split_is_unchanged_is_not_stale() {
        let ctx = egui::Context::default();
        let mut world = world("stale", "select 1 as a;\n");
        let index = world.0.active_tab.unwrap();
        let run = click_hit(&ctx, &mut world, "Run", 1.02);
        let _ = frame(&ctx, &mut world, run, 1.05);
        let first = tab(&world).sql.text.clone();
        assert!(
            !world.1.registers[&index].stale(&first),
            "a fresh run is not stale"
        );
        // A comment behind the statement: the splitter drops it, so both scripts split into the same
        // statement while their texts differ.
        let second = "select 1 as a;\n-- a note the splitter drops\n".to_owned();
        assert_eq!(
            nook_engine::cmd::sql::split(&first),
            nook_engine::cmd::sql::split(&second),
            "the two scripts split the same way"
        );
        world.0.tabs[index].sql.text = second.clone();
        let _ = frame(&ctx, &mut world, Vec::new(), 1.08);
        assert!(
            world.1.registers[&index].stale(&second),
            "the editor has moved on"
        );
        let run = click_hit(&ctx, &mut world, "Run", 1.1);
        let _ = frame(&ctx, &mut world, run, 1.15);
        assert_eq!(
            world.1.registers[&index].script,
            tab(&world).sql.ran_script,
            "the register is stamped with the script the run used"
        );
        assert!(
            !world.1.registers[&index].stale(&second),
            "the rerun cleared the pill: its rows are from this script"
        );
    }

    /// A run given a script of its own — the control channel's `engine.run`, the palette's — is
    /// stamped with *that* script and not the editor's: the register used to record the editor's text
    /// and call itself fresh for a run it did not describe.
    #[test]
    fn a_run_with_a_script_of_its_own_is_stamped_with_it() {
        let ctx = egui::Context::default();
        let mut world = world("channel", "select 1 as a;\n");
        let index = world.0.active_tab.unwrap();
        let _ = frame(&ctx, &mut world, Vec::new(), 1.0);
        world
            .0
            .run(
                "sql.run",
                serde_json::json!({ "sql": "select 2 as b;", "limit": 10 }),
            )
            .expect("the other script runs");
        let _ = frame(&ctx, &mut world, Vec::new(), 1.1);
        assert_eq!(
            world.1.registers[&index].script, "select 2 as b;",
            "the register stamps what the engine ran"
        );
        assert!(
            world.1.registers[&index].stale(&tab(&world).sql.text),
            "the editor never held that script"
        );
    }

    /// `Run statement` cuts the caret's statement out of the tab's own text, and the register is
    /// stamped with the text the request came from rather than with the cut: an unchanged,
    /// newline-terminated script used to paint `stale · the script changed since this run` the moment
    /// its own statement was run.
    #[test]
    fn running_one_statement_of_an_unchanged_script_is_not_stale() {
        let ctx = egui::Context::default();
        let mut world = world("statement", "select 1 as a;\n");
        let index = world.0.active_tab.unwrap();
        let run = click_hit(&ctx, &mut world, "Run statement", 1.02);
        let _ = frame(&ctx, &mut world, run, 1.05);
        assert!(
            !tab(&world).sql.results.is_empty(),
            "the statement ran: the test is about the stamp it left"
        );
        let text = tab(&world).sql.text.clone();
        assert!(
            !world.1.registers[&index].stale(&text),
            "the register is stamped with the text the request came from, not the statement it cut out"
        );
        let f = frame(&ctx, &mut world, Vec::new(), 1.06);
        assert!(
            !f.shows(crate::register::STALE_NOTE),
            "the register painted the stale pill over an unchanged script: {:#?}",
            f.strings
        );
    }

    /// The register's `Export` cannot finish on a query tab: it routes through `grid.export`, which
    /// reads the active tab's *data* grid and refuses this one after the file picker has already asked
    /// for a path. So it is painted disabled and carries the engine's own sentence — the rule `Stop`
    /// and the grid's `editable` gate follow.
    #[test]
    fn the_registers_export_is_disabled_with_the_engines_own_refusal() {
        let ctx = egui::Context::default();
        let mut world = world("export", "select 1 as a;\n");
        let _ = frame(&ctx, &mut world, Vec::new(), 1.0);
        let run = click_hit(&ctx, &mut world, "Run", 1.02);
        let _ = frame(&ctx, &mut world, run, 1.05);
        // The run is in the background, so the click's own frame paints the editor and the rows
        // land on the next one — which is what `frame` waits for.
        let f = frame(&ctx, &mut world, Vec::new(), 1.08);
        let export = f.label("Export");
        // Hover it long enough for the tooltip. egui hides a tooltip while a click is more recent
        // than the last pointer movement, and `Run` was clicked a moment ago — so the pointer has to
        // arrive at least the grace time after that click.
        let mut time = 1.3;
        let _ = frame(&ctx, &mut world, hover(export.center()), time);
        let mut seen = false;
        let mut painted = Vec::new();
        for _ in 0..4 {
            time += 0.4;
            let f = frame(&ctx, &mut world, Vec::new(), time);
            seen |= f.shows(nook_engine::cmd::grid::no_grid_reason());
            // A button that is offered is filled with its sheet — `panel_alt` at rest, the pointer's
            // fill under the hand; the disabled face paints nothing at all. (Every control in a band
            // also gets one transparent rect from the layout machinery, which paints nothing.)
            assert!(
                !f.fills.iter().any(|(rect, fill)| rect.width() < 300.0
                    && rect.contains(export.center())
                    && *fill != Color32::TRANSPARENT),
                "the disabled Export was filled like an offered control: {:#?}",
                f.fills
                    .iter()
                    .filter(|(rect, _)| rect.contains(export.center()))
                    .collect::<Vec<_>>()
            );
            painted = f.strings;
        }
        assert!(
            seen,
            "the engine's own refusal is the Export control's hover text: {painted:#?}"
        );
    }

    #[test]
    fn the_editor_keeps_the_tab_the_user_was_reading_across_a_rerun() {
        let ctx = egui::Context::default();
        let mut world = world("rerun", "select 1 as a;\nselect 2 as b;\n");
        let index = world.0.active_tab.unwrap();
        let run = click_hit(&ctx, &mut world, "Run", 1.05);
        let _ = frame(&ctx, &mut world, run, 1.2);
        assert_eq!(tab(&world).sql.results.len(), 2);
        // The user moves to the first result, then runs the same script again.
        world.1.registers.get_mut(&index).unwrap().active = 0;
        let _ = frame(&ctx, &mut world, Vec::new(), 1.3);
        let run = click_hit(&ctx, &mut world, "Run", 1.35);
        let _ = frame(&ctx, &mut world, run, 1.5);
        assert_eq!(
            world.1.registers[&index].active, 0,
            "a rerun of the same script keeps the tab the user was reading"
        );
    }

    #[test]
    fn probe_band_narrow() {
        let ctx = egui::Context::default();
        for width in [776.0, 236.0] {
            for band in [false, true] {
                let mut world = world(&format!("probe{width}"), "select 1;\n");
                world.1.ai_band = band;
                let pane = pane_of_width(width);
                let f = frame_in(&ctx, &mut world, Vec::new(), 1.0, pane);
                let mut worst = 0.0f32;
                let mut who = String::new();
                for (rect, text) in &f.texts {
                    if rect.top() > 120.0 && rect.top() < 560.0 && text.len() > 1 {
                        let over = rect.right() - pane.right();
                        if over > worst {
                            worst = over;
                            who = text.clone();
                        }
                    }
                }
                eprintln!(
                    "width {width} band {band}: worst overflow {worst:.1} on {who:?}; sql={:?}",
                    f.layout.sql
                );
            }
        }
    }

    #[test]
    fn probe_overflow() {
        let ctx = egui::Context::default();
        for width in [776.0, 236.0, 420.0] {
            let mut world = world(&format!("ov{width}"), "select 1;\n");
            world.1.ai_band = false;
            let pane = pane_of_width(width);
            let f = frame_in(&ctx, &mut world, Vec::new(), 1.0, pane);
            let mut worst = (0.0f32, String::new());
            for (rect, text) in &f.texts {
                if text.len() > 1 {
                    let over = rect.right() - pane.right();
                    if over > worst.0 {
                        worst = (over, text.clone());
                    }
                }
            }
            eprintln!("pane {width}: worst {:.1} on {:?}", worst.0, worst.1);
        }
    }

    /// The whole mention loop, as the user runs it: focus the prompt, type a partial `@cus`, the
    /// candidate list filters, a click on the chip replaces the live token in the prompt.
    #[test]
    fn typing_a_mention_filters_the_list_and_clicking_replaces_the_token() {
        let ctx = egui::Context::default();
        let mut world = world("mention-smoke", "select 1;\n");
        let open = click_hit(&ctx, &mut world, "Generate SQL", 1.05);
        let _ = frame(&ctx, &mut world, open, 1.2);
        let f = settle(&ctx, &mut world, 1.3);
        assert!(
            f.shows("@customers"),
            "the seeded catalog offers the table: {:#?}",
            f.strings
        );

        // Click into the prompt (the painted sample text marks its rect), then type the token.
        let prompt = f
            .texts
            .iter()
            .find(|(_, text)| text.contains("@users"))
            .map(|(rect, _)| *rect)
            .expect("the prompt is painted");
        let _ = frame(&ctx, &mut world, click(prompt.center()), 1.35);
        let focus_and_type = vec![egui::Event::Text("@cus".into())];
        let _ = frame(&ctx, &mut world, focus_and_type, 1.4);
        let f = settle(&ctx, &mut world, 1.5);
        assert!(
            world.1.ai_form.prompt.contains("@cus"),
            "the prompt holds the typed token: {:?}",
            world.1.ai_form.prompt
        );
        assert_eq!(
            world.1.ai_form.mention_filter.as_deref(),
            Some("cus"),
            "the live filter reached the form"
        );
        assert_eq!(
            world.1.ai_form.mentions.len(),
            1,
            "the catalog filtered to @customers: {:?}",
            world.1.ai_form.mentions
        );

        // The chip is painted; clicking it rewrites the prompt in place.
        let chip = f
            .texts
            .iter()
            .find(|(_, text)| text.starts_with("@customers"))
            .map(|(rect, _)| *rect)
            .expect("the filtered chip is painted");
        let _ = frame(&ctx, &mut world, click(chip.center()), 1.6);
        settle(&ctx, &mut world, 1.7);
        assert!(
            world.1.ai_form.prompt.contains("@customers "),
            "the live token was replaced: {:?}",
            world.1.ai_form.prompt
        );
        assert!(
            !world.1.ai_form.prompt.contains("@cuscustomers"),
            "the replacement did not concatenate: {:?}",
            world.1.ai_form.prompt
        );
    }

    #[test]
    fn the_generate_button_opens_the_band_and_closes_it_again() {
        let ctx = egui::Context::default();
        let mut world = world("ai-band", "select 1;\n");
        assert!(!world.1.ai_band, "the band starts closed");
        let open = click_hit(&ctx, &mut world, "Generate SQL", 1.05);
        // The click is answered by the dispatch that runs after the band has been painted, so the
        // band itself appears on the frame after — the repaint the dispatch asks for.
        let _ = frame(&ctx, &mut world, open, 1.2);
        let f = frame(&ctx, &mut world, Vec::new(), 1.3);
        assert!(world.1.ai_band, "the toolbar opens the band");
        assert!(
            f.shows("Generate SQL") && f.shows("tables"),
            "the band's own bands are painted: {:#?}",
            f.strings
        );
        // The mention list is the engine's own answer for the open connection.
        assert!(
            f.shows("@customers"),
            "the catalog's table is offered as a mention: {:#?}",
            f.strings
        );
        // The band is a band, not a takeover: the editor is still painted, below the toolbar and
        // with room of its own.
        let sql = f.layout.sql.expect("the editor is still painted");
        let toolbar = f.layout.toolbar.expect("the toolbar is still painted");
        assert!(
            sql.top() > toolbar.bottom(),
            "the editor starts below the toolbar: sql {sql:?}, toolbar {toolbar:?}"
        );
        assert!(
            sql.height() > 100.0,
            "the band leaves the editor room to be an editor: {sql:?}"
        );
        // The whole form is painted, footer included — the band measures itself, so nothing is
        // clipped by a guessed height.
        for needle in [
            "01", "PROVIDER", "02", "BASE URL", "03", "API KEY", "Generate", "Cancel",
        ] {
            assert!(
                f.shows(needle),
                "the band paints {needle:?}: {:#?}",
                f.strings
            );
        }
        assert!(
            f.label("Generate").bottom() <= sql.top(),
            "the footer sits above the editor, not on it"
        );
        let close = click_hit(&ctx, &mut world, "Close", 1.4);
        let _ = frame(&ctx, &mut world, close, 1.5);
        assert!(!world.1.ai_band, "Close puts it away");
    }

    #[test]
    fn a_provider_that_needs_no_key_shows_no_key_field() {
        // Ollama and vLLM take no key, so the column says so instead of offering a field whose value
        // would be sent to a local server as an `Authorization` header.
        let ctx = egui::Context::default();
        let mut world = world("ai-keyless", "select 1;\n");
        world.1.ai_band = true;
        world.1.ai_form = crate::app::AiForm {
            provider: nook_ai::Provider::Ollama,
            ..crate::app::AiForm::default()
        };
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        assert!(
            f.shows("not needed"),
            "the key column says the provider takes none: {:#?}",
            f.strings
        );
        assert!(
            !f.shows("paste a key"),
            "and offers no field to type one into: {:#?}",
            f.strings
        );
    }

    #[test]
    fn a_provider_with_a_model_list_picks_from_a_dropdown() {
        // The model step is a picker whenever the provider ships ids: the combo's button shows the
        // effective model (the provider's own default while the field is empty), and no free-text
        // field sits beside it to hold a second, disagreeing value.
        let ctx = egui::Context::default();
        let mut world = world("ai-model-picker", "select 1;\n");
        world.1.ai_band = true;
        world.1.ai_form = crate::app::AiForm {
            provider: nook_ai::Provider::OpenAi,
            ..crate::app::AiForm::default()
        };
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        assert!(
            f.shows("MODEL"),
            "the model step is painted: {:#?}",
            f.strings
        );
        // The closed combo shows the provider's default, because the field is empty and that is
        // what a request would send.
        assert!(
            f.shows("gpt-4o-mini"),
            "the combo shows the effective model: {:#?}",
            f.strings
        );
        assert!(
            f.shows("where the request goes"),
            "the provider's description line is painted: {:#?}",
            f.strings
        );
        assert!(
            !f.shows("this server serves what it was started with"),
            "the model description belongs to list-less providers' cells only: {:#?}",
            f.strings
        );
    }

    #[test]
    fn a_provider_without_a_model_list_keeps_the_free_text_field() {
        // vLLM ships no ids — it serves what it was started with — so the picker would be an empty
        // dropdown and the field is the only way to name a model.
        let ctx = egui::Context::default();
        let mut world = world("ai-model-fallback", "select 1;\n");
        world.1.ai_band = true;
        world.1.ai_form = crate::app::AiForm {
            provider: nook_ai::Provider::Vllm,
            ..crate::app::AiForm::default()
        };
        let f = frame(&ctx, &mut world, Vec::new(), 1.0);
        assert!(
            f.shows("this server serves what it was started with"),
            "the list-less note is painted: {:#?}",
            f.strings
        );
    }
}
