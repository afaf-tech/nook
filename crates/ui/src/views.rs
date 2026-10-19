//! The non-grid tabs: object design, ER diagram, reports, server monitor, jobs, transfer and users.
//! Every one of them is built from the same pieces: a context toolbar, chips for the facts, cards
//! for the details and the shared table widget wherever rows are involved.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, Vec2};
use nook_core::Value as Cell;
use nook_engine::{Level, Session, TabKind};

use crate::app::UiState;
use crate::table::{self, TableColumn, TableView};
use crate::theme::{self as design, Tokens};

/// serde_json in, the one cell type out.
fn cell(value: &serde_json::Value) -> Cell {
    match value {
        serde_json::Value::Null => Cell::Null,
        serde_json::Value::Bool(b) => Cell::Bool(*b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => Cell::Int(i),
            (None, Some(u), _) => Cell::UInt(u),
            (None, None, Some(f)) => Cell::Float(f),
            _ => Cell::Null,
        },
        serde_json::Value::String(s) => Cell::Text(s.clone()),
        other => Cell::Json(other.to_string()),
    }
}

fn rows_from(items: &[serde_json::Value], keys: &[&str]) -> Vec<Vec<Cell>> {
    items
        .iter()
        .map(|item| keys.iter().map(|key| cell(&item[*key])).collect())
        .collect()
}

fn specs(names: &[&str], types: &[&str]) -> Vec<TableColumn> {
    names
        .iter()
        .zip(types.iter())
        .map(|(name, type_name)| TableColumn::new(*name, *type_name))
        .collect()
}

/// What a read-only list needs: the data, the room it has, and its own frame id. The object
/// inspector's preview is one of these too, which is why it is `pub(crate)`.
pub(crate) struct List<'a> {
    pub columns: &'a [TableColumn],
    pub rows: &'a [Vec<Cell>],
    pub height: f32,
    pub id: egui::Id,
    pub empty: &'a str,
    /// The row-number column: the design gives it to the data lists and not to the inspector.
    pub(crate) gutter: bool,
    /// Right-click menu for a row, forwarded to the widget: the design tab's column list uses it to
    /// offer the per-column editors. `None` keeps the list plain read-only, as other callers want.
    pub(crate) row_menu: Option<crate::table::RowMenu<'a>>,
}

/// Read-only table: the shape every list in this file uses.
pub(crate) fn list(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    spec: List<'_>,
    state: &mut table::TableState,
) -> table::TableEvent {
    let mut view = TableView {
        columns: spec.columns,
        hidden: &[],
        foreign_keys: &[],
        rows: spec.rows,
        pending: Vec::new(),
        inserted: Vec::new(),
        selected: state.selected,
        anchor: None,
        sort: None,
        editable: false,
        sortable: false,
        gutter: spec.gutter,
        height: spec.height,
        // ponytail: the catalog lists stay compact; density reaches the data grid and the results,
        // where the design demonstrates it (`.reg-grid .gr.cmft`). Thread a row height through
        // `list` when an explore register needs comfortable rows.
        row_h: design::ROW_H,
        empty_text: spec.empty,
        row_menu: spec.row_menu,
    };
    table::show(ui, tokens, &mut view, state, spec.id)
}

/// The table the design tab draws: the catalog's copy, not the tab's. The tab's copy is taken when
/// the tab opens, so after an edit — add column, rename column — it would keep drawing the table the
/// user just changed, and the next edit would run against what they no longer see on screen.
fn design_table(session: &Session, tab_index: usize) -> Option<nook_model::Table> {
    let tab = session.tabs.get(tab_index)?;
    let reference = tab.grid.table.as_ref()?;
    let catalog = session
        .conns
        .get(tab.conn?)
        .and_then(|c| c.catalog.as_ref());
    match catalog {
        // The catalog is the truth when the connection has one: a table it does not hold has been
        // renamed or dropped away from this tab, and drawing the tab's open-time copy would hand the
        // designer buttons a name that no longer exists, to fail with `no such table`.
        Some(catalog) => catalog
            .find_table(reference.schema.as_deref(), &reference.table)
            .cloned(),
        // No catalog read yet — the connection is not open — so the snapshot is all there is.
        None => tab.table.clone(),
    }
}

