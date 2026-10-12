//! The data grid: paging, sorting, filtering, inline edits and the pending-changes list.

use datacraft_engine::{Level, Session};
use egui::RichText;

use crate::app::UiState;
use crate::theme::Tokens;
use crate::tree::value_color;

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
    let pending = tab.grid.pending.len();
    let page = tab.grid.page;
    let page_size = tab.grid.page_size;
    let order = tab.grid.order.clone();
    let filter = tab.grid.filter.clone();
    let total = tab.grid.total_rows;
    let elapsed = tab.grid.elapsed_ms;
    let truncated = tab.grid.result.truncated;
    let error = tab.grid.error.clone();
    let conn = tab.conn;

    let mut reload = false;
    let mut command: Option<(&str, serde_json::Value)> = None;

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(table_name.clone()).strong().size(13.0));
        ui.separator();
        if ui.button("Refresh").clicked() {
            reload = true;
        }
        if ui.button("First").on_hover_text("First page").clicked() {
            command = Some(("grid.firstPage", serde_json::json!({})));
        }
        if ui.button("Prev").on_hover_text("Previous page").clicked() {
            command = Some(("grid.prevPage", serde_json::json!({})));
        }
        ui.label(format!("page {}", page + 1));
        if ui.button("Next").on_hover_text("Next page").clicked() {
            command = Some(("grid.nextPage", serde_json::json!({})));
        }
        if ui.button("Last").on_hover_text("Last page").clicked() {
            command = Some(("grid.lastPage", serde_json::json!({})));
        }
        egui::ComboBox::from_id_salt("page_size")
            .selected_text(format!("{page_size} rows"))
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
        if ui
            .button("Σ Count")
            .on_hover_text("Count rows on the server")
            .clicked()
        {
            command = Some(("grid.countRows", serde_json::json!({})));
        }
        ui.separator();
        if ui.button("+ Row").on_hover_text("Insert a row").clicked() {
            command = Some(("grid.insertRow", serde_json::json!({"values": {}})));
        }
        let row_selected = state.selected_cell.map(|(r, _)| r);
        if ui
            .add_enabled(row_selected.is_some(), egui::Button::new("Duplicate"))
            .clicked()
        {
            if let Some(row) = row_selected {
                command = Some(("grid.duplicateRow", serde_json::json!({"row": row})));
            }
        }
        if ui
            .add_enabled(row_selected.is_some(), egui::Button::new("Delete"))
            .clicked()
        {
            if let Some(row) = row_selected {
                command = Some(("grid.deleteRow", serde_json::json!({"row": row})));
            }
        }
        ui.separator();
        let apply_label = if pending == 0 {
            "✔ Apply".to_string()
        } else {
            format!("✔ Apply ({pending})")
        };
        if ui
            .add_enabled(
                pending > 0,
                egui::Button::new(apply_label).fill(tokens.accent),
            )
            .on_hover_text("Run the pending statements in one transaction")
            .clicked()
        {
            command = Some(("grid.apply", serde_json::json!({})));
        }
        if ui
            .add_enabled(pending > 0, egui::Button::new("Revert"))
            .clicked()
        {
            command = Some(("grid.revert", serde_json::json!({})));
        }
        ui.separator();
        ui.label("Filter:");
        let mut filter_edit = state.filter_edit.clone();
        let response = ui.add(
            egui::TextEdit::singleline(&mut filter_edit)
                .desired_width(220.0)
                .hint_text("WHERE clause, e.g. city = 'London'"),
        );
        if response.changed() {
            state.filter_edit = filter_edit.clone();
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            command = Some(("grid.filter", serde_json::json!({"where": filter_edit})));
        }
        if !filter.is_empty() && ui.small_button("✖").on_hover_text("Clear filter").clicked() {
            command = Some(("grid.clearFilter", serde_json::json!({})));
        }
        ui.separator();
        ui.menu_button("Export", |ui| {
            for (format, label) in [
                ("csv", "CSV…"),
                ("json", "JSON…"),
                ("sql", "INSERT script…"),
                ("markdown", "Markdown…"),
            ] {
                if ui.button(label).clicked() {
                    state.open_export_grid(format);
                    ui.close();
                }
            }
        });
    });

    if let Some(e) = &error {
        ui.colored_label(tokens.error, format!("last load failed: {e}"));
    }

    ui.separator();

    if rows.is_empty() {
        ui.label(
            RichText::new(
                "No rows on this page. Change the filter, go to another page, or insert a row.",
            )
            .color(tokens.text_dim),
        );
    } else {
        let dirty_cells: Vec<(usize, String)> = session.tabs[tab_index]
            .grid
            .pending
            .iter()
            .filter(|edit| edit.kind == datacraft_model::EditKind::SetCell)
            .map(|edit| (edit.row, edit.column.clone()))
            .collect();

        egui::ScrollArea::both()
            .id_salt(format!("grid-{tab_index}"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(format!("grid-table-{tab_index}"))
                    .striped(false)
                    .min_col_width(90.0)
                    .show(ui, |ui| {
                        for (column_index, name) in columns.iter().enumerate() {
                            let is_key = primary_key.contains(name);
                            let sorted_here =
                                order.contains(&format!("\"{name}\"")) || order.contains(name);
                            let label = if is_key {
                                format!("{name} (pk)")
                            } else {
                                name.clone()
                            };
                            let label = if sorted_here {
                                format!("{label} ^")
                            } else {
                                label
                            };
                            let text = RichText::new(label).size(12.0).strong().color(tokens.text);
                            let response =
                                ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                            if response
                                .on_hover_text(format!(
                                    "{}\nclick to sort",
                                    types
                                        .get(column_index)
                                        .map(String::as_str)
                                        .unwrap_or("unknown")
                                ))
                                .clicked()
                            {
                                let desc = sorted_here && !order.contains("DESC");
                                command = Some((
                                    "grid.sort",
                                    serde_json::json!({"column": name, "desc": desc}),
                                ));
                            }
                        }
                        ui.end_row();

                        for (row_index, row) in rows.iter().enumerate() {
                            for (column_index, name) in columns.iter().enumerate() {
                                let value = row
                                    .get(column_index)
                                    .cloned()
                                    .unwrap_or(datacraft_core::Value::Null);
                                let is_dirty = dirty_cells
                                    .iter()
                                    .any(|(r, c)| *r == row_index && c == name);
                                let selected =
                                    state.selected_cell == Some((row_index, column_index));
                                let text = shorten(&value.display());
                                let color = value_color(&value, tokens);
                                let response = ui.selectable_label(
                                    selected,
                                    RichText::new(text).size(12.0).color(color),
                                );
                                if is_dirty {
                                    ui.painter().rect_filled(
                                        response.rect.expand(1.0),
                                        2.0,
                                        tokens.dirty,
                                    );
                                }
                                if response.clicked() {
                                    state.selected_cell = Some((row_index, column_index));
                                    state.cell_edit = if value.is_null() {
                                        String::new()
                                    } else {
                                        value.as_text()
                                    };
                                    state.cell_editing = false;
                                }
                                if response.double_clicked() {
                                    state.selected_cell = Some((row_index, column_index));
                                    state.cell_edit = value.as_text();
                                    state.cell_editing = true;
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    // Cell editor: type a value for the selected cell and press Enter.
    ui.separator();
    ui.horizontal(|ui| {
        if let Some((row, column)) = state.selected_cell {
            let name = columns.get(column).cloned().unwrap_or_default();
            let column_type = session.tabs[tab_index]
                .grid
                .columns
                .iter()
                .find(|c| c.name == name)
                .map(|c| c.type_name.clone())
                .unwrap_or_else(|| "text".into());
            ui.label(
                RichText::new(format!("row {row} · {name} ({column_type})")).color(tokens.text_dim),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.cell_edit)
                    .desired_width(320.0)
                    .hint_text("value, or NULL"),
            );
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                command = Some((
                    "grid.editCell",
                    serde_json::json!({
                        "row": row,
                        "column": name,
                        "value": state.cell_edit,
                    }),
                ));
            }
            if ui.button("Set NULL").clicked() {
                command = Some((
                    "grid.setNull",
                    serde_json::json!({"row": row, "column": name}),
                ));
            }
            if ui
                .button("Run the statement")
                .on_hover_text("Queue this single change and apply it")
                .clicked()
            {
                command = Some((
                    "grid.editCell",
                    serde_json::json!({"row": row, "column": name, "value": state.cell_edit}),
                ));
                state.apply_after_edit = true;
            }
        } else {
            ui.label(
                RichText::new("Click a cell to select it, double-click to edit it inline.")
                    .color(tokens.text_dim)
                    .size(12.0),
            );
        }
    });

    // Pending statements, exactly as they will be sent.
    if pending > 0 {
        ui.separator();
        egui::CollapsingHeader::new(
            RichText::new(format!("Pending changes ({pending})")).size(12.0),
        )
        .id_salt(format!("pending-{tab_index}"))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(format!("pending-scroll-{tab_index}"))
                .max_height(120.0)
                .show(ui, |ui| {
                    for edit in &session.tabs[tab_index].grid.pending {
                        ui.label(
                            RichText::new(format!("{:?} · {}", edit.kind, edit.sql))
                                .size(11.0)
                                .monospace()
                                .color(tokens.text_dim),
                        );
                    }
                });
        });
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} rows on this page · {} · {} ms{}{}",
                rows.len(),
                match total {
                    Some(t) => format!("{t} rows in the table"),
                    None => "row count not measured".into(),
                },
                elapsed.round(),
                if truncated {
                    " · truncated by max_rows"
                } else {
                    ""
                },
                match conn {
                    Some(index) => format!(
                        " · {}",
                        session
                            .conns
                            .get(index)
                            .map(|c| c.cfg.name.clone())
                            .unwrap_or_default()
                    ),
                    None => String::new(),
                }
            ))
            .size(11.0)
            .color(tokens.text_dim),
        );
    });

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

fn shorten(text: &str) -> String {
    let text = text.replace('\n', "\\n");
    if text.chars().count() > 48 {
        format!("{}…", text.chars().take(47).collect::<String>())
    } else {
        text
    }
}
