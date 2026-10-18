//! One table widget for every place a result set is shown: the data grid, query results, monitor
//! tables and the log. It is painted by hand instead of built from `egui::Grid` so that a row can
//! carry a background, a hover highlight, a dirty marker and per-cell hit testing.
//!
//! Layout: horizontal scroll wraps the header *and* the body so they move together while the header
//! stays put vertically; the body scrolls on its own below it.

use egui::{Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use nook_core::Value;

use crate::theme::{self as design, Tokens, HEAD_H};

const GUTTER_W: f32 = 54.0;
const CELL_PAD: f32 = 9.0;
const MIN_COL_W: f32 = 76.0;
const MAX_COL_W: f32 = 280.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableColumn {
    pub name: String,
    pub type_name: String,
    pub primary_key: bool,
}

impl TableColumn {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        TableColumn {
            name: name.into(),
            type_name: type_name.into(),
            primary_key: false,
        }
    }

    pub fn pk(mut self, primary_key: bool) -> Self {
        self.primary_key = primary_key;
        self
    }

    fn numeric(&self) -> bool {
        let t = self.type_name.to_ascii_uppercase();
        [
            "INT", "SERIAL", "NUMERIC", "DECIMAL", "REAL", "DOUBLE", "FLOAT", "MONEY",
        ]
        .iter()
        .any(|k| t.contains(k))
    }
}

/// Per-table view state that has to survive across frames.
#[derive(Debug, Clone, Default)]
pub struct TableState {
    /// The cursor cell of the selection: the live corner, and what "the selected cell" means
    /// to every caller that wants one cell.
    pub selected: Option<(usize, usize)>,
    /// The drag origin of the selection rectangle. A plain click collapses it onto `selected`.
    pub anchor: Option<(usize, usize)>,
    pub editing: Option<(usize, usize)>,
    pub buffer: String,
    focus_editor: bool,
}

impl TableState {
    /// Open the inline editor on a cell.
    pub fn begin_edit(&mut self, row: usize, column: usize, value: String) {
        self.editing = Some((row, column));
        self.buffer = value;
        self.focus_editor = true;
    }

    pub fn stop_edit(&mut self) {
        self.editing = None;
        self.buffer.clear();
        self.focus_editor = false;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableEvent {
    None,
    Select(usize, usize),
    /// The pointer moved the live corner of the rectangle: the anchor stays, the cursor moves.
    SelectExtend(usize, usize),
    BeginEdit(usize, usize),
    Commit(usize, usize, String),
    Cancel,
    Sort(String),
    /// Hide one column, by name (the head's context menu).
    HideColumn(String),
    /// Un-hide every hidden column of this table (the head's context menu).
    ShowAllColumns,
}

/// Right-click handler for one table row: the row index in, menu items out.
pub type RowMenu<'a> = &'a mut dyn FnMut(&mut egui::Ui, usize);

pub struct TableView<'a> {
    // `editable` is the widget's read-only switch: false for the inspector's preview, for the results
    // of a statement, and for a grid whose table the engine would refuse to edit (no primary key).
    // It gates the inline editor and the dirty stripe, so a read-only view cannot look editable.
    pub columns: &'a [TableColumn],
    pub rows: &'a [Vec<Value>],
    /// Cells with an unsaved edit, as (row, column).
    pub pending: Vec<(usize, usize)>,
    /// Rows that are new and not yet written to the server.
    pub inserted: Vec<usize>,
    pub selected: Option<(usize, usize)>,
    /// The drag origin of the selection rectangle, painted as `tokens.selection` over the
    /// rectangle with the cursor cell stronger.
    pub anchor: Option<(usize, usize)>,
    pub sort: Option<(String, bool)>,
    pub editable: bool,
    pub sortable: bool,
    /// Row-number column on the left.
    pub gutter: bool,
    /// Height of the scrolling body. `0` for a table that has no body at all: the head is painted on
    /// its own — still scrollable sideways — and the caller owns the space under it (the grid's
    /// `.empty` block does).
    pub height: f32,
    /// Row height: the density setting through `theme::row_h`.
    pub row_h: f32,
    pub empty_text: &'a str,
    /// Right-click menu for a row, as `Some(callback)` — called with the row index. `None` for the
    /// default of no menu; the design tab's column list uses it to open the per-column editors.
    pub row_menu: Option<RowMenu<'a>>,
}

fn text_width(ui: &Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
        .size()
        .x
}

/// `12481` reads as `12,481`: counts are set in tabular figures everywhere the design shows them.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `1 column` but `2 columns`.
pub fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_owned()
    } else {
        many.to_owned()
    }
}