/// Object design: columns, indexes, foreign keys and the DDL, with the designer buttons.
pub fn design(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let Some(table_def) = design_table(session, tab_index) else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("this tab has no table")
                    .size(12.5)
                    .color(tokens.text_faint),
            );
        });
        return;
    };
    let mut show_ddl = false;
    let mut er_diagram = false;
    // What the designer commands act on, handed over as data: the string form `open_command` takes is
    // `&`-delimited, and both a table named `R&D` and SQLite's file path as the connection id would
    // be cut in half by it. The buttons used to send no table at all, so every one of them opened a
    // dialog whose only possible Run failed with "no such table: ".
    let mut prefill: Vec<(&str, &str)> = Vec::new();
    // Owned: the column menu below is a closure holding `prefill` for the whole body, so a borrowed
    // `&session.conns[...]` here would freeze `session` for every later `session.run` call.
    let conn_id = session.tabs[tab_index]
        .conn
        .and_then(|index| session.conns.get(index))
        .map(|c| c.cfg.id.clone());
    if let Some(conn) = &conn_id {
        prefill.push(("conn", conn.as_str()));
    }
    let table_key = table_def.key();
    prefill.push(("table", &table_key));

    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(table_def.key()).size(13.5).color(tokens.text));
            design::chip(ui, tokens, table_def.kind.label(), tokens.info);
            design::chip(
                ui,
                tokens,
                &format!("{} columns", table_def.columns.len()),
                tokens.text_dim,
            );
            design::chip(
                ui,
                tokens,
                &format!("{} indexes", table_def.indexes.len()),
                tokens.text_dim,
            );
            if let Some(rows) = table_def.row_estimate {
                design::chip(ui, tokens, &format!("~{rows} rows"), tokens.text_faint);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "DDL", true).clicked() {
                    show_ddl = true;
                }
                if design::ghost_button(ui, tokens, "ER diagram", true).clicked() {
                    er_diagram = true;
                }
                design::divider(ui, tokens);
                let mut trigger_prefill = prefill.clone();
                trigger_prefill.push(("action", "create"));
                let mut function_prefill = prefill.clone();
                function_prefill.push(("action", "create"));
                let mut sequence_prefill = prefill.clone();
                sequence_prefill.push(("action", "create"));
                if design::ghost_button(ui, tokens, "+ Trigger", true).clicked() {
                    state.open_command_with("schema.triggerEditor", &trigger_prefill);
                }
                if design::ghost_button(ui, tokens, "+ Function", true).clicked() {
                    state.open_command_with("schema.functionEditor", &function_prefill);
                }
                if design::ghost_button(ui, tokens, "+ Sequence", true).clicked() {
                    state.open_command_with("schema.sequenceEditor", &sequence_prefill);
                }
                if design::ghost_button(ui, tokens, "+ Foreign key", true).clicked() {
                    state.open_command_with("schema.addForeignKey", &prefill);
                }
                if design::ghost_button(ui, tokens, "+ Index", true).clicked() {
                    state.open_command_with("schema.createIndex", &prefill);
                }
                // A table without a key cannot be edited in the grid; this is the way out.
                if table_def.primary_key().is_empty()
                    && design::primary_button(ui, tokens, "Set primary key", true).clicked()
                {
                    state.open_command_with("schema.setPrimaryKey", &prefill);
                }
                if design::primary_button(ui, tokens, "+ Column", true).clicked() {
                    state.open_command_with("schema.addColumn", &prefill);
                }
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );

    if er_diagram {
        if let Some(conn) = session.tabs[tab_index].conn {
            state.request_diagram(session, conn);
        }
    }
    if show_ddl {
        if let Some(conn) = session.tabs[tab_index].conn {
            let params = serde_json::json!({
                "conn": session.conns[conn].cfg.id,
                "table": table_def.key(),
            });
            match session.run("schema.ddl", params) {
                Ok(value) => {
                    if let Some(sql) = value["ddl"].as_str() {
                        session.tabs[tab_index]
                            .payload
                            .insert("ddl".into(), serde_json::json!(sql));
                    }
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
    }

    egui::ScrollArea::vertical()
        .id_salt(format!("design-{tab_index}"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            design::section(
                ui,
                tokens,
                "Columns",
                Some("right-click a column to rename it, change its type, nullability, default — or drop it"),
            );
            ui.add_space(4.0);
            let column_specs = specs(
                &["name", "type", "nullable", "default", "key", "comment"],
                &["text", "text", "text", "text", "text", "text"],
            );
            let column_rows: Vec<Vec<Cell>> = table_def
                .columns
                .iter()
                .map(|column| {
                    vec![
                        Cell::Text(column.name.clone()),
                        Cell::Text(column.type_name.clone()),
                        Cell::Text(if column.nullable { "yes" } else { "no" }.into()),
                        Cell::Text(column.default.clone().unwrap_or_default()),
                        Cell::Text(
                            if column.primary_key {
                                "primary key"
                            } else if column.auto_increment {
                                "generated"
                            } else if column.enum_values.is_empty() {
                                ""
                            } else {
                                "enum"
                            }
                            .into(),
                        ),
                        Cell::Text(column.comment.clone()),
                    ]
                })
                .collect();
            // Right-click a column to edit it. The menu only records the pick — the callback
            // cannot also hold `&mut state`, which the list call borrows for its editor state —
            // and the dialog opens after `list` returns.
            let mut picked: Option<(&'static str, String)> = None;
            let mut column_menu = |ui: &mut egui::Ui, row: usize| {
                let Some(name) = table_def.columns.get(row).map(|c| c.name.clone()) else {
                    return;
                };
                if ui.button("Rename column").clicked() {
                    picked = Some(("schema.renameColumn", name.clone()));
                }
                if ui.button("Change type").clicked() {
                    picked = Some(("schema.changeColumnType", name.clone()));
                }
                if ui.button("Toggle nullability").clicked() {
                    picked = Some(("schema.setNullable", name.clone()));
                }
                if ui.button("Change default").clicked() {
                    picked = Some(("schema.setDefault", name.clone()));
                }
                ui.separator();
                if ui.button("Drop column").clicked() {
                    picked = Some(("schema.dropColumn", name.clone()));
                }
            };
            let height = (column_rows.len() as f32 * design::ROW_H + 34.0).min(360.0);
            let columns_list = List {
                gutter: true,
                columns: &column_specs,
                rows: &column_rows,
                height,
                id: egui::Id::new(("design-columns", tab_index)),
                empty: "no columns",
                row_menu: Some(&mut column_menu),
            };
            list(
                ui,
                tokens,
                columns_list,
                state.table(tab_index),
            );
            if let Some((id, column)) = picked {
                let mut pre = prefill.clone();
                pre.push(("column", &column));
                state.open_command_with(id, &pre);
            }

            if !table_def.indexes.is_empty() {
                ui.add_space(12.0);
                design::section(ui, tokens, "Indexes", None);
                ui.add_space(4.0);
                for index in &table_def.indexes {
                    let card = design::card_frame(tokens);
                    card.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&index.name).size(12.5).color(tokens.text));
                            if index.primary {
                                design::chip(ui, tokens, "primary", tokens.accent);
                            }
                            if index.unique {
                                design::chip(ui, tokens, "unique", tokens.ok);
                            }
                            if !index.kind.is_empty() {
                                design::chip(ui, tokens, &index.kind, tokens.text_faint);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(index.columns.join(", "))
                                            .size(11.5)
                                            .monospace()
                                            .color(tokens.text_dim),
                                    );
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }
            }

            if !table_def.foreign_keys.is_empty() {
                ui.add_space(12.0);
                design::section(ui, tokens, "Foreign keys", None);
                ui.add_space(4.0);
                for fk in &table_def.foreign_keys {
                    let card = design::card_frame(tokens);
                    card.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&fk.name).size(12.5).color(tokens.text));
                            if !fk.on_delete.is_empty()
                                && !fk.on_delete.eq_ignore_ascii_case("no action")
                            {
                                design::chip(
                                    ui,
                                    tokens,
                                    &format!("on delete {}", fk.on_delete.to_uppercase()),
                                    tokens.warn,
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}  →  {}({})",
                                            fk.columns.join(", "),
                                            fk.ref_table,
                                            fk.ref_columns.join(", ")
                                        ))
                                        .size(11.5)
                                        .monospace()
                                        .color(tokens.text_dim),
                                    );
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }
            }

            ui.add_space(12.0);
            design::section(
                ui,
                tokens,
                "DDL",
                Some("what the server would run to rebuild this table"),
            );
            ui.add_space(4.0);
            if let Some(serde_json::Value::String(ddl)) =
                session.tabs[tab_index].payload.get_mut("ddl")
            {
                let frame = egui::Frame::new()
                    .fill(tokens.background)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(10));
                frame.show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(ddl)
                            .code_editor()
                            .desired_rows(10)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            } else {
                ui.label(
                    RichText::new("press DDL to load the statement")
                        .size(11.5)
                        .color(tokens.text_faint),
                );
            }
        });
}

