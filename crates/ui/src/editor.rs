//! The query editor: SQL text, results, history and snippets.

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
    let mut run_sql: Option<String> = None;
    let mut command: Option<(&str, serde_json::Value)> = None;

    ui.horizontal_wrapped(|ui| {
        if ui
            .button(RichText::new("▶ Run").strong())
            .on_hover_text("Run the whole script (Cmd+Enter)")
            .clicked()
        {
            command = Some(("sql.run", serde_json::json!({})));
        }
        if ui.button("▶ Statement at cursor").clicked() {
            command = Some(("sql.runStatement", serde_json::json!({"index": 0})));
        }
        if ui.button("Format").clicked() {
            command = Some(("sql.format", serde_json::json!({})));
        }
        if ui.button("Explain").clicked() {
            command = Some(("sql.explain", serde_json::json!({})));
        }
        if ui.button("Σ Aggregate report").clicked() {
            command = Some(("report.chart", serde_json::json!({"chart": "column"})));
        }
        if ui.button("Save snippet").clicked() {
            let name = format!("Query {}", session.snippets.len() + 1);
            command = Some(("snippet.save", serde_json::json!({"name": name})));
        }
        if ui.button("Clear").clicked() {
            if let Some(tab) = session.tabs.get_mut(tab_index) {
                tab.sql.text.clear();
            }
        }
        ui.separator();
        let conn_name = session.tabs[tab_index]
            .conn
            .and_then(|i| session.conns.get(i))
            .map(|c| format!("{} · {}", c.cfg.name, c.cfg.display_uri()))
            .unwrap_or_else(|| "no connection on this tab".into());
        ui.label(RichText::new(conn_name).size(11.0).color(tokens.text_dim));
    });

    let shortcuts = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
    if shortcuts {
        command = Some(("sql.run", serde_json::json!({})));
    }

    let available = ui.available_height();
    let editor_height = (available * 0.42).clamp(120.0, 420.0);

    egui::ScrollArea::vertical()
        .id_salt(format!("editor-scroll-{tab_index}"))
        .max_height(editor_height)
        .show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::multiline(&mut session.tabs[tab_index].sql.text)
                    .code_editor()
                    .desired_rows(12)
                    .desired_width(f32::INFINITY)
                    .hint_text("SELECT * FROM customers WHERE city = 'London';\n\nCmd+Enter runs it, Cmd+Shift+F formats it."),
            );
            if response.changed() {
                state.editor_dirty = true;
            }
        });

    ui.separator();

    // Statement list, so a script is never a mystery.
    let statements = datacraft_engine::cmd::sql::split(&session.tabs[tab_index].sql.text);
    if statements.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{} statements:", statements.len()))
                    .size(11.0)
                    .color(tokens.text_dim),
            );
            for (index, statement) in statements.iter().take(12).enumerate() {
                let label = statement.chars().take(24).collect::<String>();
                if ui
                    .small_button(format!("▶ {index}: {label}…"))
                    .on_hover_text(statement)
                    .clicked()
                {
                    command = Some(("sql.runStatement", serde_json::json!({"index": index})));
                }
            }
        });
    }

    if let Some(error) = session.tabs[tab_index].sql.error.clone() {
        ui.colored_label(tokens.error, error);
    }

    let result = session.tabs[tab_index].sql.last_result.clone();
    let label = session.tabs[tab_index].sql.result_label.clone();
    if let Some(result) = result {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).size(11.0).color(tokens.text_dim));
            if result.is_tabular() {
                if ui.small_button("Export CSV…").clicked() {
                    state.open_export_grid("csv");
                }
                if ui.small_button("Chart").clicked() {
                    command = Some(("report.chart", serde_json::json!({"chart": "column"})));
                }
                if ui.small_button("Pivot").clicked() {
                    command = Some(("report.pivot", serde_json::json!({})));
                }
            }
        });
        egui::ScrollArea::both()
            .id_salt(format!("result-scroll-{tab_index}"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(format!("result-{tab_index}"))
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        for (index, name) in result.columns.iter().enumerate() {
                            let kind = result
                                .column_types
                                .get(index)
                                .map(String::as_str)
                                .unwrap_or("");
                            ui.label(
                                RichText::new(name.to_string())
                                    .strong()
                                    .size(12.0)
                                    .color(tokens.text),
                            )
                            .on_hover_text(kind);
                        }
                        ui.end_row();
                        for row in result.rows.iter().take(500) {
                            for value in row {
                                let text = value.display().replace('\n', "\\n");
                                let text = if text.chars().count() > 40 {
                                    format!("{}…", text.chars().take(39).collect::<String>())
                                } else {
                                    text
                                };
                                ui.label(
                                    RichText::new(text)
                                        .size(12.0)
                                        .monospace()
                                        .color(value_color(value, tokens)),
                                );
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    // History and snippets, side by side.
    ui.separator();
    ui.columns(2, |columns| {
        columns[0].label(RichText::new("History").strong().size(12.0));
        egui::ScrollArea::vertical()
            .id_salt(format!("history-{tab_index}"))
            .max_height(160.0)
            .show(&mut columns[0], |ui| {
                for entry in session.history.iter().rev().take(40) {
                    let label = entry.sql.replace('\n', " ");
                    let label = label.chars().take(70).collect::<String>();
                    if ui
                        .small_button(format!("{} · {} ms", label, entry.elapsed_ms.round()))
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
                            .color(tokens.text_dim),
                    );
                }
            });
        columns[1].label(RichText::new("Snippets").strong().size(12.0));
        egui::ScrollArea::vertical()
            .id_salt(format!("snippets-{tab_index}"))
            .max_height(160.0)
            .show(&mut columns[1], |ui| {
                for snippet in &session.snippets {
                    if ui
                        .small_button(snippet.name.clone())
                        .on_hover_text(&snippet.sql)
                        .clicked()
                    {
                        run_sql = Some(snippet.sql.clone());
                    }
                }
            });
    });

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
