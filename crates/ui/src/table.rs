//! One table widget for every place a result set is shown: the data grid, query results, monitor
//! tables and the log. It is painted by hand instead of built from `egui::Grid` so that a row can
//! carry a background, a hover highlight, a dirty marker and per-cell hit testing.
//!
//! Layout: horizontal scroll wraps the header *and* the body so they move together while the header
//! stays put vertically; the body scrolls on its own below it.

use datacraft_core::Value;
use egui::{Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::theme::{Tokens, HEAD_H};

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
    /// Selected cell, kept across frames.
    pub selected: Option<(usize, usize)>,
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
    BeginEdit(usize, usize),
    Commit(usize, usize, String),
    Cancel,
    Sort(String),
}

pub struct TableView<'a> {
    pub columns: &'a [TableColumn],
    pub rows: &'a [Vec<Value>],
    /// Cells with an unsaved edit, as (row, column).
    pub pending: Vec<(usize, usize)>,
    /// Rows that are new and not yet written to the server.
    pub inserted: Vec<usize>,
    pub selected: Option<(usize, usize)>,
    pub sort: Option<(String, bool)>,
    pub editable: bool,
    pub sortable: bool,
    /// Row-number column on the left.
    pub gutter: bool,
    /// Height of the scrolling body.
    pub height: f32,
    /// Row height: the density setting through `theme::row_h`.
    pub row_h: f32,
    pub empty_text: &'a str,
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

fn ellipsize(ui: &Ui, text: &str, font: egui::FontId, max: f32) -> String {
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
    view.columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let mut width =
                text_width(ui, &column.name, header_font.clone()) + CELL_PAD * 2.0 + 18.0;
            if column.primary_key {
                width += 22.0;
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
    view: &TableView<'_>,
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
    if std::env::var("DATACRAFT_TABLE_DEBUG").is_ok() {
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
    let mut event = TableEvent::None;

    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;

            // Header: names, sort caret, primary-key tag.
            let (header_rect, _) =
                ui.allocate_exact_size(Vec2::new(total_w, HEAD_H), Sense::hover());
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
                let gutter_head =
                    egui::Rect::from_min_size(header_rect.min, Vec2::new(gutter_w, HEAD_H));
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
                    let tag = Rect::from_min_size(
                        egui::pos2(cell.right() - CELL_PAD - 18.0, cell.center().y - 7.0),
                        Vec2::new(18.0, 14.0),
                    );
                    ui.painter()
                        .rect_filled(tag, CornerRadius::same(3), tokens.accent_dim);
                    ui.painter().text(
                        tag.center(),
                        Align2::CENTER_CENTER,
                        "PK",
                        tokens.font(9.5),
                        tokens.accent,
                    );
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
                        let selected_row =
                            view.selected.map(|(r, _)| r == row_index).unwrap_or(false);

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
                            let editing = state.editing == Some((row_index, column_index));
                            let dirty = view.pending.contains(&(row_index, column_index)) || is_new;
                            let value = row.get(column_index).cloned().unwrap_or(Value::Null);
                            let response = ui.interact(
                                cell,
                                id.with(("cell", row_index, column_index)),
                                Sense::click(),
                            );
                            let cell_selected = view.selected == Some((row_index, column_index));

                            if cell_selected && !selected_row {
                                ui.painter().rect_filled(
                                    cell,
                                    CornerRadius::ZERO,
                                    tokens.selection,
                                );
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
                                if response.clicked() {
                                    event = TableEvent::Select(row_index, column_index);
                                }
                                if response.double_clicked() {
                                    event = TableEvent::BeginEdit(row_index, column_index);
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
                        if std::env::var("DATACRAFT_TABLE_DEBUG").is_ok() && row_index < 2 {
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
