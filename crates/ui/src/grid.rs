//! The data grid: context toolbar, inline editing, pending-change drawer and the page footer.

use egui::RichText;
use nook_engine::{Level, Session};

use crate::app::UiState;
use crate::table::{TableColumn, TableEvent, TableView};
use crate::theme::{self as design, Tokens};

/// The form band's columns: the name box, the `PK` badge, the value field, the `NULL` button. Fixed
/// so the fields of every row start at the same x, whatever the record holds.
const FORM_NAME_W: f32 = 132.0;
const FORM_BADGE_W: f32 = 36.0;
const FORM_FIELD_W: f32 = 320.0;
const FORM_NULL_W: f32 = 56.0;
/// One form row, and the header the band spends before its first field.
const FORM_ROW_H: f32 = 30.0;
const FORM_HEAD_H: f32 = 40.0;
/// The band's floor: a one-column record still shows its header and a row of context.
const FORM_MIN_H: f32 = 112.0;

pub fn show(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let Some(tab) = session.tabs.get(tab_index) else {
        ui.label("tab is gone");
        return;
    };
    let table_name = tab
        .grid
        .table
        .as_ref()
        .map(nook_engine::TableRef::key)
        .unwrap_or_else(|| "(no table)".into());
    let columns = tab.grid.result.columns.clone();
    let rows = tab.grid.result.rows.clone();
    let types = tab.grid.result.column_types.clone();
    let primary_key = tab.grid.primary_key.clone();
    let pending_statements: Vec<String> = tab
        .grid
        .pending
        .iter()
        .map(|edit| format!("{:?} · {}", edit.kind, edit.sql))
        .collect();
    // `Copy to SQL` copies the statements themselves, not the drawer's `kind · sql` lines.
    let pending_sql: Vec<String> = tab
        .grid
        .pending
        .iter()
        .map(|edit| edit.sql.clone())
        .collect();
    let dirty_cells: Vec<(usize, usize)> = tab
        .grid
        .pending
        .iter()
        .filter(|edit| edit.kind == nook_model::EditKind::SetCell)
        .filter_map(|edit| {
            columns
                .iter()
                .position(|name| name == &edit.column)
                .map(|index| (edit.row, index))
        })
        .collect();
    let inserted_rows: Vec<usize> = tab
        .grid
        .pending
        .iter()
        .filter(|edit| edit.kind == nook_model::EditKind::InsertRow)
        .map(|edit| edit.row)
        .collect();
    let page = tab.grid.page;
    let page_size = tab.grid.page_size;
    let order = tab.grid.order.clone();
    let filter = tab.grid.filter.clone();
    let total = tab.grid.total_rows;
    let elapsed = tab.grid.elapsed_ms;
    let truncated = tab.grid.result.truncated;
    let error = tab.grid.error.clone();
    let conn = tab.conn;
    let pending = pending_statements.len();
    let editable = tab.grid.editable();
    let filter_count = tab.grid.filter_count();
    let counts = tab.grid.pending_counts();

    let mut reload = false;
    let mut command: Option<(&str, serde_json::Value)> = None;

    let next_sort = |name: &str| {
        let active = order.contains(&format!("\"{name}\"")) || order.contains(name);
        let desc = active && !order.contains("DESC");
        (name.to_owned(), desc)
    };

    // The design stacks its bands with no gap between them — each one's hairline is the next one's
    // top edge — so the pane's stray default spacing must not add one. Precedent: `tree` and
    // `inspector` zero it for the same reason.
    ui.spacing_mut().item_spacing.y = 0.0;

    // Context toolbar: what this tab is, how to page it, what to do with it.
    design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new(&table_name)
                    .size(design::T_MD)
                    .color(tokens.text),
            );
            ui.add_space(2.0);
            // `.pkbadge` beside the name, and the `.taglite` that says the grid cannot be edited.
            if !primary_key.is_empty() {
                design::pk_badge(ui, tokens, "PK");
            }
            if !editable {
                design::tag_lite(ui, tokens, "read-only");
            }
            design::chip(ui, tokens, &format!("{} rows", rows.len()), tokens.text_dim);
            if truncated {
                let chip = design::chip(ui, tokens, "truncated", tokens.warn);
                chip.on_hover_text("The driver stopped at max_rows");
            }
            design::divider(ui, tokens);

            if design::ghost_button(ui, tokens, "First", true)
                .on_hover_text("First page")
                .clicked()
            {
                command = Some(("grid.firstPage", serde_json::json!({})));
            }
            if design::ghost_button(ui, tokens, "Prev", true)
                .on_hover_text("Previous page")
                .clicked()
            {
                command = Some(("grid.prevPage", serde_json::json!({})));
            }
            let per_page = page_size.max(1) as i64;
            let pages = total
                .map(|t| (t as i64 + per_page - 1) / per_page)
                .unwrap_or(0);
            let page_label = if pages > 0 {
                format!("page {} / {}", page + 1, pages)
            } else {
                format!("page {}", page + 1)
            };
            ui.label(RichText::new(page_label).size(12.0).color(tokens.text_dim));
            if design::ghost_button(ui, tokens, "Next", true)
                .on_hover_text("Next page")
                .clicked()
            {
                command = Some(("grid.nextPage", serde_json::json!({})));
            }
            if design::ghost_button(ui, tokens, "Last", true)
                .on_hover_text("Last page")
                .clicked()
            {
                command = Some(("grid.lastPage", serde_json::json!({})));
            }
            egui::ComboBox::from_id_salt("page_size")
                .selected_text(format!("{page_size} rows"))
                .width(96.0)
                .show_ui(ui, |ui| {
                    for size in [50usize, 100, 200, 500, 1000, 5000] {
                        if ui
                            .selectable_label(size == page_size, format!("{size} rows"))
                            .clicked()
                        {
                            command =
                                Some(("grid.setPageSize", serde_json::json!({"page_size": size})));
                        }
                    }
                });
            if design::ghost_button(ui, tokens, "Count", true)
                .on_hover_text("Ask the server how many rows the table holds")
                .clicked()
            {
                command = Some(("grid.countRows", serde_json::json!({})));
            }
            if design::ghost_button(ui, tokens, "Refresh", true)
                .on_hover_text("Reload this page")
                .clicked()
            {
                reload = true;
            }

            design::divider(ui, tokens);
            // The three mutating actions follow the same `editable` flag the widget does: on a grid
            // the engine would refuse, the design's read-only frame keeps only Filter and Export.
            if design::ghost_button(ui, tokens, "+ Row", editable)
                .on_hover_text("Append an empty row")
                .clicked()
            {
                command = Some(("grid.insertRow", serde_json::json!({"values": {}})));
            }
            let selected_row = state.table(tab_index).selected.map(|(r, _)| r);
            if design::ghost_button(ui, tokens, "Duplicate", editable && selected_row.is_some())
                .on_hover_text("Copy the selected row")
                .clicked()
            {
                if let Some(row) = selected_row {
                    command = Some(("grid.duplicateRow", serde_json::json!({"row": row})));
                }
            }
            if design::ghost_button(ui, tokens, "Delete", editable && selected_row.is_some())
                .on_hover_text("Mark the selected row for deletion")
                .clicked()
            {
                if let Some(row) = selected_row {
                    command = Some(("grid.deleteRow", serde_json::json!({"row": row})));
                }
            }

            // Form view: one record as a vertical form. A flag on the UI state, not the engine's:
            // the engine command is stateless and reads whatever row it is handed.
            if design::ghost_button(ui, tokens, "Form", true)
                .on_hover_text("Show the selected record as a form")
                .clicked()
            {
                state.form_view = !state.form_view;
            }
            design::divider(ui, tokens);

            // Right cluster: the `.pend` cluster pinned to the edge, then Export. With nothing
            // staged there is no cluster — `pending_cluster` returns `None` for an empty buffer.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match design::pending_cluster(ui, tokens, counts) {
                    Some(design::PendingAction::Apply) => {
                        command = Some(("grid.apply", serde_json::json!({})));
                    }
                    Some(design::PendingAction::Revert) => {
                        command = Some(("grid.revert", serde_json::json!({})));
                    }
                    None => {}
                }
                ui.menu_button(RichText::new("Export").size(12.5), |ui| {
                    for (format, label) in [
                        ("csv", "CSV"),
                        ("json", "JSON"),
                        ("sql", "INSERT script"),
                        ("markdown", "Markdown"),
                    ] {
                        if ui.button(label).clicked() {
                            state.open_export_grid(format);
                            ui.close();
                        }
                    }
                });
                design::divider(ui, tokens);
            });
        });
    });

    // `.fchips`: one chip per active filter, the add chip, the match count and `Clear all`.
    //
    // ponytail: egui has no dashed stroke, so the add chip draws a solid hairline at the design's
    // weight instead of its dashes; upgrade path: stroke the dashes by hand along the chip's top
    // and bottom edges.
    let field_id = egui::Id::new(("grid-filter-field", tab_index));
    let mut field_open = ui
        .data_mut(|d| d.get_temp::<bool>(field_id))
        .unwrap_or(false);
    let fchips = design::band_fill(
        ui,
        design::filter_row_h(design::FCHIP_H),
        tokens.panel_alt,
        design::S4,
        egui::Align::Center,
        |ui| {
            if filter_count > 0 {
                design::chip(ui, tokens, &filter, tokens.text);
            }
            if field_open || !state.filter_edit.is_empty() {
                let mut filter_edit = state.filter_edit.clone();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut filter_edit)
                        .desired_width(320.0)
                        .hint_text("WHERE clause, e.g. city = 'London'"),
                );
                if response.changed() {
                    state.filter_edit = filter_edit.clone();
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    command = Some(("grid.filter", serde_json::json!({"where": filter_edit})));
                    field_open = false;
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    field_open = false;
                }
            } else {
                let add = egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0, tokens.stroke_strong))
                    .corner_radius(egui::CornerRadius::same(design::R_CHIP))
                    .inner_margin(egui::Margin {
                        left: design::S4,
                        right: design::S4,
                        top: 1,
                        bottom: 1,
                    })
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("+ Add filter")
                                .size(design::T_XS)
                                .color(tokens.text_dim),
                        )
                    });
                if add.response.interact(egui::Sense::click()).clicked() {
                    field_open = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if filter_count > 0
                    && design::accent_button(ui, tokens, "Clear all", true).clicked()
                {
                    state.filter_edit.clear();
                    command = Some(("grid.clearFilter", serde_json::json!({})));
                }
                ui.label(
                    RichText::new(matches_note(total, rows.len()))
                        .size(design::T_XS)
                        .monospace()
                        .color(tokens.text_dim),
                );
            });
        },
    );
    ui.data_mut(|d| d.insert_temp(field_id, field_open));
    design::band_hairline(ui, tokens, fchips);

    // `.errband` over the page that failed, then the `.stalestrip` that says what was asked for.
    if let Some(e) = &error {
        design::error_band(
            ui,
            tokens,
            "Query failed — could not read page 1",
            e,
            |ui| {
                if design::accent_button(ui, tokens, "Retry", true).clicked() {
                    reload = true;
                }
                if design::ghost_button(ui, tokens, "Details", true).clicked() {
                    state.log_open = true;
                }
            },
        );
        design::stale_strip(
            ui,
            tokens,
            &stale_note(&filter, &order),
            &format!("page {}", page + 1),
        );
    }

    // `.robanner`: the grid is on screen but the engine would refuse every edit.
    if !editable {
        let (title, text) = read_only_note(
            error.is_some(),
            tab.grid.table.is_some(),
            tab.grid.columns.len(),
        );
        design::ro_banner(ui, tokens, title, text, |ui| {
            if design::ghost_button(ui, tokens, "Reload", true).clicked() {
                reload = true;
            }
        });
    }

    // The table itself.
    let specs: Vec<TableColumn> = columns
        .iter()
        .enumerate()
        .map(|(index, name)| {
            TableColumn::new(name.clone(), types.get(index).cloned().unwrap_or_default())
                .pk(primary_key.contains(name))
        })
        .collect();
    let selected = state.table(tab_index).selected;
    let sort = (!order.is_empty()).then(|| {
        let desc = order.contains("DESC");
        let name = order
            .trim_start_matches('"')
            .split('"')
            .next()
            .unwrap_or_default()
            .to_owned();
        (name, desc)
    });
    // The design keeps the head of an empty page: the `.empty` block below it says which of the two
    // empty states this is. `height: 0` asks the widget for the head alone — there is no body to
    // scroll, but the head still scrolls sideways, so a table wider than the pane is reachable
    // rather than clipped. The widget floors its own viewport at `HEAD_H`, so the block lands 28px
    // under the hairline as the design has it.
    let empty = empty_state(rows.len(), filter_count);
    // The form band shares the pane with the table, so its height is reserved *before* the table asks
    // for its own. A table that claims the pane leaves the form the 96px under it: the header and one
    // row, cut off mid-row at the bottom of the window. The record is read here, once, because the
    // reservation needs the field count and the band must not run the command a second time.
    let form = state
        .form_view
        .then(|| form_record(session, tab_index, state));
    let form_h = form
        .as_ref()
        .map_or(0.0, |record| form_height(record, ui.available_height()));
    let available = match empty {
        EmptyState::None => (ui.available_height() - 96.0 - form_h).max(140.0),
        _ => 0.0,
    };
    let (title, body) = empty_copy(&empty, &table_name);
    // The widget's own line only appears when it has no columns to head — a tab with no table
    // behind it — and the grid's copy would then name a table that is not there.
    let fallback = if specs.is_empty() {
        "No table is open in this tab.".to_owned()
    } else {
        title.clone()
    };
    // A grid the engine would refuse holds no editor: a reload that fails clears the result under an
    // open one, and a stale `editing` would bring the editor back if the next load succeeds.
    if !editable {
        state.table(tab_index).stop_edit();
    }
    let view = TableView {
        columns: &specs,
        rows: &rows,
        pending: dirty_cells.clone(),
        inserted: inserted_rows.clone(),
        selected,
        sort,
        editable,
        sortable: true,
        gutter: true,
        height: available,
        row_h: design::row_h(session.settings.density),
        empty_text: fallback.as_str(),
    };
    ui.add_space(2.0);
    let event = crate::table::show(
        ui,
        tokens,
        &view,
        state.table(tab_index),
        egui::Id::new(("grid", tab_index)),
    );
    match event {
        TableEvent::Select(row, column) => {
            state.table(tab_index).selected = Some((row, column));
            state.table(tab_index).stop_edit();
        }
        TableEvent::BeginEdit(row, column) => {
            // A grid that became read-only under the pointer — a reload that failed clears the
            // result — opens no editor: the engine would refuse the edit it staged.
            if editable {
                let value = rows
                    .get(row)
                    .and_then(|r| r.get(column))
                    .cloned()
                    .unwrap_or(nook_core::Value::Null);
                state.table(tab_index).selected = Some((row, column));
                state
                    .table(tab_index)
                    .begin_edit(row, column, value.as_text());
            }
        }
        TableEvent::Commit(row, column, text) => {
            let name = columns.get(column).cloned().unwrap_or_default();
            state.table(tab_index).stop_edit();
            if editable {
                command = Some((
                    "grid.editCell",
                    serde_json::json!({"row": row, "column": name, "value": text}),
                ));
            }
        }
        TableEvent::Cancel => state.table(tab_index).stop_edit(),
        TableEvent::Sort(name) => {
            let (column, desc) = next_sort(&name);
            command = Some((
                "grid.sort",
                serde_json::json!({"column": column, "desc": desc}),
            ));
        }
        TableEvent::None => {}
    }

    // Form view: the selected record as a vertical form, under the table it belongs to.
    if let Some(record) = &form {
        if let Some(edit) = form_band(ui, tokens, record, tab_index, state, form_h) {
            command = Some(edit);
        }
    }

    // `.empty`, painted after the widget so the head stays on screen. The widget's own fallback copy
    // only appears when it has no columns to head, which is the one case this block skips.
    if empty != EmptyState::None && !specs.is_empty() {
        match empty {
            EmptyState::NoMatch => empty_block(ui, tokens, &title, body, |ui| {
                if design::accent_button(ui, tokens, "Clear all filters", true).clicked() {
                    state.filter_edit.clear();
                    command = Some(("grid.clearFilter", serde_json::json!({})));
                }
                if design::ghost_button(ui, tokens, "Refresh", true).clicked() {
                    reload = true;
                }
            }),
            EmptyState::NoRows => empty_block(ui, tokens, &title, body, |ui| {
                if design::accent_button(ui, tokens, "+ Row", editable).clicked() {
                    command = Some(("grid.insertRow", serde_json::json!({"values": {}})));
                }
                if design::ghost_button(ui, tokens, "Refresh", true).clicked() {
                    reload = true;
                }
            }),
            EmptyState::None => {}
        }
    }

    // Selected cell: what it is, and the actions this grid can take on it.
    let hint = design::band(ui, tokens, |ui| {
        ui.horizontal_centered(|ui| {
            match state.table(tab_index).selected {
                Some((row, column)) => {
                    let name = columns.get(column).cloned().unwrap_or_default();
                    let type_name = types.get(column).cloned().unwrap_or_else(|| "text".into());
                    ui.label(
                        RichText::new(format!("row {} · {name}", row + 1))
                            .size(12.0)
                            .color(tokens.text),
                    );
                    design::chip(ui, tokens, &type_name, tokens.info);
                    // Both the keycap and the button name actions the engine refuses without a key —
                    // `grid.setNull` goes through `pk_values` like the editor does — so a read-only
                    // grid paints the design's own read-only frame instead of advertising them.
                    if editable {
                        design::keycap(ui, tokens, "Double-click");
                        ui.label(
                            RichText::new("edit in place")
                                .size(11.5)
                                .color(tokens.text_faint),
                        );
                        if design::ghost_button(ui, tokens, "Set NULL", true).clicked() {
                            command = Some((
                                "grid.setNull",
                                serde_json::json!({"row": row, "column": name}),
                            ));
                        }
                    } else {
                        design::tag_lite(ui, tokens, "read-only");
                    }
                }
                None => {
                    // The same refusal: there is no editor to open on a read-only grid.
                    let hint = if editable {
                        "Click a cell to select it, double-click to edit it."
                    } else {
                        "Click a cell to select it."
                    };
                    ui.label(RichText::new(hint).size(12.0).color(tokens.text_faint));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                design::chip(
                    ui,
                    tokens,
                    &match total {
                        Some(t) => format!("{t} rows in the table"),
                        None => "row count not measured".to_owned(),
                    },
                    tokens.text_dim,
                );
                design::chip(
                    ui,
                    tokens,
                    &format!("{} ms", elapsed.round()),
                    tokens.text_faint,
                );
                if let Some(index) = conn {
                    let name = session
                        .conns
                        .get(index)
                        .map(|c| c.cfg.name.clone())
                        .unwrap_or_default();
                    design::chip(ui, tokens, &name, tokens.text_faint);
                }
            });
        });
    });
    ui.painter().hline(
        hint.left()..=hint.right(),
        hint.top() + 0.5,
        egui::Stroke::new(1.0, tokens.stroke),
    );

    // Pending statements, exactly as they will be sent, and the drawer's `.dfoot`.
    if pending > 0 {
        egui::CollapsingHeader::new(
            RichText::new(design::pending_sentence(counts))
                .size(design::T_MD)
                .color(tokens.warn),
        )
        .id_salt(format!("pending-{tab_index}"))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(format!("pending-scroll-{tab_index}"))
                .max_height(120.0)
                .show(ui, |ui| {
                    for statement in &pending_statements {
                        ui.label(
                            RichText::new(statement)
                                .size(design::T_SM)
                                .monospace()
                                .color(tokens.text_dim),
                        );
                    }
                });
            // `.dfoot`: one transaction for the whole buffer, so the note says what Apply does and
            // the three actions copy the statements, drop them or run them.
            let foot = design::band_fill(
                ui,
                design::TOOLBAR_H,
                tokens.panel_alt,
                design::S4,
                egui::Align::Center,
                |ui| {
                    ui.label(
                        RichText::new("one transaction · rolled back on the first failure")
                            .size(design::T_XS)
                            .monospace()
                            .color(tokens.text_dim),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if design::primary_button(
                            ui,
                            tokens,
                            &design::apply_changes_label(pending),
                            true,
                        )
                        .on_hover_text("Run the pending statements in one transaction")
                        .clicked()
                        {
                            command = Some(("grid.apply", serde_json::json!({})));
                        }
                        if design::ghost_button(ui, tokens, "Revert", true)
                            .on_hover_text("Throw the pending statements away")
                            .clicked()
                        {
                            command = Some(("grid.revert", serde_json::json!({})));
                        }
                        if design::ghost_button(ui, tokens, "Copy to SQL", true)
                            .on_hover_text("Copy the pending statements to the clipboard")
                            .clicked()
                        {
                            // Every statement already ends in `;`, so they are joined by a newline:
                            // a `;` separator would leave an empty statement between each pair.
                            ui.ctx().copy_text(copy_sql(&pending_sql));
                        }
                    });
                },
            );
            design::band_top_hairline(ui, tokens, foot);
        });
    }

    if let Some((id, params)) = command {
        if let Some(conn_index) = session.tabs[tab_index].conn {
            let mut params = params;
            if let Some(map) = params.as_object_mut() {
                map.insert(
                    "conn".into(),
                    serde_json::json!(session.conns[conn_index].cfg.id),
                );
            }
            match session.run(id, params) {
                Ok(_) => {
                    if id == "grid.editCell" && state.apply_after_edit {
                        state.apply_after_edit = false;
                        if let Err(e) = session.run("grid.apply", serde_json::json!({})) {
                            session.log(Level::Error, e.to_string());
                        }
                    }
                }
                Err(e) => session.log(Level::Error, format!("{id}: {e}")),
            }
        }
    }
    if reload {
        if let Err(e) = nook_engine::cmd::load_grid(session, tab_index) {
            session.log(Level::Error, e.to_string());
        }
    }
}

