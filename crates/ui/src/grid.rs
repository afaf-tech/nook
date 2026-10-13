//! The data grid: context toolbar, inline editing, pending-change drawer and the page footer.

use datacraft_engine::{Level, Session};
use egui::RichText;

use crate::app::UiState;
use crate::table::{TableColumn, TableEvent, TableView};
use crate::theme::{self as design, Tokens};

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
        .map(datacraft_engine::TableRef::key)
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
        .filter(|edit| edit.kind == datacraft_model::EditKind::SetCell)
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
        .filter(|edit| edit.kind == datacraft_model::EditKind::InsertRow)
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
            if design::ghost_button(ui, tokens, "+ Row", true)
                .on_hover_text("Append an empty row")
                .clicked()
            {
                command = Some(("grid.insertRow", serde_json::json!({"values": {}})));
            }
            let selected_row = state.table(tab_index).selected.map(|(r, _)| r);
            if design::ghost_button(ui, tokens, "Duplicate", selected_row.is_some())
                .on_hover_text("Copy the selected row")
                .clicked()
            {
                if let Some(row) = selected_row {
                    command = Some(("grid.duplicateRow", serde_json::json!({"row": row})));
                }
            }
            if design::ghost_button(ui, tokens, "Delete", selected_row.is_some())
                .on_hover_text("Mark the selected row for deletion")
                .clicked()
            {
                if let Some(row) = selected_row {
                    command = Some(("grid.deleteRow", serde_json::json!({"row": row})));
                }
            }

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
                    RichText::new(design::matches_label(total.unwrap_or(rows.len() as u64)))
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
        let (title, text) = read_only_note(&primary_key, error.is_some(), columns.len());
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
    // empty states this is, so the widget only needs room for its header.
    let empty = empty_state(rows.len(), filter_count);
    let available = match empty {
        EmptyState::None => (ui.available_height() - 96.0).max(140.0),
        _ => design::HEAD_H,
    };
    let (title, body) = empty_copy(&empty, &table_name);
    // The widget's own line only appears when it has no columns to head — a tab with no table
    // behind it — and the grid's copy would then name a table that is not there.
    let fallback = if specs.is_empty() {
        "No table is open in this tab.".to_owned()
    } else {
        title.clone()
    };
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
            let value = rows
                .get(row)
                .and_then(|r| r.get(column))
                .cloned()
                .unwrap_or(datacraft_core::Value::Null);
            state.table(tab_index).selected = Some((row, column));
            state
                .table(tab_index)
                .begin_edit(row, column, value.as_text());
        }
        TableEvent::Commit(row, column, text) => {
            let name = columns.get(column).cloned().unwrap_or_default();
            state.table(tab_index).stop_edit();
            command = Some((
                "grid.editCell",
                serde_json::json!({"row": row, "column": name, "value": text}),
            ));
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
                if design::accent_button(ui, tokens, "+ Row", true).clicked() {
                    command = Some(("grid.insertRow", serde_json::json!({"values": {}})));
                }
                if design::ghost_button(ui, tokens, "Refresh", true).clicked() {
                    reload = true;
                }
            }),
            EmptyState::None => {}
        }
    }

    // Selected cell: what it is and the two shortcuts that act on it.
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
                }
                None => {
                    ui.label(
                        RichText::new("Click a cell to select it, double-click to edit it.")
                            .size(12.0)
                            .color(tokens.text_faint),
                    );
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
                            ui.ctx().copy_text(pending_sql.join(";\n"));
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
        if let Err(e) = datacraft_engine::cmd::load_grid(session, tab_index) {
            session.log(Level::Error, e.to_string());
        }
    }
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

/// `.robanner`'s title and body: the reason `GridState::editable` is false, in the same order the
/// engine checks them, so the banner never blames the primary key for a page that failed to load.
fn read_only_note(
    primary_key: &[String],
    failed: bool,
    columns: usize,
) -> (&'static str, &'static str) {
    if failed {
        (
            "Editing is disabled — the page did not load",
            "DataCraft will not edit rows it has not read. Retry the page, or check the connection.",
        )
    } else if columns == 0 {
        (
            "Editing is disabled — the table's columns are unknown",
            "Reopen the table from the tree so DataCraft can read its shape, then edit it.",
        )
    } else if primary_key.is_empty() {
        (
            "Editing is disabled — this table has no primary key",
            "Without a unique key DataCraft cannot identify the single row to update or delete. Selection, copy and export stay available; run an UPDATE in the query editor if you really mean it.",
        )
    } else {
        (
            "Editing is disabled — the grid is not bound to a table",
            "Open a table from the tree to edit its rows; this page has no table behind it.",
        )
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
        let key = vec!["id".to_owned()];
        assert!(read_only_note(&key, true, 3).0.contains("did not load"));
        assert!(read_only_note(&key, false, 0)
            .0
            .contains("columns are unknown"));
        assert!(read_only_note(&[], false, 3).0.contains("no primary key"));
        assert!(read_only_note(&key, false, 3)
            .0
            .contains("not bound to a table"));
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