/// Truncate to `max` characters with an ellipsis; used by the tree and the tab strip.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Truncate `text` to the width `max` with an ellipsis, so a cell or a pill prints what its own rect
/// has room for rather than overrunning it. The first run of text that fits is kept, so a caller that
/// reserves the width of the ellipsis is never given more than it asked for.
pub fn ellipsize(ui: &Ui, text: &str, font: egui::FontId, max: f32) -> String {
    if text_width(ui, text, font.clone()) <= max {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = format!("{}…", chars.iter().collect::<String>());
        if text_width(ui, &candidate, font.clone()) <= max {
            return candidate;
        }
    }
    "…".to_owned()
}

fn total_width(widths: &[f32]) -> f32 {
    widths.iter().sum()
}

/// A column is numeric when its values are, which is more reliable than the declared type:
/// `id INTEGER PRIMARY KEY` arrives as `INTEGER`, a computed sum as nothing at all.
fn numeric_column(rows: &[Vec<Value>], index: usize) -> bool {
    let mut seen = false;
    for value in rows.iter().filter_map(|row| row.get(index)) {
        match value {
            Value::Null => continue,
            Value::Int(_) | Value::UInt(_) | Value::Float(_) => seen = true,
            _ => return false,
        }
    }
    seen
}

fn column_widths(ui: &Ui, tokens: &Tokens, view: &TableView<'_>) -> Vec<f32> {
    let header_font = tokens.font(12.5);
    let cell_font = tokens.mono(12.5);
    let badge = design::pk_badge_width(ui, tokens, "PK");
    view.columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let mut width =
                text_width(ui, &column.name, header_font.clone()) + CELL_PAD * 2.0 + 18.0;
            if column.primary_key {
                // The `.pkbadge` itself plus the design's `gap: var(--s2)` before it.
                width += badge + f32::from(design::S2);
            }
            let longest = view
                .rows
                .iter()
                .take(400)
                .filter_map(|row| row.get(index))
                .map(|value| text_width(ui, &value.display(), cell_font.clone()) + CELL_PAD * 2.0)
                .fold(0.0_f32, f32::max);
            width = width.max(longest).clamp(MIN_COL_W, MAX_COL_W);
            if column.numeric() || numeric_column(view.rows, index) {
                width = width.min(160.0);
            }
            width
        })
        .collect()
}