/// The record the form is showing, read once per frame: the table's `available` height needs the
/// field count before the table is laid out, and the band then paints this same answer.
fn form_record(
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
) -> Result<serde_json::Value, String> {
    let selected_row = state.table(tab_index).selected.map(|(r, _)| r).unwrap_or(0);
    session
        .run("grid.formView", serde_json::json!({"row": selected_row}))
        .map_err(|e| e.to_string())
}

/// Height the form band takes from the pane: the header, one row per field, and room to see that
/// there is more — floored so a two-column record still reads as a form, and capped at half the pane
/// so a wide record cannot leave the table it belongs to a strip. The band scrolls for the rest.
fn form_height(record: &Result<serde_json::Value, String>, pane: f32) -> f32 {
    let fields = record
        .as_ref()
        .ok()
        .and_then(|out| out["fields"].as_array().map(Vec::len))
        .unwrap_or(0);
    let wanted = FORM_HEAD_H + (fields.max(1) + 1) as f32 * FORM_ROW_H;
    wanted.clamp(FORM_MIN_H, (pane * 0.5).max(FORM_MIN_H))
}

/// One cell of a form row. The width is reserved before the cell paints, so no row's columns depend on
/// what the frames before it measured: egui's grid learns a column's width a frame late, which moves
/// the fields of a record whose badge or `NULL` button is missing and jitters the rest by a pixel.
fn form_cell<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(width, FORM_ROW_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add(ui)
        },
    )
    .inner
}