/// ER diagram: deterministic boxes and foreign-key arrows, pan and zoom.
pub fn diagram(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let payload = session.tabs[tab_index].payload.get("diagram").cloned();
    let mut reload = false;
    let mut export = false;
    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("ER diagram").size(13.5).color(tokens.text));
            if let Some(payload) = &payload {
                design::chip(
                    ui,
                    tokens,
                    &format!(
                        "{} tables",
                        payload["nodes"].as_array().map(Vec::len).unwrap_or(0)
                    ),
                    tokens.text_dim,
                );
                design::chip(
                    ui,
                    tokens,
                    &format!(
                        "{} relations",
                        payload["edges"].as_array().map(Vec::len).unwrap_or(0)
                    ),
                    tokens.text_dim,
                );
            }
            design::chip(
                ui,
                tokens,
                &format!("{:.0}% zoom", state.diagram_zoom * 100.0),
                tokens.text_faint,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "Export SVG", true).clicked() {
                    export = true;
                }
                if design::ghost_button(ui, tokens, "Reload", true).clicked() {
                    reload = true;
                }
                if design::ghost_button(ui, tokens, "Reset view", true).clicked() {
                    state.diagram_offset = egui::vec2(0.0, 0.0);
                    state.diagram_zoom = 1.0;
                }
                design::keycap(ui, tokens, "drag to pan");
                design::keycap(ui, tokens, "scroll to zoom");
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    if reload {
        if let Some(conn) = session.tabs[tab_index].conn {
            state.request_diagram(session, conn);
        }
    }
    if export {
        if let Some(conn) = session.tabs[tab_index].conn {
            if let Some(path) = state.pick_save_path("diagram.svg") {
                let params = serde_json::json!({"conn": session.conns[conn].cfg.id, "path": path});
                match session.run("diagram.exportSvg", params) {
                    Ok(value) => session.log(
                        Level::Success,
                        format!("wrote {}", value["path"].as_str().unwrap_or("")),
                    ),
                    Err(e) => session.log(Level::Error, e.to_string()),
                }
            }
        }
    }

    let Some(payload) = payload else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("reading the catalog…")
                    .size(12.5)
                    .color(tokens.text_faint),
            );
        });
        return;
    };
    let nodes = payload["nodes"].as_array().cloned().unwrap_or_default();
    let edges = payload["edges"].as_array().cloned().unwrap_or_default();

    let (response, painter) = ui.allocate_painter(
        Vec2::new(ui.available_width(), ui.available_height().max(200.0)),
        Sense::click_and_drag(),
    );
    painter.rect_filled(response.rect, 0.0, tokens.background);

    // Canvas dots, so panning has a reference.
    let step = 26.0;
    let mut x = response.rect.left() - response.rect.left() % step;
    while x < response.rect.right() {
        let mut y = response.rect.top() - response.rect.top() % step;
        while y < response.rect.bottom() {
            painter.circle_filled(egui::pos2(x, y), 0.8, tokens.stroke);
            y += step;
        }
        x += step;
    }

    let origin = response.rect.min + state.diagram_offset;
    let zoom = state.diagram_zoom;

    for edge in &edges {
        let (Some(from), Some(to)) = (
            nodes.get(edge["from"].as_u64().unwrap_or(0) as usize),
            nodes.get(edge["to"].as_u64().unwrap_or(0) as usize),
        ) else {
            continue;
        };
        let from_pos = egui::pos2(
            from["x"].as_f64().unwrap_or(0.0) as f32 + from["w"].as_f64().unwrap_or(140.0) as f32,
            from["y"].as_f64().unwrap_or(0.0) as f32 + 26.0,
        );
        let to_pos = egui::pos2(
            to["x"].as_f64().unwrap_or(0.0) as f32,
            to["y"].as_f64().unwrap_or(0.0) as f32 + 26.0,
        );
        let a = origin + from_pos.to_vec2() * zoom;
        let b = origin + to_pos.to_vec2() * zoom;
        let mid_x = (a.x + b.x) / 2.0;
        let stroke = Stroke::new(1.4, tokens.accent.gamma_multiply(0.75));
        painter.line_segment([a, egui::pos2(mid_x, a.y)], stroke);
        painter.line_segment([egui::pos2(mid_x, a.y), egui::pos2(mid_x, b.y)], stroke);
        painter.line_segment([egui::pos2(mid_x, b.y), b], stroke);
        // Arrow head at the referenced table.
        painter.add(egui::Shape::convex_polygon(
            vec![
                b,
                egui::pos2(b.x - 7.0, b.y - 4.0),
                egui::pos2(b.x - 7.0, b.y + 4.0),
            ],
            tokens.accent,
            Stroke::NONE,
        ));
        painter.circle_filled(a, 3.0, tokens.accent.gamma_multiply(0.6));
    }

    for node in &nodes {
        let x = node["x"].as_f64().unwrap_or(0.0) as f32;
        let y = node["y"].as_f64().unwrap_or(0.0) as f32;
        let w = node["w"].as_f64().unwrap_or(140.0) as f32;
        let h = node["h"].as_f64().unwrap_or(60.0) as f32;
        let rect =
            egui::Rect::from_min_size(origin + egui::vec2(x, y) * zoom, egui::vec2(w, h) * zoom);
        if !response.rect.intersects(rect) {
            continue;
        }
        let radius = CornerRadius::same((6.0 * zoom) as u8);
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, 2.0)),
            radius,
            Color32::from_black_alpha(if tokens.dark { 90 } else { 24 }),
        );
        painter.rect_filled(rect, radius, tokens.panel);
        let head = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 24.0 * zoom));
        painter.rect_filled(
            head,
            CornerRadius {
                nw: radius.nw,
                ne: radius.ne,
                sw: 0,
                se: 0,
            },
            tokens.grid_header,
        );
        painter.text(
            rect.min + Vec2::new(9.0, 12.0 * zoom),
            Align2::LEFT_CENTER,
            node["table"].as_str().unwrap_or(""),
            egui::FontId::proportional(12.0 * zoom),
            tokens.text,
        );
        let keys: Vec<String> = node["primary_key"]
            .as_array()
            .map(|keys| {
                keys.iter()
                    .filter_map(|k| k.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if !keys.is_empty() {
            let badge = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 24.0, rect.top() + 6.0),
                Vec2::new(17.0, 13.0),
            );
            painter.rect_filled(badge, CornerRadius::same(3), tokens.accent_dim);
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                "PK",
                egui::FontId::proportional(9.0),
                tokens.accent,
            );
        }
        for (index, column) in node["columns"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if index > 24 {
                break;
            }
            let name = column.as_str().unwrap_or("");
            let is_key = keys.iter().any(|k| k == name);
            let row = egui::Rect::from_min_size(
                egui::pos2(
                    rect.left(),
                    rect.top() + 24.0 * zoom + index as f32 * 18.0 * zoom,
                ),
                Vec2::new(rect.width(), 18.0 * zoom),
            );
            if index % 2 == 1 {
                painter.rect_filled(row, CornerRadius::ZERO, tokens.grid_row_alt);
            }
            if is_key {
                painter.rect_filled(
                    egui::Rect::from_min_size(row.min, Vec2::new(2.0, row.height())),
                    CornerRadius::ZERO,
                    tokens.warn,
                );
            }
            painter.text(
                row.left_center() + Vec2::new(10.0, 0.0),
                Align2::LEFT_CENTER,
                name,
                egui::FontId::monospace(10.5 * zoom),
                if is_key {
                    tokens.dirty_text
                } else {
                    tokens.text_dim
                },
            );
        }
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, tokens.stroke_strong),
            egui::StrokeKind::Inside,
        );
    }

    if response.dragged() {
        state.diagram_offset += response.drag_delta();
    }
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if response.hovered() && scroll.abs() > 0.0 {
        state.diagram_zoom = (state.diagram_zoom * (1.0 + scroll * 0.001)).clamp(0.3, 3.0);
    }
}

