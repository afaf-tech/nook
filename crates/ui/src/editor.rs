//! The query editor: toolbar, SQL text, statement strip, results table, history and snippets.

use datacraft_engine::{Level, Session};
use egui::RichText;

use crate::app::UiState;
use crate::table::{self, TableColumn, TableView};
use crate::theme::{self as design, Tokens};

pub fn show(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let mut run_sql: Option<String> = None;
    let mut command: Option<(&str, serde_json::Value)> = None;

    let statements = datacraft_engine::cmd::sql::split(&session.tabs[tab_index].sql.text);
    let conn_name = session.tabs[tab_index]
        .conn
        .and_then(|i| session.conns.get(i))
        .map(|c| c.cfg.name.clone())
        .unwrap_or_else(|| "no connection".into());
    let error = session.tabs[tab_index].sql.error.clone();
    let result = session.tabs[tab_index].sql.last_result.clone();
    let result_label = session.tabs[tab_index].sql.result_label.clone();

    // Context toolbar.
    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            if design::primary_button(ui, tokens, "Run", true)
                .on_hover_text("Run the whole script (Cmd+Enter)")
                .clicked()
            {
                command = Some(("sql.run", serde_json::json!({})));
            }
            if design::ghost_button(ui, tokens, "Run statement", true)
                .on_hover_text("Run the statement the cursor sits in")
                .clicked()
            {
                command = Some(("sql.runStatement", serde_json::json!({"index": 0})));
            }
            design::divider(ui, tokens);
            if design::ghost_button(ui, tokens, "Format", true)
                .on_hover_text("Pretty-print the script (Cmd+Shift+F)")
                .clicked()
            {
                command = Some(("sql.format", serde_json::json!({})));
            }
            if design::ghost_button(ui, tokens, "Explain", true)
                .on_hover_text("Ask the server for the plan")
                .clicked()
            {
                command = Some(("sql.explain", serde_json::json!({})));
            }
            design::divider(ui, tokens);
            if design::ghost_button(ui, tokens, "Chart", true)
                .on_hover_text("Send the last result to the report view")
                .clicked()
            {
                command = Some(("report.chart", serde_json::json!({"chart": "column"})));
            }
            if design::ghost_button(ui, tokens, "Pivot", true).clicked() {
                command = Some(("report.pivot", serde_json::json!({})));
            }
            if design::ghost_button(ui, tokens, "Save snippet", true).clicked() {
                let name = format!("Query {}", session.snippets.len() + 1);
                command = Some(("snippet.save", serde_json::json!({"name": name})));
            }
            if design::ghost_button(ui, tokens, "Clear", true).clicked() {
                if let Some(tab) = session.tabs.get_mut(tab_index) {
                    tab.sql.text.clear();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "Export CSV", true).clicked() {
                    state.open_export_grid("csv");
                }
                design::chip(ui, tokens, &conn_name, tokens.info);
            });
        });
    });

    let shortcuts = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
    if shortcuts {
        command = Some(("sql.run", serde_json::json!({})));
    }

    // Text area: as tall as the script needs, never more than a third of the window.
    let lines = session.tabs[tab_index].sql.text.lines().count().max(6);
    let editor_height =
        (lines as f32 * 19.0 + 32.0).clamp(140.0, (ui.available_height() * 0.45).max(140.0));
    let frame = egui::Frame::new()
        .fill(tokens.background)
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 8,
            bottom: 8,
        });
    frame.show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(format!("editor-scroll-{tab_index}"))
            .max_height(editor_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut session.tabs[tab_index].sql.text)
                        .code_editor()
                        .desired_rows(lines)
                        .desired_width(f32::INFINITY)
                        .hint_text(
                            "SELECT * FROM customers WHERE city = 'London';\n\nCmd+Enter runs it, Cmd+Shift+F formats it.",
                        ),
                );
                if response.changed() {
                    state.editor_dirty = true;
                }
            });
    });

    // Statement strip: one chip per statement, plus the counters.
    let strip = design::band(ui, tokens, |ui| {
        ui.horizontal_centered(|ui| {
            let text = &session.tabs[tab_index].sql.text;
            design::chip(
                ui,
                tokens,
                &if statements.len() == 1 {
                    "1 statement".to_owned()
                } else {
                    format!("{} statements", statements.len())
                },
                tokens.text_dim,
            );
            design::chip(
                ui,
                tokens,
                &format!("{} lines", text.lines().count()),
                tokens.text_faint,
            );
            for (index, statement) in statements.iter().take(6).enumerate() {
                let label = table::clip(statement.trim(), 26);
                if design::ghost_button(ui, tokens, &format!("{index}: {label}"), true)
                    .on_hover_text(statement)
                    .clicked()
                {
                    command = Some(("sql.runStatement", serde_json::json!({"index": index})));
                }
            }
            if statements.len() > 6 {
                design::chip(
                    ui,
                    tokens,
                    &format!("+{} more", statements.len() - 6),
                    tokens.text_faint,
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(error) = &error {
                    ui.label(RichText::new(error).size(11.5).color(tokens.error));
                }
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=strip.right(),
        strip.top() + 0.5,
        egui::Stroke::new(1.0, tokens.stroke),
    );

    // Results.
    if let Some(result) = result {
        let specs: Vec<TableColumn> = result
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
        let head = design::band(ui, tokens, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("Result").size(12.5).color(tokens.text));
                if !result_label.is_empty() && !result_label.contains("rows") {
                    design::chip(ui, tokens, &result_label, tokens.text_faint);
                }
                design::chip(
                    ui,
                    tokens,
                    &format!("{} rows", result.rows.len()),
                    tokens.text_dim,
                );
                design::chip(
                    ui,
                    tokens,
                    &format!("{} ms", result.elapsed_ms.round()),
                    tokens.text_faint,
                );
                if result.affected > 0 {
                    design::chip(
                        ui,
                        tokens,
                        &format!("{} affected", result.affected),
                        tokens.ok,
                    );
                }
            });
        });
        ui.painter().hline(
            head.left()..=head.right(),
            head.bottom() - 0.5,
            egui::Stroke::new(1.0, tokens.stroke),
        );

        if result.rows.is_empty() {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("the statement ran and returned no rows")
                        .size(12.0)
                        .color(tokens.text_faint),
                );
            });
        } else {
            let view = TableView {
                columns: &specs,
                rows: &result.rows,
                pending: Vec::new(),
                inserted: Vec::new(),
                selected: state.table(tab_index).selected,
                sort: None,
                editable: false,
                sortable: false,
                gutter: true,
                height: (ui.available_height() - 30.0).max(120.0),
                row_h: design::row_h(session.settings.density),
                empty_text: "no rows",
            };
            crate::table::show(
                ui,
                tokens,
                &view,
                state.table(tab_index),
                egui::Id::new(("result", tab_index)),
            );
        }
    } else if error.is_none() {
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Write a statement and press Run, or Cmd+Enter.")
                    .size(12.0)
                    .color(tokens.text_faint),
            );
        });
    }

    // History and snippets side by side, small.
    let bottom = design::band(ui, tokens, |ui| {
        ui.columns(2, |columns| {
            columns[0].label(RichText::new("History").size(12.0).color(tokens.text));
            egui::ScrollArea::vertical()
                .id_salt(format!("history-{tab_index}"))
                .max_height(120.0)
                .show(&mut columns[0], |ui| {
                    for entry in session.history.iter().rev().take(40) {
                        let label = table::clip(&entry.sql.replace('\n', " "), 58);
                        if design::ghost_button(
                            ui,
                            tokens,
                            &format!("{label} · {:.0} ms", entry.elapsed_ms),
                            true,
                        )
                        .on_hover_text(&entry.sql)
                        .clicked()
                        {
                            run_sql = Some(entry.sql.clone());
                        }
                    }
                    if session.history.is_empty() {
                        ui.label(
                            RichText::new("nothing yet")
                                .size(11.0)
                                .color(tokens.text_faint),
                        );
                    }
                });
            columns[1].label(RichText::new("Snippets").size(12.0).color(tokens.text));
            egui::ScrollArea::vertical()
                .id_salt(format!("snippets-{tab_index}"))
                .max_height(120.0)
                .show(&mut columns[1], |ui| {
                    for snippet in &session.snippets {
                        if design::ghost_button(ui, tokens, &snippet.name, true)
                            .on_hover_text(&snippet.sql)
                            .clicked()
                        {
                            run_sql = Some(snippet.sql.clone());
                        }
                    }
                    if session.snippets.is_empty() {
                        ui.label(
                            RichText::new("no saved snippets")
                                .size(11.0)
                                .color(tokens.text_faint),
                        );
                    }
                });
        });
    });
    ui.painter().hline(
        bottom.left()..=bottom.right(),
        bottom.top() + 0.5,
        egui::Stroke::new(1.0, tokens.stroke),
    );

    if let Some(sql) = run_sql {
        if let Some(tab) = session.tabs.get_mut(tab_index) {
            tab.sql.text = sql;
        }
    }
    if let Some((id, params)) = command {
        let mut params = params;
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
        match session.run(id, params) {
            Ok(value) => {
                if id == "sql.explain" {
                    let text = value["plan"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| {
                                    row.as_array()
                                        .map(|cells| {
                                            cells
                                                .iter()
                                                .map(|c| c.as_str().unwrap_or_default())
                                                .collect::<Vec<_>>()
                                                .join("  ")
                                        })
                                        .unwrap_or_default()
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    if let Some(tab) = session.tabs.get_mut(tab_index) {
                        tab.sql.result_label = "explain plan".into();
                        tab.sql.last_result = Some(datacraft_model::ResultSet {
                            columns: vec!["plan".into()],
                            column_types: vec!["text".into()],
                            rows: text
                                .lines()
                                .map(|line| vec![datacraft_core::Value::text(line)])
                                .collect(),
                            ..datacraft_model::ResultSet::default()
                        });
                    }
                }
                if id == "sql.format" {
                    if let Some(formatted) = value["sql"].as_str() {
                        if let Some(tab) = session.tabs.get_mut(tab_index) {
                            tab.sql.text = formatted.to_string();
                        }
                    }
                }
                if id == "report.chart" || id == "report.pivot" {
                    if let Some(tab) = session.tabs.get_mut(tab_index) {
                        tab.payload.insert("chart".into(), value.clone());
                        tab.kind = datacraft_engine::TabKind::Report;
                    }
                }
                if id == "snippet.save" {
                    session.log(Level::Success, "snippet saved");
                }
            }
            Err(e) => session.log(Level::Error, format!("{id}: {e}")),
        }
    }
}
