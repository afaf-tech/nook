//! The connection sidebar: search, connection cards, schema and table lists.

use datacraft_engine::{ConnState, Session, TabKind, TableRef};
use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

use crate::app::UiState;
use crate::theme::{self as design, Tokens};

/// What the tree wants the app to do next, so the tree itself stays read-only.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeAction {
    None,
    OpenConnection(usize),
    CloseConnection(usize),
    SelectConnection(usize),
    Refresh(usize),
    OpenData(usize, TableRef),
    OpenDesign(usize, TableRef),
    OpenDdl(usize, TableRef),
    OpenDiagram(usize),
    NewQuery(usize),
    Monitor(usize),
    Users(usize),
}

const ROW_H: f32 = 22.0;
const INDENT: f32 = 14.0;

pub fn show(
    ui: &mut egui::Ui,
    session: &Session,
    state: &mut UiState,
    tokens: &Tokens,
) -> TreeAction {
    let mut action = TreeAction::None;

    // Header: title and the one action that starts everything.
    let header = design::band_h(ui, tokens, 34.0, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Connections").size(13.0).color(tokens.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "+", true)
                    .on_hover_text("New connection")
                    .clicked()
                {
                    state.open_connection_dialog(None);
                }
                if design::ghost_button(ui, tokens, "All", true)
                    .on_hover_text("Refresh every open connection")
                    .clicked()
                {
                    if let Some(index) = session.active_conn {
                        action = TreeAction::Refresh(index);
                    }
                }
            });
        });
    });
    ui.painter().hline(
        header.left()..=header.right(),
        header.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );

    let search = design::band_h(ui, tokens, 38.0, |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.tree_filter)
                    .desired_width(ui.available_width())
                    .hint_text("Filter connections and tables"),
            );
        });
    });
    ui.painter().hline(
        search.left()..=search.right(),
        search.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );

    let needle = state.tree_filter.trim().to_ascii_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("connection_tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if session.conns.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("No connections yet")
                            .size(12.5)
                            .color(tokens.text_dim),
                    );
                    ui.label(
                        RichText::new("Add one with +, or start the app with --conn postgres://…")
                            .size(11.0)
                            .color(tokens.text_faint),
                    );
                });
                return;
            }

            for (index, conn) in session.conns.iter().enumerate() {
                let is_active = session.active_conn == Some(index);
                let (state_color, state_label) = match conn.state {
                    ConnState::Connected => (tokens.ok, "connected"),
                    ConnState::Failed => (tokens.error, "failed"),
                    ConnState::Disconnected => (tokens.text_faint, "not connected"),
                };
                let key = format!("conn:{}", conn.cfg.id);
                let mut expanded = state.tree_expanded.contains(&key);
                let tables = conn.catalog.as_ref().map(|c| c.table_count()).unwrap_or(0);

                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
                if is_active {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                        CornerRadius::ZERO,
                        tokens.accent,
                    );
                }
                if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::ZERO, tokens.hover);
                }
                if is_active {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 3.0, rect.top()),
                            rect.max,
                        ),
                        CornerRadius::ZERO,
                        tokens.accent_dim.gamma_multiply(0.55),
                    );
                }

                let arrow = if expanded { "▾" } else { "▸" };
                let arrow_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 16.0, rect.center().y),
                    Vec2::new(18.0, 18.0),
                );
                let arrow_response =
                    ui.interact(arrow_rect, ui.id().with(("expand", index)), Sense::click());
                ui.painter().text(
                    arrow_rect.center(),
                    Align2::CENTER_CENTER,
                    arrow,
                    tokens.font(11.0),
                    tokens.text_dim,
                );
                if arrow_response.clicked() {
                    if expanded {
                        state.tree_expanded.remove(&key);
                    } else {
                        state.tree_expanded.insert(key.clone());
                    }
                    expanded = !expanded;
                }

                ui.painter().circle_filled(
                    egui::pos2(rect.left() + 34.0, rect.center().y - 7.0),
                    3.5,
                    state_color,
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 44.0, rect.center().y - 8.0),
                    Align2::LEFT_CENTER,
                    &conn.cfg.name,
                    tokens.font(12.5),
                    tokens.text,
                );
                let target = if conn.cfg.file.is_empty() {
                    let (host, port) = conn.cfg.target();
                    format!("{} · {host}:{port}", conn.cfg.kind.label())
                } else {
                    conn.cfg.file.clone()
                };
                // Second line: engine plus target, unless the target only repeats the name above.
                let file = target.rsplit('/').next().unwrap_or(&target).to_owned();
                let short = if file == conn.cfg.name {
                    conn.cfg.kind.label().to_owned()
                } else {
                    file
                };
                ui.painter().text(
                    egui::pos2(rect.left() + 44.0, rect.center().y + 10.0),
                    Align2::LEFT_CENTER,
                    crate::table::clip(&short, 36),
                    tokens.font(11.0),
                    tokens.text_faint,
                );
                if !conn.cfg.group.is_empty() {
                    ui.painter().text(
                        egui::pos2(rect.right() - 12.0, rect.center().y - 8.0),
                        Align2::RIGHT_CENTER,
                        &conn.cfg.group,
                        tokens.font(10.5),
                        tokens.text_faint,
                    );
                }
                ui.painter().text(
                    egui::pos2(rect.right() - 12.0, rect.center().y + 10.0),
                    Align2::RIGHT_CENTER,
                    format!("{tables} tables"),
                    tokens.font(10.5),
                    tokens.text_faint,
                );

                if response.clicked() {
                    action = TreeAction::SelectConnection(index);
                    if !expanded {
                        state.tree_expanded.insert(key.clone());
                        expanded = true;
                    }
                }
                if response.double_clicked() && !conn.is_open() {
                    action = TreeAction::OpenConnection(index);
                }
                conn_menu(&response, index, conn.is_open(), &mut action, state);
                response.on_hover_text(format!(
                    "{} · {state_label}\n{} · {target}",
                    conn.cfg.name,
                    conn.cfg.kind.label()
                ));

                if !expanded {
                    continue;
                }

                match (&conn.server, &conn.catalog) {
                    (Some(server), Some(catalog)) => {
                        ui.horizontal(|ui| {
                            ui.add_space(INDENT);
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} ms",
                                    server.version, conn.catalog_ms
                                ))
                                .size(10.5)
                                .color(tokens.text_faint),
                            );
                        });
                        for schema in &catalog.schemas {
                            let schema_key = format!("schema:{index}:{}", schema.name);
                            let mut schema_open = state.tree_expanded.contains(&schema_key);
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW_H),
                                Sense::click(),
                            );
                            if response.hovered() {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::ZERO, tokens.hover);
                            }
                            ui.painter().text(
                                egui::pos2(rect.left() + INDENT + 6.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                if schema_open { "▾" } else { "▸" },
                                tokens.font(10.5),
                                tokens.text_dim,
                            );
                            ui.painter().text(
                                egui::pos2(rect.left() + INDENT + 22.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                &schema.name,
                                tokens.font(12.0),
                                tokens.text,
                            );
                            ui.painter().text(
                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                format!("{}", schema.tables.len()),
                                tokens.font(10.5),
                                tokens.text_faint,
                            );
                            if response.clicked() {
                                if schema_open {
                                    state.tree_expanded.remove(&schema_key);
                                } else {
                                    state.tree_expanded.insert(schema_key.clone());
                                }
                                schema_open = !schema_open;
                            }
                            if !schema_open {
                                continue;
                            }

                            for table in &schema.tables {
                                if !needle.is_empty()
                                    && !table.name.to_ascii_lowercase().contains(&needle)
                                    && !schema.name.to_ascii_lowercase().contains(&needle)
                                {
                                    continue;
                                }
                                let reference = TableRef {
                                    schema: table.schema.clone(),
                                    table: table.name.clone(),
                                };
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), ROW_H),
                                    Sense::click(),
                                );
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        CornerRadius::ZERO,
                                        tokens.hover,
                                    );
                                }
                                let is_view = matches!(
                                    table.kind,
                                    datacraft_model::TableKind::View
                                        | datacraft_model::TableKind::MatView
                                );
                                let badge = egui::Rect::from_center_size(
                                    egui::pos2(rect.left() + INDENT + 34.0, rect.center().y),
                                    Vec2::new(16.0, 14.0),
                                );
                                ui.painter().rect_filled(
                                    badge,
                                    CornerRadius::same(3),
                                    if is_view {
                                        tokens.panel_high
                                    } else {
                                        tokens.accent_dim
                                    },
                                );
                                ui.painter().text(
                                    badge.center(),
                                    Align2::CENTER_CENTER,
                                    if is_view { "V" } else { "T" },
                                    tokens.font(9.5),
                                    if is_view {
                                        tokens.text_dim
                                    } else {
                                        tokens.accent
                                    },
                                );
                                ui.painter().text(
                                    egui::pos2(rect.left() + INDENT + 48.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    crate::table::clip(&table.name, 30),
                                    tokens.font(12.0),
                                    tokens.text,
                                );
                                if let Some(rows) = table.row_estimate {
                                    ui.painter().text(
                                        egui::pos2(rect.right() - 12.0, rect.center().y),
                                        Align2::RIGHT_CENTER,
                                        format!("~{rows}"),
                                        tokens.font(10.0),
                                        tokens.text_faint,
                                    );
                                }
                                if response.clicked() {
                                    action = TreeAction::OpenData(index, reference.clone());
                                }
                                response
                                    .on_hover_text(format!(
                                        "{} · {} columns{}",
                                        table.kind.label(),
                                        table.columns.len(),
                                        if table.comment.is_empty() {
                                            String::new()
                                        } else {
                                            format!("\n{}", table.comment)
                                        }
                                    ))
                                    .context_menu(|ui| {
                                        if ui.button("Open data").clicked() {
                                            action = TreeAction::OpenData(index, reference.clone());
                                            ui.close();
                                        }
                                        if ui.button("Design").clicked() {
                                            action =
                                                TreeAction::OpenDesign(index, reference.clone());
                                            ui.close();
                                        }
                                        if ui.button("DDL").clicked() {
                                            action = TreeAction::OpenDdl(index, reference.clone());
                                            ui.close();
                                        }
                                        ui.separator();
                                        if ui.button("Generate data…").clicked() {
                                            state.open_datagen(index, reference.clone());
                                            ui.close();
                                        }
                                        if ui.button("Export…").clicked() {
                                            state.open_export(index, reference.clone());
                                            ui.close();
                                        }
                                    });
                            }
                        }
                    }
                    (None, _) => {
                        ui.horizontal(|ui| {
                            ui.add_space(INDENT + 20.0);
                            ui.label(RichText::new("not connected").size(11.0).color(tokens.warn));
                            if design::ghost_button(ui, tokens, "Connect", true).clicked() {
                                action = TreeAction::OpenConnection(index);
                            }
                        });
                    }
                    (Some(_), None) => {
                        ui.horizontal(|ui| {
                            ui.add_space(INDENT + 20.0);
                            ui.label(
                                RichText::new("connected, catalog empty")
                                    .size(11.0)
                                    .color(tokens.warn),
                            );
                        });
                    }
                }
                ui.add_space(2.0);
            }
        });
    action
}