/// The selected record as a vertical form, painted under the table it belongs to.
///
/// The engine command overlays the pending `SetCell` edits for the row, so the form shows what
/// `grid.apply` will send — the UI merges nothing. The row is passed explicitly: the engine's cursor
/// is never written, so a default there would silently show the first record.
///
/// Returns the edit the user made, if any; the caller runs it, the way the table's own events are run.
fn form_band(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    record: &Result<serde_json::Value, String>,
    tab_index: usize,
    state: &mut UiState,
    reserved: f32,
) -> Option<(&'static str, serde_json::Value)> {
    let mut command = None;
    let out = match record {
        Ok(out) => out,
        // A row outside the page is an honest error band, never an empty form.
        Err(e) => {
            design::band_hairline(ui, tokens, ui.available_rect_before_wrap());
            design::error_band(ui, tokens, "Form view failed", e, |ui| {
                if design::ghost_button(ui, tokens, "Close form", true).clicked() {
                    state.form_view = false;
                }
            });
            return None;
        }
    };
    let row = out["row"].as_u64().unwrap_or(0) as usize;
    let editable_form = out["editable"].as_bool().unwrap_or(false);
    design::band_hairline(ui, tokens, ui.available_rect_before_wrap());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Record {row}"))
                .size(design::T_MD)
                .color(tokens.text),
        );
        if !editable_form {
            design::tag_lite(ui, tokens, "read-only");
        }
    });
    if let Some(fields) = out["fields"].as_array() {
        // The band holds the height the pane reserved for it, not whatever is left over: the hint and
        // the pending drawer are painted after it, and a band that grew to the pane's end would push
        // their buttons out of reach. The rest of the fields scroll.
        egui::ScrollArea::vertical()
            .id_salt(("form_scroll", tab_index))
            .auto_shrink([false, false])
            .max_height((reserved - FORM_HEAD_H).max(FORM_ROW_H))
            .show(ui, |ui| {
                for f in fields {
                    let name = f["name"].as_str().unwrap_or_default().to_owned();
                    ui.horizontal(|ui| {
                        form_cell(ui, FORM_NAME_W, |ui| {
                            ui.label(
                                RichText::new(&name)
                                    .size(design::T_SM)
                                    .color(tokens.text_dim),
                            );
                        });
                        form_cell(ui, FORM_BADGE_W, |ui| {
                            if f["primary_key"].as_bool().unwrap_or(false) {
                                design::pk_badge(ui, tokens, "PK");
                            }
                        });
                        form_cell(ui, FORM_FIELD_W, |ui| {
                            // A fresh buffer per frame would drop every keystroke at frame
                            // end: the edit text lives in the widget state instead, keyed by
                            // tab+row+column, so another record never wears this one's text.
                            // The widget renders OUTSIDE `data_mut` — that closure holds the
                            // context's write lock, and TextEdit takes it again for its own
                            // state: recursive write lock = a hang in release, a deadlock
                            // panic in debug.
                            let field_id = egui::Id::new(("form_field", tab_index, row, &name));
                            let shown = f["display"].as_str().unwrap_or_default().to_owned();
                            let mut text = ui.ctx().data_mut(|d| {
                                d.get_temp(field_id).unwrap_or_else(|| shown.clone())
                            });
                            let edited = ui.add_enabled(
                                editable_form,
                                egui::TextEdit::singleline(&mut text).desired_width(FORM_FIELD_W),
                            );
                            ui.ctx().data_mut(|d| d.insert_temp(field_id, text.clone()));
                            if editable_form && edited.lost_focus() && text != shown {
                                command = Some((
                                    "grid.editCell",
                                    serde_json::json!({"row": row, "column": name, "value": text}),
                                ));
                            }
                        });
                        form_cell(ui, FORM_NULL_W, |ui| {
                            // Only a field with a value can be set to NULL, but the cell stays: a
                            // row that dropped it would move the field that follows it.
                            if editable_form
                                && !f["value"].is_null()
                                && design::ghost_button(ui, tokens, "NULL", true)
                                    .on_hover_text("Set this field to NULL")
                                    .clicked()
                            {
                                command = Some((
                                    "grid.setNull",
                                    serde_json::json!({"row": row, "column": name}),
                                ));
                            }
                        });
                    });
                }
            });
    }
    command
}