/// Report tab: a chart, a pivot or a plain table, from the payload the command produced.
pub fn report(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let payload = session.tabs[tab_index]
        .payload
        .get("chart")
        .cloned()
        .unwrap_or_default();
    let kind = payload["chart"].as_str().unwrap_or("table").to_string();
    let sql = payload["sql"].as_str().unwrap_or("").to_string();
    let mut rerun: Option<&str> = None;

    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("Report").size(13.5).color(tokens.text));
            if !sql.is_empty() {
                ui.label(
                    RichText::new(table::clip(&sql.replace('\n', " "), 70))
                        .size(11.0)
                        .monospace()
                        .color(tokens.text_faint),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let options = [
                    ("Bar", kind == "bar"),
                    ("Column", kind == "column"),
                    ("Line", kind == "line"),
                    ("Area", kind == "area"),
                    ("Pie", kind == "pie"),
                    ("Table", kind == "table"),
                ];
                if let Some(index) = design::segmented(
                    ui,
                    tokens,
                    &options.iter().map(|(l, s)| (*l, *s)).collect::<Vec<_>>(),
                ) {
                    rerun = Some(match index {
                        0 => "bar",
                        1 => "column",
                        2 => "line",
                        3 => "area",
                        4 => "pie",
                        _ => "table",
                    });
                }
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    if let (Some(chart), Some(conn)) = (rerun, session.tabs[tab_index].conn) {
        let params = serde_json::json!({
            "conn": session.conns[conn].cfg.id,
            "sql": sql,
            "chart": chart,
        });
        match session.run("report.chart", params) {
            Ok(value) => {
                session.tabs[tab_index]
                    .payload
                    .insert("chart".into(), value);
            }
            Err(e) => session.log(Level::Error, e.to_string()),
        }
    }

    // Pivot shape.
    if payload.get("rows").is_some() && payload.get("cells").is_some() {
        let row_keys = payload["rows"].as_array().cloned().unwrap_or_default();
        let column_keys = payload["columns"].as_array().cloned().unwrap_or_default();
        let mut names: Vec<&str> = vec!["row"];
        let column_labels: Vec<String> = column_keys
            .iter()
            .map(|c| c.as_str().unwrap_or("").to_string())
            .collect();
        for label in &column_labels {
            names.push(label);
        }
        let column_specs: Vec<TableColumn> =
            names.iter().map(|n| TableColumn::new(*n, "text")).collect();
        let rows: Vec<Vec<Cell>> = row_keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let mut row = vec![Cell::Text(key.as_str().unwrap_or("").to_string())];
                for cell_value in payload["cells"][index]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    row.push(match cell_value.as_f64() {
                        Some(v) => Cell::Float(v),
                        None => Cell::Null,
                    });
                }
                row
            })
            .collect();
        let height = (rows.len() as f32 * design::ROW_H + 34.0).min(ui.available_height());
        list(
            ui,
            tokens,
            List {
                gutter: true,
                columns: &column_specs,
                rows: &rows,
                height,
                id: egui::Id::new(("pivot", tab_index)),
                empty: "nothing to pivot",
                row_menu: None,
            },
            state.table(tab_index),
        );
        ui.label(
            RichText::new(payload["summary"].as_str().unwrap_or(""))
                .size(11.5)
                .color(tokens.text_dim),
        );
        return;
    }

    if kind == "table" {
        let result = payload.get("result").or_else(|| payload.get("data"));
        let Some(result) = result else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("no rows in this report")
                        .size(12.5)
                        .color(tokens.text_faint),
                );
            });
            return;
        };
        let names: Vec<String> = result["columns"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|c| c.as_str().unwrap_or("").to_string())
            .collect();
        let column_specs: Vec<TableColumn> = names
            .iter()
            .map(|n| TableColumn::new(n.clone(), ""))
            .collect();
        let items = result["rows"].as_array().cloned().unwrap_or_default();
        let rows: Vec<Vec<Cell>> = items
            .iter()
            .map(|item| {
                names
                    .iter()
                    .map(|name| item.get(name).map(cell).unwrap_or(Cell::Null))
                    .collect()
            })
            .collect();
        let height = (rows.len() as f32 * design::ROW_H + 34.0).min(ui.available_height());
        list(
            ui,
            tokens,
            List {
                gutter: true,
                columns: &column_specs,
                rows: &rows,
                height,
                id: egui::Id::new(("report-table", tab_index)),
                empty: "no rows",
                row_menu: None,
            },
            state.table(tab_index),
        );
        ui.label(
            RichText::new(payload["summary"].as_str().unwrap_or(""))
                .size(11.5)
                .color(tokens.text_dim),
        );
        return;
    }

    let labels: Vec<String> = payload["labels"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    let series: Vec<(String, Vec<f64>)> = payload["series"]
        .as_array()
        .map(|s| {
            s.iter()
                .map(|entry| {
                    (
                        entry["name"].as_str().unwrap_or("value").to_string(),
                        entry["values"]
                            .as_array()
                            .map(|v| v.iter().filter_map(|x| x.as_f64()).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if labels.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(
                    "nothing to chart yet — run a query that returns a label column and a number",
                )
                .size(12.5)
                .color(tokens.text_faint),
            );
        });
        return;
    }

    let (response, painter) = ui.allocate_painter(
        Vec2::new(
            ui.available_width(),
            (ui.available_height() - 30.0).max(220.0),
        ),
        Sense::hover(),
    );
    let outer = response.rect.shrink2(Vec2::new(28.0, 22.0));
    let plot = egui::Rect::from_min_max(
        egui::pos2(outer.left() + 54.0, outer.top()),
        egui::pos2(outer.right(), outer.bottom() - 22.0),
    );
    let max = series
        .iter()
        .flat_map(|(_, values)| values.iter().cloned())
        .fold(0.0f64, f64::max)
        .max(1.0);
    let palette = tokens.chart;

    if kind != "pie" {
        // Y gridlines and their labels.
        for step in 0..=4 {
            let y = plot.bottom() - plot.height() * step as f32 / 4.0;
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(0.75, tokens.grid_line),
            );
            painter.text(
                egui::pos2(plot.left() - 8.0, y),
                Align2::RIGHT_CENTER,
                format_axis(max * step as f64 / 4.0),
                egui::FontId::proportional(10.0),
                tokens.text_faint,
            );
        }
    }

    match kind.as_str() {
        "pie" => {
            let values = series.first().map(|(_, v)| v.clone()).unwrap_or_default();
            let total: f64 = values.iter().sum::<f64>().max(0.001);
            let center = outer.center();
            let radius = outer.height().min(outer.width()) * 0.34;
            let mut start = 0.0f32;
            for (index, value) in values.iter().enumerate() {
                let sweep = (value / total) as f32 * std::f32::consts::TAU;
                let steps = ((sweep / 0.1).ceil() as usize).max(3);
                let points: Vec<egui::Pos2> = (0..=steps)
                    .map(|step| {
                        let angle = start + sweep * step as f32 / steps as f32;
                        center + egui::vec2(angle.cos(), angle.sin()) * radius
                    })
                    .collect();
                for pair in points.windows(2) {
                    painter.add(egui::Shape::convex_polygon(
                        vec![center, pair[0], pair[1]],
                        palette[index % palette.len()],
                        Stroke::NONE,
                    ));
                }
                if sweep > 0.25 {
                    let angle = start + sweep / 2.0;
                    painter.text(
                        center + egui::vec2(angle.cos(), angle.sin()) * (radius * 0.68),
                        Align2::CENTER_CENTER,
                        format!("{:.0}%", value / total * 100.0),
                        egui::FontId::proportional(10.5),
                        tokens.accent_text,
                    );
                }
                start += sweep;
                if start >= std::f32::consts::TAU {
                    break;
                }
            }
            // Legend.
            let mut y = outer.top();
            for (index, label) in labels.iter().enumerate() {
                let x = outer.right() - 130.0;
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(x, y + 2.0), Vec2::new(9.0, 9.0)),
                    CornerRadius::same(2),
                    palette[index % palette.len()],
                );
                painter.text(
                    egui::pos2(x + 15.0, y + 7.0),
                    Align2::LEFT_CENTER,
                    table::clip(label, 18),
                    egui::FontId::proportional(11.0),
                    tokens.text_dim,
                );
                y += 16.0;
            }
        }
        "line" | "area" => {
            for (series_index, (name, values)) in series.iter().enumerate() {
                let colour = palette[series_index % palette.len()];
                let points: Vec<egui::Pos2> = values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let x = if values.len() <= 1 {
                            plot.left()
                        } else {
                            plot.left() + plot.width() * index as f32 / (values.len() - 1) as f32
                        };
                        let y = plot.bottom() - (value / max) as f32 * plot.height();
                        egui::pos2(x, y)
                    })
                    .collect();
                if kind == "area" && points.len() > 1 {
                    let mut shape = points.clone();
                    shape.push(egui::pos2(plot.right(), plot.bottom()));
                    shape.push(egui::pos2(plot.left(), plot.bottom()));
                    painter.add(egui::Shape::convex_polygon(
                        shape,
                        colour.gamma_multiply(0.25),
                        Stroke::NONE,
                    ));
                }
                painter.add(egui::Shape::line(points.clone(), Stroke::new(2.0, colour)));
                for point in &points {
                    painter.circle_filled(*point, 2.5, colour);
                }
                // Legend entry.
                let legend = egui::pos2(
                    plot.left() + 8.0 + series_index as f32 * 110.0,
                    plot.top() + 6.0,
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(legend, Vec2::new(9.0, 9.0)),
                    CornerRadius::same(2),
                    colour,
                );
                painter.text(
                    legend + Vec2::new(15.0, 5.0),
                    Align2::LEFT_CENTER,
                    table::clip(name, 14),
                    egui::FontId::proportional(11.0),
                    tokens.text_dim,
                );
            }
        }
        _ => {
            let group_width = plot.width() / labels.len().max(1) as f32;
            let bar_width = (group_width * 0.7 / series.len().max(1) as f32).max(3.0);
            for (label_index, _) in labels.iter().enumerate() {
                for (series_index, (_, values)) in series.iter().enumerate() {
                    let value = values.get(label_index).cloned().unwrap_or(0.0);
                    let height = (value / max) as f32 * plot.height();
                    let x = plot.left()
                        + group_width * label_index as f32
                        + group_width * 0.15
                        + bar_width * series_index as f32;
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(x, plot.bottom() - height),
                        Vec2::new(bar_width * 0.9, height.max(1.0)),
                    );
                    painter.rect_filled(
                        bar,
                        CornerRadius {
                            nw: 2,
                            ne: 2,
                            sw: 0,
                            se: 0,
                        },
                        palette[series_index % palette.len()],
                    );
                }
            }
            for (index, label) in labels.iter().enumerate() {
                if labels.len() > 24 && index % 2 == 1 {
                    continue;
                }
                painter.text(
                    egui::pos2(
                        plot.left() + group_width * (index as f32 + 0.5),
                        plot.bottom() + 6.0,
                    ),
                    Align2::CENTER_TOP,
                    table::clip(label, 12),
                    egui::FontId::proportional(10.0),
                    tokens.text_dim,
                );
            }
        }
    }

    if kind != "pie" {
        painter.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            Stroke::new(1.0, tokens.stroke_strong),
        );
    }
    design::band(ui, tokens, |ui| {
        ui.horizontal_centered(|ui| {
            if let Some(summary) = payload["summary"].as_str() {
                ui.label(RichText::new(summary).size(11.5).color(tokens.text_dim));
            }
            for (index, (name, _)) in series.iter().enumerate().take(6) {
                design::dot(ui, tokens.chart[index % tokens.chart.len()]);
                ui.label(RichText::new(name).size(11.0).color(tokens.text_faint));
            }
        });
    });
}