/// Paint the table and report what the user did. At most one event per frame.
pub fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    view: &mut TableView<'_>,
    state: &mut TableState,
    id: egui::Id,
) -> TableEvent {
    if view.columns.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(view.empty_text)
                    .size(12.5)
                    .color(tokens.text_faint),
            );
        });
        return TableEvent::None;
    }

    let widths = column_widths(ui, tokens, view);
    // Rows and gridlines span the whole viewport, so a narrow table does not leave dead stripes.
    let viewport_w = ui.available_width().max(total_width(&widths));
    if std::env::var("NOOK_TABLE_DEBUG").is_ok() {
        eprintln!(
            "table: cols={} rows={} widths={:?} avail={:?} height={}",
            view.columns.len(),
            view.rows.len(),
            widths,
            ui.available_size(),
            view.height
        );
    }
    let gutter_w = if view.gutter { GUTTER_W } else { 0.0 };
    let total_w = total_width(&widths) + gutter_w;

    if view.height <= 0.0 {
        // No body, so nothing to scroll vertically — but the head alone must still move sideways,
        // or a table wider than the pane is clipped with no bar to reach the columns past the edge.
        // The area's height is the head's own, so the caller's `.empty` block still lands 28px under
        // the hairline; `auto_shrink` collapses the viewport to the content and the floor below only
        // keeps it from egui's 64px default should the bar style ever stop floating.
        return egui::ScrollArea::horizontal()
            .id_salt(id.with("h"))
            .auto_shrink([false, true])
            .min_scrolled_height(HEAD_H)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                head(ui, tokens, view, &widths, viewport_w, total_w, id)
            })
            .inner;
    }

    let mut event = TableEvent::None;
    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            event = head(ui, tokens, view, &widths, viewport_w, total_w, id);

            egui::ScrollArea::vertical()
                .id_salt(id.with("v"))
                .max_height(view.height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    for (row_index, row) in view.rows.iter().enumerate() {
                        let (row_rect, row_response) =
                            ui.allocate_exact_size(Vec2::new(total_w, view.row_h), Sense::click());
                        let full = egui::Rect::from_min_max(
                            row_rect.min,
                            egui::pos2(row_rect.left() + viewport_w, row_rect.bottom()),
                        );
                        let is_new = view.inserted.contains(&row_index);
                        // With a rectangle in play (anchor and cursor differ), the rectangle is
                        // the selection; the full-row bar would paint over it.
                        let rect_active =
                            view.selected.zip(view.anchor).is_some_and(|(s, a)| s != a);
                        let selected_row = !rect_active
                            && view.selected.map(|(r, _)| r == row_index).unwrap_or(false);

                        if selected_row {
                            ui.painter()
                                .rect_filled(full, CornerRadius::ZERO, tokens.selection);
                        } else if is_new {
                            ui.painter().rect_filled(
                                full,
                                CornerRadius::ZERO,
                                tokens.accent_dim.gamma_multiply(0.7),
                            );
                        } else if row_index % 2 == 1 {
                            ui.painter()
                                .rect_filled(full, CornerRadius::ZERO, tokens.grid_row_alt);
                        }
                        if row_response.hovered() && !selected_row {
                            ui.painter()
                                .rect_filled(full, CornerRadius::ZERO, tokens.hover);
                        }
                        if view.gutter {
                            let gutter = egui::Rect::from_min_size(
                                row_rect.min,
                                Vec2::new(gutter_w, view.row_h),
                            );
                            ui.painter().rect_filled(
                                gutter,
                                CornerRadius::ZERO,
                                tokens.panel_alt.gamma_multiply(0.6),
                            );
                        }
                        if is_new {
                            ui.painter().rect_filled(
                                Rect::from_min_size(row_rect.min, Vec2::new(2.0, view.row_h)),
                                CornerRadius::ZERO,
                                tokens.accent,
                            );
                        }

                        // Gutter: row number, plus a dot when the row has pending edits.
                        if view.gutter {
                            let gutter =
                                Rect::from_min_size(row_rect.min, Vec2::new(gutter_w, view.row_h));
                            ui.painter().text(
                                egui::pos2(gutter.right() - CELL_PAD, gutter.center().y),
                                Align2::RIGHT_CENTER,
                                format!("{}", row_index + 1),
                                tokens.mono(11.0),
                                tokens.text_faint,
                            );
                            if view.pending.iter().any(|(r, _)| *r == row_index) {
                                ui.painter().circle_filled(
                                    egui::pos2(gutter.left() + 10.0, gutter.center().y),
                                    3.0,
                                    tokens.warn,
                                );
                            }
                            ui.painter().vline(
                                gutter.right() - 0.5,
                                gutter.top()..=gutter.bottom(),
                                Stroke::new(1.0, tokens.grid_line),
                            );
                        }

                        let mut x = row_rect.left() + gutter_w;
                        for (column_index, column) in view.columns.iter().enumerate() {
                            let cell = Rect::from_min_size(
                                egui::pos2(x, row_rect.top()),
                                Vec2::new(widths[column_index], view.row_h),
                            );
                            // An editor only exists while the grid can be edited: a reload that
                            // fails clears the result under an open editor, and a `Commit` from it
                            // would be refused by the engine.
                            let editing =
                                view.editable && state.editing == Some((row_index, column_index));
                            let dirty = view.editable
                                && (view.pending.contains(&(row_index, column_index)) || is_new);
                            let value = row.get(column_index).cloned().unwrap_or(Value::Null);
                            let response = ui.interact(
                                cell,
                                id.with(("cell", row_index, column_index)),
                                Sense::click_and_drag(),
                            );
                            let (cursor, anchor) = (view.selected, view.anchor);
                            let in_rect = cursor.zip(anchor).is_some_and(|(c, a)| {
                                row_index >= a.0.min(c.0)
                                    && row_index <= a.0.max(c.0)
                                    && column_index >= a.1.min(c.1)
                                    && column_index <= a.1.max(c.1)
                            }) || cursor == Some((row_index, column_index));
                            let cell_selected = in_rect;

                            if cell_selected && !selected_row {
                                let fill = if cursor == Some((row_index, column_index)) {
                                    // The cursor cell is the strong corner: it must read as
                                    // "this is the cell", not just part of the rectangle.
                                    tokens.selection.gamma_multiply(1.35)
                                } else {
                                    tokens.selection
                                };
                                ui.painter().rect_filled(cell, CornerRadius::ZERO, fill);
                            }
                            if dirty {
                                ui.painter().rect_filled(
                                    cell,
                                    CornerRadius::ZERO,
                                    tokens.dirty.gamma_multiply(0.55),
                                );
                                ui.painter().rect_filled(
                                    Rect::from_min_size(cell.min, Vec2::new(2.0, cell.height())),
                                    CornerRadius::ZERO,
                                    tokens.warn,
                                );
                            }
                            if response.hovered() && !editing {
                                ui.painter().rect_stroke(
                                    cell,
                                    CornerRadius::ZERO,
                                    Stroke::new(1.0, tokens.stroke_strong),
                                    StrokeKind::Inside,
                                );
                            }

                            if editing {
                                let inner = cell.shrink2(Vec2::new(3.0, 2.0));
                                let editor = ui.put(
                                    inner,
                                    egui::TextEdit::singleline(&mut state.buffer)
                                        .font(tokens.mono(12.5))
                                        .margin(Vec2::new(4.0, 1.0))
                                        .desired_width(inner.width()),
                                );
                                if state.focus_editor {
                                    editor.request_focus();
                                    state.focus_editor = false;
                                }
                                let entered = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                if escaped {
                                    event = TableEvent::Cancel;
                                } else if entered
                                    || (editor.lost_focus() && event == TableEvent::None)
                                {
                                    event = TableEvent::Commit(
                                        row_index,
                                        column_index,
                                        state.buffer.clone(),
                                    );
                                }
                            } else {
                                let display = value.display();
                                let colour = match &value {
                                    Value::Null => tokens.text_faint,
                                    Value::Bytes(_) => tokens.info,
                                    _ if dirty => tokens.dirty_text,
                                    _ => tokens.text,
                                };
                                let font = tokens.mono(12.5);
                                let max = cell.width() - CELL_PAD * 2.0;
                                let text = ellipsize(ui, &display, font.clone(), max);
                                let (anchor, position) = if column.numeric()
                                    || numeric_column(view.rows, column_index)
                                {
                                    (
                                        Align2::RIGHT_CENTER,
                                        egui::pos2(cell.right() - CELL_PAD, cell.center().y),
                                    )
                                } else {
                                    (
                                        Align2::LEFT_CENTER,
                                        egui::pos2(cell.left() + CELL_PAD, cell.center().y),
                                    )
                                };
                                let painter = ui.painter().with_clip_rect(cell);
                                painter.text(position, anchor, text, font, colour);
                                if response.hovered() && display.len() > 60 {
                                    response.clone().on_hover_text(display);
                                }
                                // A drag paints the rectangle live: the pointer's cell becomes
                                // the cursor while the anchor stays where the drag began. A
                                // plain click collapses both onto the one cell; shift-click
                                // keeps the anchor and moves only the cursor.
                                if response.dragged()
                                    || (response.clicked()
                                        && ui.input(|i| i.modifiers.shift)
                                        && view.anchor.is_some())
                                {
                                    event = TableEvent::SelectExtend(row_index, column_index);
                                } else if response.clicked() {
                                    event = TableEvent::Select(row_index, column_index);
                                }
                                if view.editable && response.double_clicked() {
                                    event = TableEvent::BeginEdit(row_index, column_index);
                                }
                                if let Some(menu) = view.row_menu.as_deref_mut() {
                                    response.context_menu(|ui| menu(ui, row_index));
                                }
                            }

                            ui.painter().rect_stroke(
                                cell,
                                CornerRadius::ZERO,
                                Stroke::new(0.75, tokens.grid_line),
                                StrokeKind::Inside,
                            );
                            x += widths[column_index];
                        }

                        ui.painter().hline(
                            full.left()..=full.right(),
                            row_rect.bottom() - 0.5,
                            Stroke::new(0.5, tokens.grid_line),
                        );
                        if std::env::var("NOOK_TABLE_DEBUG").is_ok() && row_index < 2 {
                            eprintln!(
                                "row {row_index}: rect={:?} clip={:?} first={:?}",
                                row_rect,
                                ui.painter().clip_rect(),
                                view.rows[row_index].first().map(|v| v.display())
                            );
                        }
                    }
                });
        });

    event
}

