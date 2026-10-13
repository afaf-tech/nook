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

    let mut reload = false;
    let mut command: Option<(&str, serde_json::Value)> = None;

    let next_sort = |name: &str| {
        let active = order.contains(&format!("\"{name}\"")) || order.contains(name);
        let desc = active && !order.contains("DESC");
        (name.to_owned(), desc)
    };

    // Context toolbar: what this tab is, how to page it, what to do with it.
    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(&table_name).size(13.5).color(tokens.text));
            ui.add_space(2.0);
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

            // Right cluster: the two actions that matter while editing.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if pending > 0 {
                    format!("Apply {pending}")
                } else {
                    "Apply".to_owned()
                };
                if design::primary_button(ui, tokens, &label, pending > 0)
                    .on_hover_text("Run the pending statements in one transaction")
                    .clicked()
                {
                    command = Some(("grid.apply", serde_json::json!({})));
                }
                if design::ghost_button(ui, tokens, "Revert", pending > 0)
                    .on_hover_text("Throw the pending statements away")
                    .clicked()
                {
                    command = Some(("grid.revert", serde_json::json!({})));
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

    // Filter row: the WHERE clause, on its own line under the toolbar.
    let filter_band = design::band(ui, tokens, |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("Filter").size(12.0).color(tokens.text_dim));
            let mut filter_edit = state.filter_edit.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut filter_edit)
                    .desired_width(ui.available_width().min(460.0) - 90.0)
                    .hint_text("WHERE clause, e.g. city = 'London'"),
            );
            if response.changed() {
                state.filter_edit = filter_edit.clone();
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                command = Some(("grid.filter", serde_json::json!({"where": filter_edit})));
            }
            if !filter.is_empty() {
                design::chip(ui, tokens, &filter, tokens.info);
                if design::ghost_button(ui, tokens, "Clear", true).clicked() {
                    command = Some(("grid.clearFilter", serde_json::json!({})));
                }
            }
        });
    });
    ui.painter().hline(
        toolbar.left()..=filter_band.right(),
        filter_band.bottom() - 0.5,
        egui::Stroke::new(1.0, tokens.stroke),
    );

    if let Some(e) = &error {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("last load failed: {e}"))
                .size(12.0)
                .color(tokens.error),
        );
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
    // The table needs room for its own header plus the hint bar below it.
    let available = (ui.available_height() - 96.0).max(140.0);
    let view = TableView {
        columns: &specs,
        rows: &rows,
        pending: dirty_cells.clone(),
        inserted: inserted_rows.clone(),
        selected,
        sort,
        editable: true,
        sortable: true,
        gutter: true,
        height: available,
        row_h: design::row_h(session.settings.density),
        empty_text: "No rows on this page. Change the filter, turn the page, or insert a row.",
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

    // Pending statements, exactly as they will be sent.
    if pending > 0 {
        egui::CollapsingHeader::new(
            RichText::new(format!("Pending changes ({pending})"))
                .size(12.0)
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
                                .size(11.5)
                                .monospace()
                                .color(tokens.text_dim),
                        );
                    }
                });
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