fn format_axis(value: f64) -> String {
    if value.abs() >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value.abs() >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if value.abs() >= 10.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// Server monitor: sessions, status, variables, dashboard.
pub fn monitor(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let payload = session.tabs[tab_index]
        .payload
        .get("monitor")
        .cloned()
        .unwrap_or_default();
    let kind = payload["kind"].as_str().unwrap_or("dashboard").to_string();
    let conn = session.tabs[tab_index].conn;
    let mut requested: Option<&'static str> = None;

    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("Server monitor")
                    .size(13.5)
                    .color(tokens.text),
            );
            if let Some(index) = conn {
                design::chip(
                    ui,
                    tokens,
                    &session
                        .conns
                        .get(index)
                        .map(|c| c.cfg.name.clone())
                        .unwrap_or_default(),
                    tokens.info,
                );
            }
            if let Some(index) = design::segmented(
                ui,
                tokens,
                &[
                    ("Dashboard", kind == "dashboard"),
                    ("Sessions", kind == "processes"),
                    ("Status", kind == "status"),
                    ("Variables", kind == "variables"),
                ],
            ) {
                requested = Some(match index {
                    1 => "processes",
                    2 => "status",
                    3 => "variables",
                    _ => "dashboard",
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "Refresh", true).clicked() {
                    if let Some(index) = conn {
                        state.request_monitor(session, index, &kind);
                    }
                }
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    if let (Some(kind), Some(index)) = (requested, conn) {
        state.request_monitor(session, index, kind);
    }

    match kind.as_str() {
        "processes" => {
            let sessions = payload["sessions"].as_array().cloned().unwrap_or_default();
            let column_specs = specs(
                &["id", "user", "database", "state", "elapsed_ms", "query"],
                &["text", "text", "text", "text", "int8", "text"],
            );
            let rows = rows_from(
                &sessions,
                &["id", "user", "database", "state", "elapsed_ms", "query"],
            );
            let height = (ui.available_height() - 40.0).max(160.0);
            let event = list(
                ui,
                tokens,
                List {
                    gutter: true,
                    columns: &column_specs,
                    rows: &rows,
                    height,
                    id: egui::Id::new(("sessions", tab_index)),
                    empty: "no sessions reported by this server",
                    row_menu: None,
                },
                state.table(tab_index),
            );
            let mut kill: Option<String> = None;
            if let table::TableEvent::Select(row, _) = event {
                kill = sessions
                    .get(row)
                    .and_then(|s| s["id"].as_str().map(str::to_string));
            }
            if let (Some(id), Some(index)) = (kill, conn) {
                let params = serde_json::json!({
                    "conn": session.conns[index].cfg.id,
                    "id": id,
                    "confirm": true,
                });
                match session.run("monitor.kill", params) {
                    Ok(_) => session.log(Level::Warn, format!("session {id} terminated")),
                    Err(e) => session.log(Level::Error, e.to_string()),
                }
            }
            ui.label(
                RichText::new("click a session row to terminate that connection")
                    .size(11.0)
                    .color(tokens.text_faint),
            );
        }
        "status" | "variables" => {
            let key = if kind == "status" {
                "status"
            } else {
                "variables"
            };
            let items = payload[key].as_array().cloned().unwrap_or_default();
            let needle = state.monitor_filter.trim().to_ascii_lowercase();
            let filtered: Vec<serde_json::Value> = items
                .iter()
                .filter(|item| {
                    needle.is_empty()
                        || item["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains(&needle)
                })
                .cloned()
                .collect();
            let filter_band = design::band(ui, tokens, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("Filter").size(12.0).color(tokens.text_dim));
                    ui.add(
                        egui::TextEdit::singleline(&mut state.monitor_filter)
                            .desired_width(260.0)
                            .hint_text("name contains…"),
                    );
                    design::chip(
                        ui,
                        tokens,
                        &format!("{} of {} shown", filtered.len(), items.len()),
                        tokens.text_faint,
                    );
                });
            });
            ui.painter().hline(
                filter_band.left()..=filter_band.right(),
                filter_band.bottom() - 0.5,
                Stroke::new(1.0, tokens.stroke),
            );
            let column_specs = specs(&["name", "value", "unit"], &["text", "text", "text"]);
            let rows = rows_from(&filtered, &["name", "value", "unit"]);
            let height = (ui.available_height() - 40.0).max(160.0);
            list(
                ui,
                tokens,
                List {
                    gutter: true,
                    columns: &column_specs,
                    rows: &rows,
                    height,
                    id: egui::Id::new(("monitor-list", tab_index)),
                    empty: "this server reports nothing here",
                    row_menu: None,
                },
                state.table(tab_index),
            );
        }
        _ => {
            let server = payload["server"].clone();
            let tables = payload["tables"].as_array().cloned().unwrap_or_default();
            let cards: Vec<(String, String, Color32)> = vec![
                (
                    "Engine".into(),
                    server["version"].as_str().unwrap_or("—").to_string(),
                    tokens.info,
                ),
                (
                    "Data size".into(),
                    server["size"].as_str().unwrap_or("—").to_string(),
                    tokens.ok,
                ),
                (
                    "Uptime".into(),
                    server["uptime_s"]
                        .as_i64()
                        .map(|seconds| format!("{:.1} h", seconds as f64 / 3600.0))
                        .unwrap_or_else(|| "—".into()),
                    tokens.accent,
                ),
                (
                    "Tables".into(),
                    format!(
                        "{} · {} connections",
                        payload["table_count"].as_u64().unwrap_or(0),
                        payload["connections"].as_u64().unwrap_or(0)
                    ),
                    tokens.warn,
                ),
            ];
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for (label, value, colour) in cards {
                    let frame = design::card_frame(tokens);
                    frame.show(ui, |ui| {
                        ui.set_min_width(170.0);
                        ui.label(RichText::new(label).size(11.0).color(tokens.text_faint));
                        ui.label(RichText::new(value).size(14.0).color(colour));
                    });
                }
            });
            ui.add_space(10.0);
            design::section(
                ui,
                tokens,
                "Largest tables",
                Some("bar length is relative size"),
            );
            ui.add_space(4.0);
            let max = tables
                .iter()
                .filter_map(|t| t["size_bytes"].as_f64())
                .fold(0.0f64, f64::max)
                .max(1.0);
            egui::ScrollArea::vertical()
                .id_salt(format!("monitor-tables-{tab_index}"))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for item in tables.iter().take(40) {
                        let size = item["size_bytes"].as_f64().unwrap_or(0.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(table::clip(
                                    item["table"].as_str().unwrap_or(""),
                                    32,
                                ))
                                .size(12.0)
                                .monospace()
                                .color(tokens.text),
                            );
                            design::meter(
                                ui,
                                tokens,
                                (size / max) as f32,
                                (ui.available_width() - 190.0).max(60.0),
                                tokens.accent,
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} rows · {}",
                                    item["rows"]
                                        .as_i64()
                                        .map(|r| r.to_string())
                                        .unwrap_or_else(|| "?".into()),
                                    item["size"].as_str().unwrap_or("")
                                ))
                                .size(11.0)
                                .color(tokens.text_dim),
                            );
                        });
                    }
                });
        }
    }
}