fn conn_menu(
    response: &egui::Response,
    index: usize,
    open: bool,
    action: &mut TreeAction,
    state: &mut UiState,
) {
    response.context_menu(|ui| {
        if ui.button(if open { "Close" } else { "Open" }).clicked() {
            *action = if open {
                TreeAction::CloseConnection(index)
            } else {
                TreeAction::OpenConnection(index)
            };
            ui.close();
        }
        if ui.button("Refresh catalog").clicked() {
            *action = TreeAction::Refresh(index);
            ui.close();
        }
        if ui.button("New query").clicked() {
            *action = TreeAction::NewQuery(index);
            ui.close();
        }
        if ui.button("ER diagram").clicked() {
            *action = TreeAction::OpenDiagram(index);
            ui.close();
        }
        if ui.button("Server monitor").clicked() {
            *action = TreeAction::Monitor(index);
            ui.close();
        }
        if ui.button("Users and privileges").clicked() {
            *action = TreeAction::Users(index);
            ui.close();
        }
        ui.separator();
        if ui.button("Edit connection…").clicked() {
            state.open_connection_dialog(Some(index));
            ui.close();
        }
    });
}

/// Open a data tab for a table and load its first page.
pub fn open_data_tab(session: &mut Session, index: usize, reference: TableRef) {
    let title = match &reference.schema {
        Some(s) => format!("{s}.{}", reference.table),
        None => reference.table.clone(),
    };
    let tab = session.add_tab(TabKind::Data, title, Some(index));
    if let Err(e) = datacraft_engine::cmd::bind_grid_table(session, tab, reference) {
        session.log(datacraft_engine::Level::Error, e.to_string());
        return;
    }
    if let Err(e) = datacraft_engine::cmd::load_grid(session, tab) {
        session.log(datacraft_engine::Level::Error, e.to_string());
    }
}

/// Colour for a cell value type, used by the results view.
pub fn value_color(value: &datacraft_core::Value, tokens: &Tokens) -> Color32 {
    match value {
        datacraft_core::Value::Null => tokens.text_dim,
        datacraft_core::Value::Int(_)
        | datacraft_core::Value::UInt(_)
        | datacraft_core::Value::Float(_) => tokens.accent,
        datacraft_core::Value::Bool(_) => tokens.warn,
        _ => tokens.text,
    }
}
