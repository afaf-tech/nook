//! The connection tree: connections, schemas, tables, views.

use datacraft_engine::{ConnState, Session, TabKind, TableRef};
use egui::{Color32, RichText};

use crate::app::UiState;
use crate::theme::Tokens;

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

pub fn show(
    ui: &mut egui::Ui,
    session: &Session,
    state: &mut UiState,
    tokens: &Tokens,
) -> TreeAction {
    let mut action = TreeAction::None;
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Connections").size(14.0).color(tokens.text));
        if ui
            .small_button("+")
            .on_hover_text("New connection")
            .clicked()
        {
            state.open_connection_dialog(None);
        }
    });
    ui.separator();

    let mut selected = state.connection_filter.clone();
    egui::ScrollArea::vertical()
        .id_salt("connection_tree")
        .show(ui, |ui| {
            if session.conns.is_empty() {
                ui.label(
                    RichText::new("No connections yet. Add one with +, or run the app with a URI:\n  datacraft --conn postgres://localhost/shop")
                        .color(tokens.text_dim)
                        .size(12.0),
                );
                return;
            }
            for (index, conn) in session.conns.iter().enumerate() {
                let is_active = session.active_conn == Some(index);
                let (dot, dot_color) = match conn.state {
                    ConnState::Connected => ("●", tokens.ok),
                    ConnState::Failed => ("●", tokens.error),
                    ConnState::Disconnected => ("○", tokens.text_dim),
                };
                let label = format!("{dot}  {}", conn.cfg.name);
                let response = ui.selectable_label(
                    is_active,
                    RichText::new(label).color(dot_color).size(13.0),
                );
                if response.clicked() {
                    action = TreeAction::SelectConnection(index);
                    selected = Some(conn.cfg.id.clone());
                }
                response.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        action = TreeAction::OpenConnection(index);
                        ui.close();
                    }
                    if ui.button("Close").clicked() {
                        action = TreeAction::CloseConnection(index);
                        ui.close();
                    }
                    if ui.button("Refresh catalog").clicked() {
                        action = TreeAction::Refresh(index);
                        ui.close();
                    }
                    if ui.button("New query").clicked() {
                        action = TreeAction::NewQuery(index);
                        ui.close();
                    }
                    if ui.button("ER diagram").clicked() {
                        action = TreeAction::OpenDiagram(index);
                        ui.close();
                    }
                    if ui.button("Server monitor").clicked() {
                        action = TreeAction::Monitor(index);
                        ui.close();
                    }
                    if ui.button("Users and privileges").clicked() {
                        action = TreeAction::Users(index);
                        ui.close();
                    }
                    if ui.button("Edit…").clicked() {
                        state.open_connection_dialog(Some(index));
                        ui.close();
                    }
                });
                if !is_active {
                    continue;
                }
                match (&conn.server, &conn.catalog) {
                    (Some(server), Some(catalog)) => {
                        ui.label(
                            RichText::new(format!("    {} · {} ms", server.version, conn.catalog_ms))
                                .color(tokens.text_dim)
                                .size(11.0),
                        );
                        for schema in &catalog.schemas {
                            let header = format!("{} ({})", schema.name, schema.tables.len());
                            egui::CollapsingHeader::new(RichText::new(header).size(12.0))
                                .id_salt(format!("schema-{index}-{}", schema.name))
                                .default_open(schema.name == "main" || schema.name == "public")
                                .show(ui, |ui| {
                                    for table in &schema.tables {
                                        let reference = TableRef {
                                            schema: table.schema.clone(),
                                            table: table.name.clone(),
                                        };
                                        let icon = match table.kind {
                                            datacraft_model::TableKind::View => "v",
                                            datacraft_model::TableKind::MatView => "v",
                                            _ => "t",
                                        };
                                        let text = format!("{icon} {}", table.name);
                                        let response = ui.selectable_label(
                                            false,
                                            RichText::new(text).size(12.0).color(tokens.text),
                                        );
                                        if response.clicked() {
                                            action = TreeAction::OpenData(index, reference.clone());
                                        }
                                        let hover = format!(
                                            "{} · {} columns{}",
                                            table.kind.label(),
                                            table.columns.len(),
                                            table
                                                .row_estimate
                                                .map(|r| format!(" · ~{r} rows"))
                                                .unwrap_or_default()
                                        );
                                        response.on_hover_text(hover).context_menu(|ui| {
                                            if ui.button("Open data").clicked() {
                                                action =
                                                    TreeAction::OpenData(index, reference.clone());
                                                ui.close();
                                            }
                                            if ui.button("Design").clicked() {
                                                action =
                                                    TreeAction::OpenDesign(index, reference.clone());
                                                ui.close();
                                            }
                                            if ui.button("DDL").clicked() {
                                                action =
                                                    TreeAction::OpenDdl(index, reference.clone());
                                                ui.close();
                                            }
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
                                });
                        }
                    }
                    (None, _) => {
                        ui.label(
                            RichText::new("    not connected — double-click to open")
                                .color(tokens.warn)
                                .size(11.0),
                        );
                        if ui
                            .small_button("Open")
                            .on_hover_text("Connect and read the catalog")
                            .clicked()
                        {
                            action = TreeAction::OpenConnection(index);
                        }
                    }
                    (Some(_), None) => {
                        ui.label(
                            RichText::new("    connected, catalog empty")
                                .color(tokens.warn)
                                .size(11.0),
                        );
                    }
                }
                ui.add_space(4.0);
            }
        });
    state.connection_filter = selected;
    action
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

/// Colour for a cell value type, used by the grid and the results table.
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