/// Automation: jobs, their schedule and their log.
pub fn jobs(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let mut run: Option<String> = None;
    let mut delete: Option<String> = None;
    let mut toggle: Option<(String, bool)> = None;
    let mut open_new = false;
    let mut run_due = false;

    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("Scheduled jobs")
                    .size(13.5)
                    .color(tokens.text),
            );
            design::chip(
                ui,
                tokens,
                &format!("{} jobs", session.jobs.jobs.len()),
                tokens.text_dim,
            );
            design::chip(
                ui,
                tokens,
                &format!(
                    "{} enabled",
                    session.jobs.jobs.iter().filter(|j| j.enabled).count()
                ),
                tokens.ok,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::primary_button(ui, tokens, "+ New job", true).clicked() {
                    open_new = true;
                }
                if design::ghost_button(ui, tokens, "Run due now", true)
                    .on_hover_text("The app already does this every 30 seconds")
                    .clicked()
                {
                    run_due = true;
                }
                design::chip(
                    ui,
                    tokens,
                    &format!(
                        "store {}",
                        nook_engine::profiles::jobs_path()
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ),
                    tokens.text_faint,
                );
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    if open_new {
        state.open_command("job.create".into());
    }
    if run_due {
        match session.run("job.runDue", serde_json::json!({})) {
            Ok(value) => session.log(
                Level::Info,
                format!("{} job(s) were due", value["due"].as_u64().unwrap_or(0)),
            ),
            Err(e) => session.log(Level::Error, e.to_string()),
        }
    }

    egui::ScrollArea::vertical()
        .id_salt("jobs")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if session.jobs.jobs.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("No jobs yet")
                            .size(13.0)
                            .color(tokens.text_dim),
                    );
                    ui.label(
                        RichText::new("A job is a command plus a schedule: nightly backup, hourly sync, a query that feeds a report.")
                            .size(11.5)
                            .color(tokens.text_faint),
                    );
                });
            }
            for job in &session.jobs.jobs {
                let frame = design::card_frame(tokens);
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        design::dot(
                            ui,
                            if job.enabled { tokens.ok } else { tokens.text_faint },
                        );
                        ui.label(RichText::new(&job.name).size(13.0).color(tokens.text));
                        design::chip(ui, tokens, job.kind.label(), tokens.info);
                        design::chip(ui, tokens, &job.schedule.describe(), tokens.text_dim);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if design::ghost_button(ui, tokens, "Delete", true).clicked() {
                                delete = Some(job.id.clone());
                            }
                            if design::ghost_button(
                                ui,
                                tokens,
                                if job.enabled { "Disable" } else { "Enable" },
                                true,
                            )
                            .clicked()
                            {
                                toggle = Some((job.id.clone(), !job.enabled));
                            }
                            if design::ghost_button(ui, tokens, "Run now", true).clicked() {
                                run = Some(job.id.clone());
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        design::chip(
                            ui,
                            tokens,
                            &format!("{} runs", job.runs),
                            tokens.text_faint,
                        );
                        if job.failures > 0 {
                            design::chip(
                                ui,
                                tokens,
                                &format!("{} failures", job.failures),
                                tokens.error,
                            );
                        }
                        ui.label(
                            RichText::new(table::clip(
                                job.last_status.as_deref().unwrap_or("never ran"),
                                90,
                            ))
                            .size(11.0)
                            .monospace()
                            .color(tokens.text_dim),
                        );
                    });
                });
                ui.add_space(6.0);
            }

            ui.add_space(8.0);
            design::section(ui, tokens, "Recent runs", None);
            ui.add_space(4.0);
            for entry in session.jobs.recent_log(None, 50) {
                ui.horizontal(|ui| {
                    design::dot(ui, if entry.ok { tokens.ok } else { tokens.error });
                    ui.label(
                        RichText::new(format!(
                            "{} · {} · {} ms",
                            entry.at, entry.job, entry.ms
                        ))
                        .size(11.0)
                        .monospace()
                        .color(tokens.text_faint),
                    );
                    ui.label(
                        RichText::new(table::clip(&entry.message, 80))
                            .size(11.0)
                            .color(if entry.ok { tokens.text_dim } else { tokens.error }),
                    );
                });
            }
        });

    if let Some(id) = run {
        match session.run("job.run", serde_json::json!({ "id": id })) {
            Ok(value) => session.log(
                Level::Success,
                format!("job {id}: {}", value["summary"].as_str().unwrap_or("done")),
            ),
            Err(e) => session.log(Level::Error, format!("job {id}: {e}")),
        }
    }
    if let Some((id, enabled)) = toggle {
        let _ = session.run(
            "job.setEnabled",
            serde_json::json!({"id": id, "enabled": enabled}),
        );
    }
    if let Some(id) = delete {
        let _ = session.run("job.delete", serde_json::json!({"id": id}));
    }
}