/// Which `.empty` block the grid paints under its head, if either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyState {
    /// No rows, but a filter is doing the excluding.
    NoMatch,
    /// No rows and nothing asked for them: the table itself is empty.
    NoRows,
    /// There are rows, so the block has nothing to say.
    None,
}

/// The empty state follows two facts only: whether the page came back empty, and whether a filter is
/// active. `GridState::filter_count` is the engine's answer to the second, so the chip row and the
/// block cannot disagree about it.
fn empty_state(rows: usize, filter_count: usize) -> EmptyState {
    match (rows, filter_count) {
        (0, 0) => EmptyState::NoRows,
        (0, _) => EmptyState::NoMatch,
        _ => EmptyState::None,
    }
}

/// `.empty`'s h4 and paragraph, in the design's two variants. The no-match body points at the WHERE
/// clause rather than at the design's filter chip, because this app's filter is one raw clause.
fn empty_copy(state: &EmptyState, table: &str) -> (String, &'static str) {
    match state {
        EmptyState::NoMatch => (
            "No rows match these filters.".to_owned(),
            "The filters above are excluding every row. Remove one, or edit the WHERE clause.",
        ),
        EmptyState::NoRows => (
            format!("{table} has no rows yet."),
            "Insert a row here, or write an INSERT in the query editor — nothing is staged until you apply.",
        ),
        EmptyState::None => (String::new(), ""),
    }
}