/// The head: names, the sort caret and the `.pkbadge`, painted inside the caller's horizontal scroll
/// area — the one that carries the body, or the head's own when the caller asked for no body.
fn head(
    ui: &mut Ui,
    tokens: &Tokens,
    view: &TableView<'_>,
    widths: &[f32],
    viewport_w: f32,
    total_w: f32,
    id: egui::Id,
) -> TableEvent {
    let mut event = TableEvent::None;
    let gutter_w = if view.gutter { GUTTER_W } else { 0.0 };
    let badge_w = design::pk_badge_width(ui, tokens, "PK");
    // Header: names, sort caret, primary-key tag.
    let (header_rect, _) = ui.allocate_exact_size(Vec2::new(total_w, HEAD_H), Sense::hover());
    let header_fill = egui::Rect::from_min_max(
        header_rect.min,
        egui::pos2(header_rect.left() + viewport_w, header_rect.bottom()),
    );
    ui.painter().rect_filled(
        header_fill,
        CornerRadius {
            nw: tokens.radius,
            ne: tokens.radius,
            sw: 0,
            se: 0,
        },
        tokens.grid_header,
    );
    if view.gutter {
        let gutter_head = egui::Rect::from_min_size(header_rect.min, Vec2::new(gutter_w, HEAD_H));
        ui.painter()
            .rect_filled(gutter_head, CornerRadius::ZERO, tokens.panel_alt);
        ui.painter().text(
            gutter_head.center(),
            Align2::CENTER_CENTER,
            "#",
            tokens.font(11.0),
            tokens.text_faint,
        );
    }
    ui.painter().hline(
        header_fill.left()..=header_fill.right(),
        header_rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    let mut x = header_rect.left() + gutter_w;
    for (index, column) in view.columns.iter().enumerate() {
        let cell = Rect::from_min_size(
            egui::pos2(x, header_rect.top()),
            Vec2::new(widths[index], HEAD_H),
        );
        let response = ui
            .interact(cell, id.with(("head", index)), Sense::click())
            .on_hover_text(format!("{}  {}", column.name, column.type_name));
        if response.hovered() {
            ui.painter()
                .rect_filled(cell, CornerRadius::ZERO, tokens.hover);
        }
        // The head's context menu: hide this column, or bring every hidden one back. Only
        // offered when the caller allows sorting — the data grid — so the read-only previews
        // keep their full column set.
        if view.sortable {
            response.context_menu(|ui| {
                if ui
                    .button(format!("Hide column \"{}\"", column.name))
                    .clicked()
                {
                    event = TableEvent::HideColumn(column.name.clone());
                }
                if ui.button("Show all columns").clicked() {
                    event = TableEvent::ShowAllColumns;
                }
            });
        }
        let sorted = view
            .sort
            .as_ref()
            .filter(|(name, _)| name == &column.name)
            .map(|(_, desc)| *desc);
        let label = match sorted {
            Some(true) => format!("{} ▼", column.name),
            Some(false) => format!("{} ▲", column.name),
            None => column.name.clone(),
        };
        ui.painter().text(
            egui::pos2(cell.left() + CELL_PAD, cell.center().y),
            Align2::LEFT_CENTER,
            label,
            tokens.font(12.5),
            if sorted.is_some() {
                tokens.accent
            } else {
                tokens.text
            },
        );
        if column.primary_key {
            // `.pkbadge`, through the same definition the toolbar's badge uses.
            let tag = Rect::from_min_size(
                egui::pos2(
                    cell.right() - CELL_PAD - badge_w,
                    cell.center().y - design::MICRO_TAG_H / 2.0,
                ),
                Vec2::new(badge_w, design::MICRO_TAG_H),
            );
            design::paint_pk_badge(ui, tokens, tag);
        }
        ui.painter().vline(
            cell.right() - 0.5,
            cell.top()..=cell.bottom(),
            Stroke::new(1.0, tokens.grid_line),
        );
        if response.clicked() && view.sortable {
            event = TableEvent::Sort(column.name.clone());
        }
        x += widths[index];
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A headless frame the way the app drives one: no texture uploads, one screen rect.
    fn screen(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(819.0, 900.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn view<'a>(
        columns: &'a [TableColumn],
        rows: &'a [Vec<Value>],
        editable: bool,
        height: f32,
    ) -> TableView<'a> {
        TableView {
            columns,
            rows,
            pending: Vec::new(),
            inserted: Vec::new(),
            selected: None,
            anchor: None,
            sort: None,
            editable,
            sortable: true,
            gutter: true,
            height,
            row_h: design::row_h(nook_engine::Density::Compact),
            empty_text: "no rows",
            row_menu: None,
        }
    }

    /// Double-click the cell at `(row, column)` and report the event the widget produced. The first
    /// frame only lays the table out, so the second one can be pointed at the cell's real rect.
    fn double_click_cell(
        view: &mut TableView<'_>,
        id: egui::Id,
        row: usize,
        column: usize,
    ) -> TableEvent {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        let mut state = TableState::default();
        let mut out = ctx.run_ui(screen(Vec::new()), |ui| {
            let _ = show(ui, &tokens, view, &mut state, id);
        });
        // The frame paints into a texture delta the headless context never applies.
        out.textures_delta.clear();
        let cell = ctx
            .read_response(id.with(("cell", row, column)))
            .expect("the cell registered its rect in the first frame")
            .rect;
        let mut events = vec![egui::Event::PointerMoved(cell.center())];
        for _ in 0..2 {
            events.push(press(cell.center(), true));
            events.push(press(cell.center(), false));
        }
        let mut event = TableEvent::None;
        let mut out = ctx.run_ui(screen(events), |ui| {
            event = show(ui, &tokens, view, &mut state, id);
        });
        out.textures_delta.clear();
        event
    }

    /// The engine refuses `grid.editCell` on a grid with no primary key, so the widget must not offer
    /// the editor there — a `BeginEdit` the caller has to swallow is the UI advertising a dead action.
    #[test]
    fn a_read_only_view_opens_no_editor_on_a_double_click() {
        let columns = [
            TableColumn::new("id", "INTEGER").pk(true),
            TableColumn::new("email", "TEXT"),
        ];
        let rows = vec![vec![Value::Int(1), Value::text("a@b.c")]];
        let id = egui::Id::new("read-only-table");
        let event = double_click_cell(&mut view(&columns, &rows, false, 120.0), id, 0, 1);
        assert_ne!(
            event,
            TableEvent::BeginEdit(0, 1),
            "a read-only grid must not ask for an editor"
        );
        assert_eq!(
            event,
            TableEvent::Select(0, 1),
            "the click still selects the cell"
        );
    }

    /// The other half of the same contract: an editable grid does open the editor on a double click.
    #[test]
    fn an_editable_view_opens_the_editor_on_a_double_click() {
        let columns = [
            TableColumn::new("id", "INTEGER").pk(true),
            TableColumn::new("email", "TEXT"),
        ];
        let rows = vec![vec![Value::Int(1), Value::text("a@b.c")]];
        let id = egui::Id::new("editable-table");
        let event = double_click_cell(&mut view(&columns, &rows, true, 120.0), id, 0, 1);
        assert_eq!(event, TableEvent::BeginEdit(0, 1));
    }

    /// Right-clicking a row of a view that carries a `row_menu` must reach the callback with that
    /// row — the design tab's column menu is the only caller, and a wrong index there would edit
    /// the wrong column.
    #[test]
    fn a_secondary_click_reaches_the_row_menu_with_the_row() {
        let columns = [
            TableColumn::new("id", "INTEGER").pk(true),
            TableColumn::new("email", "TEXT"),
        ];
        let rows = vec![vec![Value::Int(1), Value::text("a@b.c")]];
        let mut picked: Option<usize> = None;
        let mut menu = |ui: &mut egui::Ui, row: usize| {
            picked = Some(row);
            let _ = ui.button("Rename column");
        };
        let mut table_view = view(&columns, &rows, false, 120.0);
        table_view.row_menu = Some(&mut menu);
        let id = egui::Id::new("menu-table");
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        let mut state = TableState::default();
        let mut out = ctx.run_ui(screen(Vec::new()), |ui| {
            let _ = show(ui, &tokens, &mut table_view, &mut state, id);
        });
        out.textures_delta.clear();
        let cell = ctx
            .read_response(id.with(("cell", 0, 1)))
            .expect("the cell registered its rect in the first frame")
            .rect;
        let events = vec![
            egui::Event::PointerMoved(cell.center()),
            egui::Event::PointerButton {
                pos: cell.center(),
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: cell.center(),
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ];
        let mut second = ctx.run_ui(screen(events), |ui| {
            let _ = show(ui, &tokens, &mut table_view, &mut state, id);
        });
        second.textures_delta.clear();
        assert_eq!(picked, Some(0), "the menu opens for the right-clicked row");
    }

    /// An empty page asks for the head alone, and that head must still be laid out at its full width
    /// inside a horizontal scroll area: the pane here is narrower than the table (54px of gutter plus
    /// five 76px columns), so a head clipped to the pane would leave the columns past its edge with
    /// no bar to reach them. It must also advance the caller by exactly the head's height, so the
    /// `.empty` block that follows lands under the hairline as the design has it.
    #[test]
    fn an_empty_page_still_reports_the_heads_rects() {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        let columns: Vec<TableColumn> = (0..5)
            .map(|index| TableColumn::new(format!("column_{index}"), "TEXT"))
            .collect();
        let rows: Vec<Vec<Value>> = Vec::new();
        let id = egui::Id::new("empty-page-table");
        let pane_w = 300.0;
        let mut state = TableState::default();
        let screen = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(pane_w, 400.0),
            )),
            events,
            ..Default::default()
        };
        let mut advanced = 0.0;
        let mut run = |events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(screen(events), |ui| {
                // `grid::show` zeroes the pane's spacing so its bands are flush; the measure below
                // must be the head's, not the default spacing of a wrapped row.
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                let before = ui.cursor().top();
                let mut inline = view(&columns, &rows, true, 0.0);
                let event = show(ui, &tokens, &mut inline, &mut state, id);
                assert_eq!(event, TableEvent::None);
                advanced = ui.cursor().top() - before;
            });
            // The frame paints into a texture delta the headless context never applies.
            out.textures_delta.clear();
            advanced
        };
        let advanced = run(Vec::new());
        assert!(
            (advanced - HEAD_H).abs() < 0.5,
            "the head-only page advanced {advanced}, not its head's {HEAD_H}"
        );
        let head = |index: usize| {
            ctx.read_response(id.with(("head", index)))
                .expect("the head's column registered its rect")
                .rect
        };
        let tail = columns.len() - 1;
        assert!(
            head(tail).right() > pane_w,
            "the head must keep its full width, past the pane it is drawn in"
        );
        for index in 0..columns.len() {
            assert!(
                (head(index).height() - HEAD_H).abs() < 0.5,
                "column {index} is not the head's height"
            );
            if index > 0 {
                assert!(
                    (head(index).left() - head(index - 1).right()).abs() < 0.5,
                    "the head's columns must tile the row"
                );
            }
        }

        // The bar is what makes the columns past the pane reachable: a wheel over the head moves it
        // sideways until the last column sits inside the pane. Without the scroll area the head
        // cannot move at all, whatever its width.
        let mut reached = false;
        for _ in 0..10 {
            let advanced = run(vec![
                egui::Event::PointerMoved(egui::pos2(pane_w / 2.0, HEAD_H / 2.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-40.0, 0.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::default(),
                },
            ]);
            assert!(
                (advanced - HEAD_H).abs() < 0.5,
                "the head-only page advanced {advanced}, not its head's {HEAD_H}"
            );
            if head(tail).right() <= pane_w + 0.5 {
                reached = true;
                break;
            }
        }
        assert!(
            reached,
            "the head never scrolled sideways, so the column at x={} is unreachable",
            head(tail).right()
        );
    }
}