/// Data transfer and synchronization wizard.
#[allow(clippy::too_many_lines)]
pub fn transfer(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let conns: Vec<(String, String)> = session
        .conns
        .iter()
        .map(|c| (c.cfg.id.clone(), c.cfg.name.clone()))
        .collect();
    let mut from = session.tabs[tab_index]
        .payload
        .get("from")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| conns.first().map(|(id, _)| id.clone()))
        .unwrap_or_default();
    let mut to = session.tabs[tab_index]
        .payload
        .get("to")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let mut tables = session.tabs[tab_index]
        .payload
        .get("tables")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();
    let mut create = session.tabs[tab_index]
        .payload
        .get("create_tables")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut truncate = session.tabs[tab_index]
        .payload
        .get("truncate_target")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // The mapping editor's rows, held in the tab so it survives a repaint.
    let mut mapping: Vec<(String, String)> = session.tabs[tab_index]
        .payload
        .get("mapping")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some((
                        r.get("source")?.as_str()?.to_string(),
                        r.get("target")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let speed: Option<serde_json::Value> = session.tabs[tab_index].payload.get("speed").cloned();
    let mut action: Option<&str> = None;

    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("Data transfer").size(13.5).color(tokens.text));
            ui.label(
                RichText::new("copy tables between any two open connections, across engines")
                    .size(11.5)
                    .color(tokens.text_faint),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "Compare structures", true).clicked() {
                    action = Some("diff.structure");
                }
                if design::ghost_button(ui, tokens, "Compare data", true).clicked() {
                    action = Some("sync.data");
                }
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );

    ui.add_space(8.0);
    ui.columns(2, |columns| {
        for (side, index) in [("Source", 0), ("Target", 1)] {
            design::card_frame(tokens).show(&mut columns[index], |ui| {
                ui.set_min_width(240.0);
                ui.label(RichText::new(side).size(11.5).color(tokens.text_faint));
                let current = if index == 0 { from.clone() } else { to.clone() };
                let pick = if index == 0 { &mut from } else { &mut to };
                egui::ComboBox::from_id_salt(format!("transfer-{index}"))
                    .selected_text(
                        conns
                            .iter()
                            .find(|(id, _)| id == &current)
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| "pick a connection".into()),
                    )
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (id, name) in &conns {
                            ui.selectable_value(pick, id.clone(), name);
                        }
                    });
            });
            columns[index].add_space(4.0);
        }
    });

    ui.add_space(4.0);
    design::card_frame(tokens).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Tables").size(12.0).color(tokens.text_dim));
            ui.add(
                egui::TextEdit::singleline(&mut tables)
                    .desired_width(340.0)
                    .hint_text("users, orders  ·  * for every table"),
            );
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut create, "Create missing tables");
            ui.checkbox(&mut truncate, "Empty the target first");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::primary_button(ui, tokens, "Transfer", true).clicked() {
                    action = Some("transfer.data");
                }
                if design::ghost_button(ui, tokens, "Preview", true).clicked() {
                    action = Some("transfer.preview");
                }
                if design::ghost_button(ui, tokens, "Sync structures", true).clicked() {
                    action = Some("sync.structure");
                }
            });
        });
    });

    ui.add_space(4.0);
    design::card_frame(tokens).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Column mapping")
                    .size(12.0)
                    .color(tokens.text_dim),
            );
            ui.label(
                RichText::new("optional · source column in the file, target column in the table")
                    .size(11.0)
                    .color(tokens.text_faint),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::ghost_button(ui, tokens, "Time a page", true).clicked() {
                    action = Some("transfer.speedTest");
                }
            });
        });
        ui.add_space(4.0);
        let mut remove: Option<usize> = None;
        for (i, (source, target)) in mapping.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(source)
                        .desired_width(180.0)
                        .hint_text("source"),
                );
                ui.label(RichText::new("→").size(11.5).color(tokens.text_faint));
                ui.add(
                    egui::TextEdit::singleline(target)
                        .desired_width(180.0)
                        .hint_text("target"),
                );
                if design::ghost_button(ui, tokens, "×", true).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            mapping.remove(i);
        }
        ui.horizontal(|ui| {
            if design::ghost_button(ui, tokens, "Add a pair", true).clicked() {
                mapping.push((String::new(), String::new()));
            }
            if !mapping.is_empty() {
                design::chip(
                    ui,
                    tokens,
                    "a mapping names every column it writes; unmapped columns are left alone",
                    tokens.text_faint,
                );
            }
        });
        if let Some(report) = &speed {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} rows/s · read {} ms · write {} ms · suggested batch {}",
                    report["rows_per_second"].as_f64().unwrap_or(0.0),
                    report["read_ms"].as_u64().unwrap_or(0),
                    report["write_ms"].as_u64().unwrap_or(0),
                    report["suggested_batch"].as_u64().unwrap_or(0),
                ))
                .size(11.5)
                .color(tokens.text_dim),
            );
            ui.label(
                RichText::new(report["note"].as_str().unwrap_or(""))
                    .size(11.0)
                    .color(tokens.text_faint),
            );
        }
    });

    // The sync half. A conflict is the only thing this card has to show that the transfer card
    // cannot: a row the rules refused to settle. It is listed with its reason, because "1 conflict"
    // without the reason is a number a person cannot act on.
    let mut sync_action: Option<&str> = None;
    let sync_report: Option<serde_json::Value> =
        session.tabs[tab_index].payload.get("sync").cloned();
    design::card_frame(tokens).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Row sync").size(12.0).color(tokens.text_dim));
            ui.label(
                RichText::new(
                    "two passes · a row both sides changed is reported, never overwritten",
                )
                .size(11.0)
                .color(tokens.text_faint),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::primary_button(ui, tokens, "Merge both ways", true).clicked() {
                    sync_action = Some("sync.bidirectional");
                }
                if design::ghost_button(ui, tokens, "Delete extras", true).clicked() {
                    sync_action = Some("sync.dataDelete");
                }
            });
        });
        if let Some(report) = &sync_report {
            ui.add_space(4.0);
            ui.label(
                RichText::new(report["summary"].as_str().unwrap_or(""))
                    .size(11.5)
                    .color(tokens.text_dim),
            );
            if let Some(conflicts) = report["conflicts"].as_array() {
                for conflict in conflicts.iter().take(20) {
                    let key = conflict["key"]
                        .as_array()
                        .map(|parts| {
                            parts
                                .iter()
                                .flat_map(|p| {
                                    p.as_object().map(|o| {
                                        o.iter()
                                            .map(|(k, v)| {
                                                format!("{k}={}", v.as_str().unwrap_or(""))
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                })
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            key,
                            conflict["reason"].as_str().unwrap_or("")
                        ))
                        .size(11.0)
                        .color(tokens.warn),
                    );
                }
            }
        }
    });

    let list: Vec<String> = tables
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let action = action.or(sync_action);
    if let Some(id) = action {
        // The mapping goes through `transfer.columnMapping` so the editor and the command agree on
        // the `source=target` form rather than the view inventing its own.
        let mut column_mapping: Vec<String> = Vec::new();
        if !mapping.is_empty() {
            let rows: Vec<serde_json::Value> = mapping
                .iter()
                .filter(|(s, t)| !s.trim().is_empty() && !t.trim().is_empty())
                .map(|(s, t)| serde_json::json!({"source": s.trim(), "target": t.trim()}))
                .collect();
            match session.run("transfer.columnMapping", serde_json::json!({"map": rows})) {
                Ok(value) => {
                    column_mapping = value["columnMapping"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
        let params = match id {
            "transfer.data" => {
                let mut params = serde_json::json!({
                    "from": from, "to": to, "tables": list,
                    "create_tables": create, "truncate_target": truncate,
                });
                if !column_mapping.is_empty() {
                    params["columnMapping"] = serde_json::json!(column_mapping);
                }
                params
            }
            "transfer.speedTest" => serde_json::json!({
                "from": from, "to": to,
                "table": list.first().cloned().unwrap_or_default(),
            }),
            "transfer.preview" => serde_json::json!({"from": from, "to": to, "tables": list}),
            "sync.data" => {
                serde_json::json!({"from": from, "to": to, "dry_run": true, "tables": list})
            }
            "sync.bidirectional" => {
                // Dry by default: the card reports the conflicts first, and a second press with
                // `dry_run: false` is what writes.
                serde_json::json!({
                    "a": from, "b": to, "tables": list,
                    "dry_run": !session.tabs[tab_index].payload.contains_key("sync"),
                })
            }
            "sync.dataDelete" => {
                serde_json::json!({"from": from, "to": to, "tables": list})
            }
            "sync.structure" => serde_json::json!({"from": from, "to": to, "apply": true}),
            _ => serde_json::json!({"from": from, "to": to}),
        };
        match session.run(id, params) {
            Ok(value) => {
                let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                session.tabs[tab_index]
                    .payload
                    .insert("output".into(), serde_json::json!(text));
                if id == "transfer.data" {
                    session.log(
                        Level::Success,
                        format!("transfer: {}", value["summary"].as_str().unwrap_or("done")),
                    );
                }
                if id.starts_with("sync.") {
                    session.tabs[tab_index]
                        .payload
                        .insert("sync".into(), value.clone());
                }
                if id == "transfer.speedTest" {
                    // Kept for the card's own line, not just the raw JSON in the output pane.
                    session.tabs[tab_index]
                        .payload
                        .insert("speed".into(), value);
                }
            }
            Err(e) => session.log(Level::Error, e.to_string()),
        }
    }

    session.tabs[tab_index].payload.insert(
        "mapping".into(),
        serde_json::json!(mapping
            .iter()
            .map(|(s, t)| serde_json::json!({"source": s, "target": t}))
            .collect::<Vec<_>>()),
    );
    session.tabs[tab_index]
        .payload
        .insert("from".into(), serde_json::json!(from));
    session.tabs[tab_index]
        .payload
        .insert("to".into(), serde_json::json!(to));
    session.tabs[tab_index]
        .payload
        .insert("tables".into(), serde_json::json!(tables));
    session.tabs[tab_index]
        .payload
        .insert("create_tables".into(), serde_json::json!(create));
    session.tabs[tab_index]
        .payload
        .insert("truncate_target".into(), serde_json::json!(truncate));

    ui.add_space(8.0);
    if let Some(output) = session.tabs[tab_index]
        .payload
        .get("output")
        .and_then(|v| v.as_str())
    {
        design::card_frame(tokens).show(ui, |ui| {
            ui.label(
                RichText::new("Last run")
                    .size(11.5)
                    .color(tokens.text_faint),
            );
            egui::ScrollArea::vertical()
                .id_salt(format!("transfer-output-{tab_index}"))
                .max_height(ui.available_height().max(120.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(output)
                            .monospace()
                            .size(11.5)
                            .color(tokens.text_dim),
                    );
                });
        });
    }
    let _ = state;
}

/// Users and grants.
pub fn users(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let Some(conn_index) = session.tabs[tab_index].conn else {
        return;
    };
    let payload = session.tabs[tab_index]
        .payload
        .get("users")
        .cloned()
        .unwrap_or_default();
    let users_list = payload["users"].as_array().cloned().unwrap_or_default();
    let mut refresh = false;
    let mut create = false;

    let toolbar = design::band(ui, tokens, |ui| {
        ui.set_min_height(design::TOOLBAR_H);
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("Users and privileges")
                    .size(13.5)
                    .color(tokens.text),
            );
            design::chip(
                ui,
                tokens,
                &format!("{} users", users_list.len()),
                tokens.text_dim,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::primary_button(ui, tokens, "Create user", true).clicked() {
                    create = true;
                }
                if design::ghost_button(ui, tokens, "Refresh", true).clicked() {
                    refresh = true;
                }
            });
        });
    });
    ui.painter().hline(
        toolbar.left()..=toolbar.right(),
        toolbar.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    if create {
        state.open_command("priv.createUser".into());
    }
    if refresh {
        state.request_users(session, conn_index);
    }

    let column_specs = specs(
        &["user", "superuser", "can_log_in", "expires", "attributes"],
        &["text", "bool", "bool", "text", "text"],
    );
    let rows = rows_from(
        &users_list,
        &["display", "superuser", "can_login", "expires", "attributes"],
    );
    let height = (ui.available_height() - 130.0).max(140.0);
    let event = list(
        ui,
        tokens,
        List {
            gutter: true,
            columns: &column_specs,
            rows: &rows,
            height,
            id: egui::Id::new(("users", tab_index)),
            empty: "SQLite has no users; Postgres and MySQL do",
            row_menu: None,
        },
        state.table(tab_index),
    );
    if let table::TableEvent::Select(row, _) = event {
        if let Some(name) = users_list.get(row).and_then(|u| u["name"].as_str()) {
            match session.run(
                "priv.grants",
                serde_json::json!({"conn": session.conns[conn_index].cfg.id, "user": name}),
            ) {
                Ok(value) => {
                    let text = value["grants"]
                        .as_array()
                        .map(|g| {
                            g.iter()
                                .map(|v| v.as_str().unwrap_or("").to_string())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    session.tabs[tab_index]
                        .payload
                        .insert("grants".into(), serde_json::json!(text));
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
    }

    ui.add_space(6.0);
    if let Some(grants) = session.tabs[tab_index]
        .payload
        .get("grants")
        .and_then(|v| v.as_str())
    {
        design::card_frame(tokens).show(ui, |ui| {
            ui.label(RichText::new("Grants").size(11.5).color(tokens.text_faint));
            egui::ScrollArea::vertical()
                .id_salt(format!("grants-{tab_index}"))
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(grants)
                            .monospace()
                            .size(11.5)
                            .color(tokens.text_dim),
                    );
                });
        });
    } else {
        ui.label(
            RichText::new("click a user to read its grants")
                .size(11.0)
                .color(tokens.text_faint),
        );
    }
}

/// The start page: what this is, what to do first, and the measured feature coverage.
pub fn welcome(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let report = nook_engine::parity::measure();
    egui::ScrollArea::vertical()
        .id_salt("welcome")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let (mark, _) = ui.allocate_exact_size(Vec2::new(52.0, 52.0), Sense::hover());
                ui.painter().rect_filled(mark, CornerRadius::same(14), tokens.accent);
                ui.painter().text(
                    mark.center(),
                    Align2::CENTER_CENTER,
                    "DC",
                    tokens.font(20.0),
                    tokens.accent_text,
                );
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Nook").size(26.0).color(tokens.text));
                    ui.label(
                        RichText::new("Database administration for SQLite, PostgreSQL and MySQL — native, one command registry behind the UI, the CLI, the control channel and MCP.")
                            .size(12.5)
                            .color(tokens.text_dim),
                    );
                });
            });

            ui.add_space(20.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(24.0);
                design::card_frame(tokens).show(ui, |ui| {
                    ui.set_min_width(430.0);
                    ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Navicat feature coverage")
                            .size(12.0)
                            .color(tokens.text_faint),
                    );
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{}/{}",
                                report.covered.len(),
                                report.total()
                            ))
                            .size(24.0)
                            .color(tokens.accent),
                        );
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            design::meter(
                                ui,
                                tokens,
                                (report.percent() / 100.0) as f32,
                                240.0,
                                tokens.accent,
                            );
                            ui.label(
                                RichText::new(format!("{:.1}% measured", report.percent()))
                                    .size(11.0)
                                    .color(tokens.text_dim),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    for area in report.by_area.iter().take(5) {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:<14}", table::clip(&area.area, 14)))
                                    .size(11.0)
                                    .monospace()
                                    .color(tokens.text_dim),
                            );
                            design::meter(
                                ui,
                                tokens,
                                (area.percent() / 100.0) as f32,
                                130.0,
                                tokens.info,
                            );
                            ui.label(
                                RichText::new(format!("{}/{}", area.covered, area.total))
                                    .size(10.5)
                                    .color(tokens.text_faint),
                            );
                        });
                    }
                    });
                });

                design::card_frame(tokens).show(ui, |ui| {
                    ui.set_min_width(320.0);
                    ui.vertical(|ui| {
                    ui.label(RichText::new("Start here").size(12.5).color(tokens.text));
                    ui.add_space(4.0);
                    if design::primary_button(ui, tokens, "Add a connection", true).clicked() {
                        state.open_connection_dialog(None);
                    }
                    if design::ghost_button(ui, tokens, "Load the sample database", true).clicked() {
                        state.want_demo = true;
                    }
                    if design::ghost_button(ui, tokens, "New query tab", true).clicked() {
                        let index =
                            session.add_tab(TabKind::Query, "Query", session.active_conn);
                        state.active_tab_override = Some(index);
                    }
                    if design::ghost_button(ui, tokens, "Jobs and automation", true).clicked() {
                        let index = session.add_tab(TabKind::Jobs, "Jobs", None);
                        state.active_tab_override = Some(index);
                    }
                    if design::ghost_button(ui, tokens, "Command palette", true).clicked() {
                        state.palette_open = true;
                    }
                    ui.add_space(8.0);
                    for (keys, what) in [
                        ("Cmd K", "command palette"),
                        ("Cmd Enter", "run the query"),
                        ("Cmd S", "apply grid edits"),
                        ("Cmd W", "close the tab"),
                        ("Cmd L", "show the log"),
                    ] {
                        ui.horizontal(|ui| {
                            design::keycap(ui, tokens, keys);
                            ui.label(RichText::new(what).size(11.5).color(tokens.text_dim));
                        });
                    }
                    });
                });
            });

            ui.add_space(16.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(24.0);
                design::card_frame(tokens).show(ui, |ui| {
                    ui.set_min_width(470.0);
                    ui.vertical(|ui| {
                    ui.label(RichText::new("Still missing").size(12.5).color(tokens.text));
                    ui.add_space(2.0);
                    for (area, ids) in report.missing_by_area().into_iter().take(6) {
                        ui.label(
                            RichText::new(format!("{area}: {} gaps — {}", ids.len(), table::clip(&ids.join(", "), 44)))
                                .size(11.0)
                                .color(tokens.text_dim),
                        );
                    }
                    });
                });
                design::card_frame(tokens).show(ui, |ui| {
                    ui.set_min_width(280.0);
                    ui.vertical(|ui| {
                    ui.label(RichText::new("This session").size(12.5).color(tokens.text));
                    for (label, value) in [
                        ("connections", session.conns.len().to_string()),
                        ("saved profiles", session.profiles.list().len().to_string()),
                        ("jobs", session.jobs.jobs.len().to_string()),
                        ("snippets", session.snippets.len().to_string()),
                        ("commands run", session.history.len().to_string()),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).size(11.5).color(tokens.text_dim));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(value).size(11.5).color(tokens.text),
                                    );
                                },
                            );
                        });
                    }
                    });
                });
            });
            ui.add_space(20.0);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_tab_draws_the_table_the_edit_left_behind() {
        let mut session = crate::demo_session();
        let tab = session.add_tab(TabKind::Design, "main.customers", Some(0));
        let reference = nook_engine::TableRef {
            schema: None,
            table: "customers".into(),
        };
        session.tabs[tab].grid.table = Some(reference.clone());
        session.tabs[tab].table = Some(
            session
                .table_of(0, None, "customers")
                .expect("the demo session has customers"),
        );

        session
            .run(
                "schema.addColumn",
                serde_json::json!({
                    "conn": session.conns[0].cfg.id.clone(),
                    "table": "customers",
                    "name": "loyalty_tier",
                    "type": "TEXT",
                }),
            )
            .expect("the column is added");

        let names = |table: &nook_model::Table| {
            table
                .columns
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        let drawn = design_table(&session, tab).expect("the tab still names a table");
        assert!(
            names(&drawn).contains(&"loyalty_tier".to_string()),
            "the tab must draw the edited definition, drew {:?}",
            names(&drawn)
        );
        // The tab's own copy is the stale one this fix steps over.
        let stale = session.tabs[tab].table.clone().expect("a snapshot");
        assert!(!names(&stale).contains(&"loyalty_tier".to_string()));
    }

    /// A connection of this test's own, holding `users`: the demo session's database is one file at
    /// one path that every test in this crate shares, and this one renames a table.
    fn session_with_users(name: &str) -> (nook_engine::Session, usize) {
        let path =
            std::env::temp_dir().join(format!("nook-views-{}-{name}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut session = nook_engine::Session::new();
        session
            .conns
            .push(nook_engine::Conn::new(nook_core::ConnectionConfig::sqlite(
                path.to_string_lossy(),
            )));
        session.active_conn = Some(0);
        nook_engine::cmd::open_now(&mut session, 0).expect("the fixture database opens");
        nook_engine::cmd::run_sql(
            &mut session,
            0,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT);",
            Some(10),
        )
        .expect("the fixture schema is valid");
        nook_engine::cmd::refresh_catalog(&mut session, 0).expect("the catalog refreshes");
        let tab = session.add_tab(TabKind::Design, "main.users", Some(0));
        session.tabs[tab].grid.table = Some(nook_engine::TableRef {
            schema: None,
            table: "users".into(),
        });
        session.tabs[tab].table = Some(
            session
                .table_of(0, None, "users")
                .expect("the table is in the catalog"),
        );
        (session, tab)
    }

    /// A table renamed away from the tab is gone from the catalog, and the tab's open-time copy is
    /// then the wrong answer: drawing it keeps the old columns on screen and hands the designer
    /// buttons `users`, which no longer exists, so their Run can only fail with `no such table`.
    #[test]
    fn the_design_tab_draws_nothing_for_a_table_that_is_gone() {
        let (mut session, tab) = session_with_users("gone");
        session
            .run(
                "sql.run",
                serde_json::json!({"sql": "ALTER TABLE users RENAME TO people"}),
            )
            .expect("the table is renamed away");

        assert!(
            design_table(&session, tab).is_none(),
            "the tab must not keep drawing a table the catalog no longer holds"
        );
    }
}