/// `.empty`: left-aligned, 28px above the h4 and 30px below the paragraph, with the actions under
/// them and no fill of its own.
fn empty_block(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    body: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 20,
            right: 20,
            top: 28,
            bottom: 30,
        })
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = f32::from(design::S3);
                ui.label(RichText::new(title).size(design::T_MD).color(tokens.text));
                ui.label(
                    RichText::new(body)
                        .size(design::T_SM)
                        .color(tokens.text_dim),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = f32::from(design::S3);
                    add(ui);
                });
            });
        });
}

/// The clipboard text for `Copy to SQL`: the staged statements in order, one per line. Each
/// `edit.sql` already ends in `;` (`record_edit` stores what it was handed), so nothing separates
/// them — a `;` here would add an empty statement between every pair.
fn copy_sql(statements: &[String]) -> String {
    statements.join("\n")
}

/// `.robanner`'s title and body: the reason `GridState::editable` is false, in the order it checks
/// them — bound to a table, a page that loaded, known columns, a primary key — and fed the same two
/// facts it decides on (the binding and the schema), so the banner never blames the primary key for
/// a page that failed to load or for a table whose shape is unknown. It is painted only when
/// `editable()` is false, so by the last branch the primary key is what is left standing.
fn read_only_note(
    failed: bool,
    bound: bool,
    schema_columns: usize,
) -> (&'static str, &'static str) {
    if !bound {
        (
            "Editing is disabled — the grid is not bound to a table",
            "Open a table from the tree to edit its rows; this page has no table behind it.",
        )
    } else if failed {
        (
            "Editing is disabled — the page did not load",
            "Nook will not edit rows it has not read. Retry the page, or check the connection.",
        )
    } else if schema_columns == 0 {
        (
            "Editing is disabled — the table's columns are unknown",
            "Reopen the table from the tree so Nook can read its shape, then edit it.",
        )
    } else {
        (
            "Editing is disabled — this table has no primary key",
            "Without a unique key Nook cannot identify the single row to update or delete. Selection, copy and export stay available; run an UPDATE in the query editor if you really mean it.",
        )
    }
}

/// `.fchips`' right-hand fact. `grid.countRows` is the only command that fills `GridState::total_rows`,
/// and `grid.filter`, `grid.clearFilter` and `grid.sort` clear it again because they change which rows
/// the grid is asking for — so there is no count to print until the server has counted, and the honest
/// fact is the page in hand rather than a match total that is really just this page's length.
fn matches_note(total: Option<u64>, rows: usize) -> String {
    match total {
        Some(n) => design::matches_label(n),
        None => format!(
            "{} {} on this page",
            crate::table::grouped(rows as u64),
            crate::table::plural(rows as u64, "row", "rows")
        ),
    }
}

/// `.stalestrip`'s note: the order and the filter the grid is still asking for, so a reload that
/// failed cannot hide what the last good page was read with.
///
/// ponytail: the engine clears the page on a failed load, so the strip carries the last good order
/// and filter but not the last good rows; upgrade path: keep the previous `ResultSet` in
/// `GridState` and paint it inside the design's `.stale` block.
fn stale_note(filter: &str, order: &str) -> String {
    let filter = if filter.trim().is_empty() {
        "no filter".to_owned()
    } else {
        format!("filter {filter}")
    };
    let order = if order.trim().is_empty() {
        "unsorted".to_owned()
    } else {
        format!("order by {order}")
    };
    format!("Stale page · the last good page was read with {filter} · {order}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_state_follows_the_rows_and_the_filter() {
        assert_eq!(empty_state(0, 1), EmptyState::NoMatch);
        assert_eq!(empty_state(0, 0), EmptyState::NoRows);
        assert_eq!(empty_state(4, 1), EmptyState::None);
        assert_eq!(empty_state(4, 0), EmptyState::None);
    }

    #[test]
    fn the_two_empty_variants_keep_the_designs_copy() {
        let (title, body) = empty_copy(&EmptyState::NoMatch, "main.customers");
        assert_eq!(title, "No rows match these filters.");
        assert!(body.contains("WHERE"), "the no-match body names the clause");
        let (title, _) = empty_copy(&EmptyState::NoRows, "main.customers");
        assert_eq!(title, "main.customers has no rows yet.");
        assert_eq!(empty_copy(&EmptyState::None, "main.customers").0, "");
    }

    #[test]
    fn the_read_only_banner_names_the_reason_the_engine_gave() {
        // The four ways `GridState::editable` comes out false, fed from the schema and the binding —
        // the two facts the caller has — so each branch is reachable from `grid::show`.
        assert!(read_only_note(false, false, 3)
            .0
            .contains("not bound to a table"));
        assert!(read_only_note(true, true, 3).0.contains("did not load"));
        assert!(read_only_note(false, true, 0)
            .0
            .contains("columns are unknown"));
        assert!(read_only_note(false, true, 3).0.contains("no primary key"));
    }

    #[test]
    fn the_copied_statements_keep_one_statement_per_line() {
        // `record_edit` stores each statement with its own `;`, so joining them on `;` left an empty
        // statement between every pair.
        let statements = [
            "INSERT INTO main.t (id) VALUES (1);".to_owned(),
            "UPDATE main.t SET a = 2 WHERE id = 1;".to_owned(),
        ];
        let sql = copy_sql(&statements);
        assert_eq!(sql, format!("{}\n{}", statements[0], statements[1]));
        assert_eq!(
            sql.split(';')
                .filter(|part| !part.trim().is_empty())
                .count(),
            2,
            "a separator added here would leave an empty statement behind"
        );
    }

    #[test]
    fn the_match_note_reports_only_what_is_known() {
        assert_eq!(matches_note(Some(1), 1), "1 match");
        assert_eq!(matches_note(Some(12_481), 50), "12,481 matches");
        assert_eq!(matches_note(None, 1), "1 row on this page");
        assert_eq!(matches_note(None, 50), "50 rows on this page");
    }

    /// The pane's bands are flush, and an empty page asks the widget for its head alone: the head's
    /// hairline is the `.empty` block's top edge and the block's own 28px padding is the first ink
    /// under it. A scroll area around that head would floor its viewport at egui's 64px minimum and
    /// push the block 72px further down, which is what this measures.
    ///
    /// The call shape mirrors `app::show`'s central pane — `CentralPanel::default_margins()` *and*
    /// the `.frame(egui::Frame::new().fill(tokens.panel))` that replaces its inner margin, which is
    /// what makes the pane's bands start at the window's origin — and the band stack mirrors this
    /// module's `show`, so the numbers here are the ones the app paints.
    #[test]
    fn the_pane_bands_are_flush_and_an_empty_page_gets_the_head_alone() {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(819.0, 900.0),
            )),
            ..Default::default()
        };
        let columns = [
            TableColumn::new("id", "INTEGER").pk(true),
            TableColumn::new("name", "TEXT"),
        ];
        let rows: Vec<Vec<nook_core::Value>> = Vec::new();
        let mut marks: Vec<(&str, f32, f32)> = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default_margins()
                .frame(egui::Frame::new().fill(tokens.panel))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let toolbar = design::band(ui, &tokens, |ui| {
                        ui.set_min_height(design::TOOLBAR_H);
                        ui.horizontal_centered(|ui| {
                            let _ = design::ghost_button(ui, &tokens, "Refresh", true);
                        });
                    });
                    // The pane's frame carries no margin, so the first band starts at the window's
                    // origin; with `default_margins()`' own 8px it would measure 8 here.
                    marks.push(("pane_origin", toolbar.top(), 0.0));
                    let fchips = design::band_fill(
                        ui,
                        design::filter_row_h(design::FCHIP_H),
                        tokens.panel_alt,
                        design::S4,
                        egui::Align::Center,
                        |ui| {
                            let _ = design::chip(ui, &tokens, "x", tokens.text);
                        },
                    );
                    design::band_hairline(ui, &tokens, fchips);
                    marks.push(("toolbar_to_fchips", fchips.top() - toolbar.bottom(), 0.0));

                    ui.add_space(2.0);
                    let view = TableView {
                        columns: &columns,
                        rows: &rows,
                        pending: Vec::new(),
                        inserted: Vec::new(),
                        selected: None,
                        sort: None,
                        editable: true,
                        sortable: true,
                        gutter: true,
                        // An empty page asks for the head alone.
                        height: 0.0,
                        row_h: design::row_h(nook_engine::Density::Compact),
                        empty_text: "head-only",
                    };
                    let mut state = crate::table::TableState::default();
                    let before = ui.cursor().top();
                    let _ = crate::table::show(
                        ui,
                        &tokens,
                        &view,
                        &mut state,
                        egui::Id::new("head-only"),
                    );
                    let after = ui.cursor().top();
                    marks.push(("head_only", after - before, design::HEAD_H));

                    let block = egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 20,
                            right: 20,
                            top: 28,
                            bottom: 30,
                        })
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = f32::from(design::S3);
                                ui.label(
                                    RichText::new("main.customers has no rows yet.")
                                        .size(design::T_MD)
                                        .color(tokens.text),
                                )
                                .rect
                                .top()
                            })
                            .inner
                        });
                    // The block starts at the head's hairline, and its own 28px puts the first ink there.
                    marks.push(("empty_block_gap", block.response.rect.top() - after, 0.0));
                    marks.push(("first_ink_below_head", block.inner - after, 28.0));
                });
        });
        // The frame paints into a texture delta the headless context never applies.
        out.textures_delta.clear();
        for (name, got, want) in marks {
            assert!(
                (got - want).abs() < 0.5,
                "{name}: measured {got}, the design wants {want}"
            );
        }
    }

    /// Every stroked box the frame painted with the form's field width, with the clip rect it was
    /// painted under: a band that scrolls paints the rows below its viewport too, and those are not
    /// fields the user can see.
    fn form_field_boxes(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Rect, egui::Rect)> {
        fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<(egui::Rect, egui::Rect)>) {
            match shape {
                egui::Shape::Rect(r) if r.stroke.width > 0.0 => out.push((r.rect, clip)),
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        walk(s, clip, out);
                    }
                }
                _ => {}
            }
        }
        let mut rects = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, clipped.clip_rect, &mut rects);
        }
        let mut fields: Vec<(egui::Rect, egui::Rect)> = rects
            .into_iter()
            .filter(|(r, _)| (r.width() - FORM_FIELD_W).abs() < 0.5)
            .collect();
        fields.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        fields
    }

    /// Every text run the frame painted *whole* under its clip rect, with that rect: a row the pane cuts
    /// off is not evidence that the row is on screen.
    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
        fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(text) => {
                    let rect = text.visual_bounding_rect();
                    if rect.top() >= clip.top() - 0.5 && rect.bottom() <= clip.bottom() + 0.5 {
                        out.push((text.galley.text().to_owned(), rect));
                    }
                }
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        walk(s, clip, out);
                    }
                }
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, clipped.clip_rect, &mut texts);
        }
        texts
    }

    /// A live SQLite connection holding `users`, one data tab open on it, and the tokens: what the grid
    /// paints from. `name` keeps each test's database its own — one file per process would be shared by
    /// the tests the harness runs in parallel. `selected` is the record the form shows.
    fn form_pane(name: &str, selected: usize, extra: usize) -> (Session, UiState, Tokens, usize) {
        let path = std::env::temp_dir().join(format!("nook-form-{}-{name}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut session = Session::new();
        session
            .conns
            .push(nook_engine::Conn::new(nook_core::ConnectionConfig::sqlite(
                path.to_string_lossy(),
            )));
        session.active_conn = Some(0);
        nook_engine::cmd::open_now(&mut session, 0).expect("the fixture database opens");
        // `extra` more text columns, for the record too wide to show in one band.
        let filler: String = (1..=extra).map(|i| format!(", c{i} TEXT")).collect();
        let names: String = (1..=extra).map(|i| format!(", c{i}")).collect();
        let values: String = (1..=extra).map(|i| format!(", 'v{i}'")).collect();
        nook_engine::cmd::run_sql(
            &mut session,
            0,
            &format!(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, note TEXT, city TEXT{filler});\
                 INSERT INTO users (id, email, note, city{names}) VALUES (1, 'a@b.c', 'hi', 'London'{values}), (2, 'd@e.f', NULL, 'Lima'{values});"
            ),
            Some(10),
        )
        .expect("the fixture schema is valid");
        nook_engine::cmd::refresh_catalog(&mut session, 0).expect("the catalog refreshes");
        let conn = session.conns[0].cfg.id.clone();
        session
            .run(
                "grid.open",
                serde_json::json!({"conn": conn, "table": "users"}),
            )
            .expect("the data tab opens");
        let index = session.active_tab.expect("a data tab is active");
        let mut state = UiState::default();
        state.form_view = true;
        state.table(index).selected = Some((selected, 0));
        (session, state, Tokens::of("dark"), index)
    }

    /// What one frame of the grid, with the form open, painted: the form's field boxes and every text
    /// run, with the pane they were painted in.
    type Painted = (
        Vec<(egui::Rect, egui::Rect)>,
        Vec<(String, egui::Rect)>,
        egui::Rect,
    );

    /// Two frames of the grid with the form open, in the pane the app's own window leaves — the window
    /// less the app bar, the tab strip and the status bar, which is what `app::show` hands the pane.
    /// Two frames, because egui carries a grid's column widths into the next frame: a form whose
    /// columns move between the frame the form opens on and the one after it is exactly what the user
    /// sees as unaligned.
    fn form_frames(name: &str, size: egui::Vec2, selected: usize, extra: usize) -> Vec<Painted> {
        let (mut session, mut state, tokens, index) = form_pane(name, selected, extra);
        let ctx = egui::Context::default();
        tokens.apply(&ctx);
        let chrome = design::APP_BAR_H + design::TAB_BAR_H + design::STATUS_H;
        let pane =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(size.x, size.y - chrome));
        let input = egui::RawInput {
            screen_rect: Some(pane),
            ..Default::default()
        };
        let mut frames = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(input.clone(), |ui| {
                egui::CentralPanel::default_margins()
                    .frame(egui::Frame::new().fill(tokens.panel))
                    .show(ui, |ui| {
                        show(ui, &mut session, index, &mut state, &tokens);
                    });
            });
            out.textures_delta.clear();
            frames.push((
                form_field_boxes(&out.shapes),
                painted_text(&out.shapes),
                pane,
            ));
        }
        frames
    }

    /// The band's reservation: a header, a row per field and one spare so the last row is not the
    /// pane's edge — floored so a one-field record still shows a form, and capped at half the pane so
    /// a wide record cannot leave the table it belongs to a strip. The band's inner scroll area is
    /// bounded by this same number, so what it does not use stays with the bands painted after it.
    #[test]
    fn the_form_band_reserves_a_floor_and_a_ceiling() {
        let record = |fields: usize| {
            let fields: Vec<serde_json::Value> = (0..fields)
                .map(|i| serde_json::json!({"name": format!("c{i}")}))
                .collect();
            Ok(serde_json::json!({ "fields": fields }))
        };
        assert_eq!(
            form_height(&record(4), 900.0),
            FORM_HEAD_H + 5.0 * FORM_ROW_H,
            "four fields and the header, plus a row to show there is more"
        );
        assert_eq!(
            form_height(&record(40), 600.0),
            300.0,
            "half the pane is the ceiling, however wide the record"
        );
        assert_eq!(
            form_height(&record(1), 200.0),
            FORM_MIN_H,
            "a floor, so a narrow record and a short pane still read as a form"
        );
        assert_eq!(
            form_height(&Err("the row is gone".to_owned()), 900.0),
            FORM_MIN_H,
            "an error band is a band too"
        );
    }

    /// The form band: the table's own height is cut by the band's before the table is laid out, so a
    /// record's fields are on screen rather than the 96px strip a table that claims the pane leaves
    /// under itself. The table keeps its floor, so the fix cannot starve the rows it shows.
    #[test]
    fn the_form_band_is_carved_out_of_the_pane_not_the_tables_leftovers() -> Result<(), String> {
        for (width, height) in [(1568.0, 543.0), (819.0, 900.0), (1000.0, 600.0)] {
            let (fields, texts, pane) = form_frames("carve", egui::vec2(width, height), 1, 0)
                .pop()
                .expect("two frames are painted");
            if fields.len() != 4 {
                return Err(format!(
                    "{width}x{height}: the form painted {} of its 4 fields",
                    fields.len()
                ));
            }
            for (field, clip) in &fields {
                if field.bottom() > pane.bottom() || field.top() < pane.top() {
                    return Err(format!(
                        "{width}x{height}: a field runs {}..{} in a pane that ends at {}",
                        field.top(),
                        field.bottom(),
                        pane.bottom()
                    ));
                }
                if !clip.contains_rect(*field) {
                    return Err(format!(
                        "{width}x{height}: a field at {}..{} is cut off at {}",
                        field.top(),
                        field.bottom(),
                        clip.bottom()
                    ));
                }
            }
            // The table's own rows are still painted above the form: the band takes its share, it does
            // not empty the table.
            if !texts.iter().any(|(text, _)| text == "a@b.c") {
                return Err(format!(
                    "{width}x{height}: the table painted no row above the form"
                ));
            }
        }
        Ok(())
    }

    /// The form's columns are fixed: every field starts at the same x, the name of each field shares a
    /// left edge, and a row whose `NULL` button is missing — the field already holds NULL — moves no
    /// other row. A record read through egui's own grid shifts these by the width of whatever the frame
    /// before it measured, which is the misalignment the user sees the moment the form opens.
    #[test]
    fn every_form_field_starts_at_the_same_x() -> Result<(), String> {
        for (frame, (fields, texts, _)) in form_frames("align", egui::vec2(819.0, 900.0), 1, 0)
            .into_iter()
            .enumerate()
        {
            let left = fields[0].0.left();
            for field in fields.iter().map(|(field, _)| field) {
                if (field.left() - left).abs() > 0.5 {
                    return Err(format!(
                        "frame {frame}: a field starts at {} where the first starts at {left}",
                        field.left()
                    ));
                }
            }
            // The name of each field is the leftmost ink on its own row, left of the field's box; the
            // `NULL` button is the last ink on the right of it. The `note` field holds NULL, so that
            // row paints no button where the other three do.
            let mut names = Vec::new();
            let mut buttons = Vec::new();
            for (field, _) in &fields {
                let row = |rect: &egui::Rect| {
                    (rect.center().y - field.center().y).abs() < FORM_ROW_H / 2.0
                };
                names.push(
                    texts
                        .iter()
                        .filter(|(_, rect)| rect.right() <= field.left() && row(rect))
                        .min_by(|a, b| a.1.left().total_cmp(&b.1.left()))
                        .map(|(text, rect)| (text.clone(), rect.left()))
                        .ok_or_else(|| {
                            format!("frame {frame}: the row at {} painted no name", field.top())
                        })?,
                );
                // The `NULL` button of the row, told apart from a field whose own display is the word
                // NULL by where it lands: a button comes after its field. Painted over the field, the
                // row counts as having no button at all — which is what an unaligned column looks like.
                buttons.extend(
                    texts
                        .iter()
                        .filter(|(text, rect)| {
                            text == "NULL" && row(rect) && rect.left() >= field.right()
                        })
                        .map(|(_, rect)| rect.left()),
                );
            }
            // A glyph's ink starts a pixel inside its box depending on the letter, so the labels are
            // compared with room for that bearing: a column that moved is tens of pixels out.
            const INK: f32 = 2.0;
            let head = names[0].1;
            for (name, left) in &names {
                if (left - head).abs() > INK {
                    return Err(format!(
                        "frame {frame}: the {name} label starts at {left}, the first at {head}"
                    ));
                }
            }
            if buttons.len() != 3 {
                return Err(format!(
                    "frame {frame}: the rows holding a value painted {} NULL buttons, not 3",
                    buttons.len()
                ));
            }
            for left in &buttons {
                if (left - buttons[0]).abs() > INK {
                    return Err(format!(
                        "frame {frame}: a NULL button starts at {left} where the first starts at {}",
                        buttons[0]
                    ));
                }
            }
        }
        Ok(())
    }

    /// A record with more fields than the band can hold: the band scrolls inside its own share of the
    /// pane, so the table above it is not pushed off the top of the window and the fields still to come
    /// are clipped by the band's viewport rather than painted over the pane's edge.
    #[test]
    fn a_wide_record_scrolls_inside_its_band() -> Result<(), String> {
        const FIELDS: usize = 28;
        let (fields, texts, pane) = form_frames("wide", egui::vec2(1000.0, 600.0), 1, FIELDS - 4)
            .pop()
            .expect("two frames are painted");
        let (_, clip) = fields.first().ok_or("the wide record painted no field")?;
        // Half the pane is the band's ceiling: past it the table it belongs to would be a strip.
        if clip.height() > pane.height() / 2.0 + FORM_HEAD_H + FORM_ROW_H {
            return Err(format!(
                "the band holds {} of a {} pane, past its half",
                clip.height(),
                pane.height()
            ));
        }
        if clip.bottom() > pane.bottom() + 0.5 {
            return Err(format!(
                "the band's viewport ends at {} in a pane that ends at {}",
                clip.bottom(),
                pane.bottom()
            ));
        }
        // A field the user can see is one the viewport holds whole; the band paints the rest into the
        // viewport's clip rect, which is how a scroll area shows part of what it holds. Those are the
        // fields that must be inside the pane.
        let visible: Vec<egui::Rect> = fields
            .iter()
            .filter(|(field, clip)| clip.contains_rect(*field))
            .map(|(field, _)| *field)
            .collect();
        for field in &visible {
            if field.bottom() > pane.bottom() || field.top() < pane.top() {
                return Err(format!(
                    "a visible field runs {}..{} in a pane that ends at {}",
                    field.top(),
                    field.bottom(),
                    pane.bottom()
                ));
            }
        }
        if visible.len() < 2 {
            return Err(format!("the band shows {} fields", visible.len()));
        }
        if visible.len() == FIELDS {
            return Err(format!(
                "the band shows all {FIELDS} fields, so the table lost its share of the pane"
            ));
        }
        // The table keeps its own row above the band. `a@b.c` is the record the form does *not* show,
        // so only the table can paint it.
        if !texts.iter().any(|(text, _)| text == "a@b.c") {
            return Err("the table painted no row above the wide form".to_owned());
        }
        Ok(())
    }

    #[test]
    fn the_stale_strip_carries_the_last_order_and_filter() {
        assert_eq!(
            stale_note("", ""),
            "Stale page · the last good page was read with no filter · unsorted"
        );
        assert_eq!(
            stale_note("status = 'live'", "\"id\" DESC"),
            "Stale page · the last good page was read with filter status = 'live' · order by \"id\" DESC"
        );
    }
}
