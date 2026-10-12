//! The non-grid tabs: object design, ER diagram, reports, server monitor, jobs, transfer and users.

use datacraft_engine::{Level, Session, TabKind};
use egui::{Color32, RichText};

use crate::app::UiState;
use crate::theme::Tokens;

/// Object design: columns, indexes, foreign keys and the DDL, with the designer buttons.
pub fn design(
    ui: &mut egui::Ui,
    session: &mut Session,
    tab_index: usize,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let Some(table) = session.tabs[tab_index].table.clone() else {
        ui.label("this tab has no table");
        return;
    };
    ui.horizontal(|ui| {
        ui.heading(RichText::new(table.key()).size(15.0));
        ui.label(
            RichText::new(format!(
                "{} · {} columns{}",
                table.kind.label(),
                table.columns.len(),
                table
                    .row_estimate
                    .map(|r| format!(" · ~{r} rows"))
                    .unwrap_or_default()
            ))
            .color(tokens.text_dim),
        );
        if ui.button("+ Column").clicked() {
            state.open_command("schema.addColumn&table=".to_string());
        }
        if ui.button("+ Index").clicked() {
            state.open_command("schema.createIndex&table=".to_string());
        }
        if ui.button("+ Foreign key").clicked() {
            state.open_command("schema.addForeignKey&table=".to_string());
        }
        if ui.button("Diagnostics").clicked() {
            state.open_command("schema.ddl&table=".to_string());
        }
        if ui.button("ER diagram").clicked() {
            if let Some(conn) = session.tabs[tab_index].conn {
                state.request_diagram(session, conn);
            }
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt(format!("design-{tab_index}"))
        .show(ui, |ui| {
            ui.label(RichText::new("Columns").strong());
            egui::Grid::new(format!("design-columns-{tab_index}"))
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    for header in ["name", "type", "nullable", "default", "key", "comment"] {
                        ui.label(RichText::new(header).strong().size(12.0));
                    }
                    ui.end_row();
                    for column in &table.columns {
                        ui.label(RichText::new(column.name.clone()).monospace());
                        ui.label(RichText::new(column.type_name.clone()).monospace());
                        ui.label(if column.nullable { "yes" } else { "no" });
                        ui.label(column.default.clone().unwrap_or_default());
                        ui.label(if column.primary_key {
                            "primary key"
                        } else if column.auto_increment {
                            "generated"
                        } else {
                            ""
                        });
                        ui.label(column.comment.clone());
                        ui.end_row();
                    }
                });

            if !table.indexes.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Indexes").strong());
                for index in &table.indexes {
                    ui.label(format!(
                        "{} ({}){}{}",
                        index.name,
                        index.columns.join(", "),
                        if index.unique { " unique" } else { "" },
                        if index.primary { " primary" } else { "" }
                    ));
                }
            }

            if !table.foreign_keys.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Foreign keys").strong());
                for fk in &table.foreign_keys {
                    ui.label(format!(
                        "{}: ({}) → {}({}){}",
                        fk.name,
                        fk.columns.join(", "),
                        fk.ref_table,
                        fk.ref_columns.join(", "),
                        if fk.on_delete.is_empty() || fk.on_delete.eq_ignore_ascii_case("no action")
                        {
                            String::new()
                        } else {
                            format!(" on delete {}", fk.on_delete.to_uppercase())
                        }
                    ));
                }
            }

            if let Some(ddl) = session.tabs[tab_index]
                .payload
                .get("ddl")
                .and_then(|v| v.as_str())
            {
                ui.add_space(8.0);
                ui.label(RichText::new("DDL").strong());
                let mut text = ddl.to_string();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .code_editor()
                        .desired_rows(10)
                        .desired_width(f32::INFINITY),
                );
            } else if ui.button("Show DDL").clicked() {
                if let Some(conn) = session.tabs[tab_index].conn {
                    let params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "table": table.key(),
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
    ui.horizontal(|ui| {
        ui.label(RichText::new("ER diagram").strong());
        if let Some(conn) = session.tabs[tab_index].conn {
            if ui.button("Reload").clicked() {
                state.request_diagram(session, conn);
            }
            if ui.button("Export SVG…").clicked() {
                let path = state.pick_save_path("diagram.svg");
                if let Some(path) = path {
                    let params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "path": path,
                    });
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
        ui.label(
            RichText::new("drag to pan · ctrl+scroll to zoom")
                .size(11.0)
                .color(tokens.text_dim),
        );
    });
    let Some(payload) = payload else {
        ui.label(RichText::new("loading the catalog…").color(tokens.text_dim));
        return;
    };
    let nodes = payload["nodes"].as_array().cloned().unwrap_or_default();
    let edges = payload["edges"].as_array().cloned().unwrap_or_default();

    let (response, painter) = ui.allocate_painter(
        egui::Vec2::new(ui.available_width(), ui.available_height().max(200.0)),
        egui::Sense::click_and_drag(),
    );
    painter.rect_filled(response.rect, 4.0, tokens.background);
    let origin = response.rect.min + state.diagram_offset;

    for edge in &edges {
        let (Some(from), Some(to)) = (
            nodes.get(edge["from"].as_u64().unwrap_or(0) as usize),
            nodes.get(edge["to"].as_u64().unwrap_or(0) as usize),
        ) else {
            continue;
        };
        let from_pos = egui::pos2(
            from["x"].as_f64().unwrap_or(0.0) as f32 + from["w"].as_f64().unwrap_or(140.0) as f32,
            from["y"].as_f64().unwrap_or(0.0) as f32 + 30.0,
        );
        let to_pos = egui::pos2(
            to["x"].as_f64().unwrap_or(0.0) as f32,
            to["y"].as_f64().unwrap_or(0.0) as f32 + 30.0,
        );
        let a = origin + from_pos.to_vec2() * state.diagram_zoom;
        let b = origin + to_pos.to_vec2() * state.diagram_zoom;
        let mid = egui::pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        painter.line_segment(
            [a, egui::pos2(mid.x, a.y)],
            egui::Stroke::new(1.5, tokens.accent),
        );
        painter.line_segment(
            [egui::pos2(mid.x, a.y), egui::pos2(mid.x, b.y)],
            egui::Stroke::new(1.5, tokens.accent),
        );
        painter.line_segment(
            [egui::pos2(mid.x, b.y), b],
            egui::Stroke::new(1.5, tokens.accent),
        );
        painter.circle_filled(b, 3.0, tokens.accent);
    }

    for node in &nodes {
        let x = node["x"].as_f64().unwrap_or(0.0) as f32;
        let y = node["y"].as_f64().unwrap_or(0.0) as f32;
        let w = node["w"].as_f64().unwrap_or(140.0) as f32;
        let h = node["h"].as_f64().unwrap_or(60.0) as f32;
        let rect = egui::Rect::from_min_size(
            origin + egui::vec2(x, y) * state.diagram_zoom,
            egui::vec2(w, h) * state.diagram_zoom,
        );
        painter.rect_filled(rect, 4.0, tokens.panel_alt);
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, tokens.stroke),
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 22.0)),
            4.0,
            tokens.grid_header,
        );
        painter.text(
            rect.min + egui::vec2(8.0, 11.0),
            egui::Align2::LEFT_CENTER,
            node["table"].as_str().unwrap_or(""),
            egui::FontId::proportional(12.0),
            tokens.text,
        );
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
            let is_key = node["primary_key"]
                .as_array()
                .map(|keys| keys.iter().any(|k| k.as_str() == Some(name)))
                .unwrap_or(false);
            painter.text(
                rect.min + egui::vec2(10.0, 30.0 + index as f32 * 18.0),
                egui::Align2::LEFT_CENTER,
                if is_key {
                    format!("{name} (pk)")
                } else {
                    name.to_string()
                },
                egui::FontId::monospace(11.0),
                if is_key { tokens.warn } else { tokens.text_dim },
            );
        }
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
pub fn report(ui: &mut egui::Ui, session: &mut Session, tab_index: usize, tokens: &Tokens) {
    let payload = session.tabs[tab_index]
        .payload
        .get("chart")
        .cloned()
        .unwrap_or_default();
    let kind = payload["chart"].as_str().unwrap_or("table").to_string();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Report").strong());
        if let Some(sql) = payload["sql"].as_str() {
            ui.label(
                RichText::new(shorten(sql, 90))
                    .size(11.0)
                    .color(tokens.text_dim),
            );
        }
        if let Some(conn) = session.tabs[tab_index].conn {
            for (id, label) in [("report.chart", "Chart"), ("report.pivot", "Pivot")] {
                if ui.small_button(label).clicked() {
                    let sql = payload["sql"].as_str().unwrap_or("").to_string();
                    let params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "sql": sql,
                        "chart": if id == "report.pivot" { "table" } else { "column" },
                    });
                    match session.run(id, params) {
                        Ok(value) => {
                            session.tabs[tab_index]
                                .payload
                                .insert("chart".into(), value);
                        }
                        Err(e) => session.log(Level::Error, e.to_string()),
                    }
                }
            }
        }
    });
    ui.separator();

    if payload.get("rows").is_some() && payload.get("cells").is_some() {
        // Pivot shape.
        let rows = payload["rows"].as_array().cloned().unwrap_or_default();
        let columns = payload["columns"].as_array().cloned().unwrap_or_default();
        egui::Grid::new(format!("pivot-{tab_index}"))
            .striped(true)
            .show(ui, |ui| {
                ui.label("");
                for column in &columns {
                    ui.label(RichText::new(column.as_str().unwrap_or("")).strong());
                }
                ui.end_row();
                for (index, row) in rows.iter().enumerate() {
                    ui.label(RichText::new(row.as_str().unwrap_or("")).strong());
                    let cells = payload["cells"][index]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    for cell in &cells {
                        ui.label(match cell.as_f64() {
                            Some(v) => format!("{v:.2}"),
                            None => "·".into(),
                        });
                    }
                    ui.end_row();
                }
            });
        return;
    }

    if kind == "table" {
        if let Some(result) = payload.get("result").or_else(|| payload.get("data")) {
            let columns = result["columns"].as_array().cloned().unwrap_or_default();
            let rows = result["rows"].as_array().cloned().unwrap_or_default();
            egui::Grid::new(format!("report-table-{tab_index}"))
                .striped(true)
                .show(ui, |ui| {
                    for column in &columns {
                        ui.label(RichText::new(column.as_str().unwrap_or("")).strong());
                    }
                    ui.end_row();
                    for row in rows.iter().take(200) {
                        if let Some(object) = row.as_object() {
                            for column in &columns {
                                let key = column.as_str().unwrap_or("");
                                ui.label(
                                    RichText::new(object.get(key).map(scalar).unwrap_or_default())
                                        .monospace(),
                                );
                            }
                        }
                        ui.end_row();
                    }
                });
        }
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
        ui.label(RichText::new("nothing to chart yet").color(tokens.text_dim));
        return;
    }
    let (response, painter) = ui.allocate_painter(
        egui::Vec2::new(ui.available_width(), ui.available_height().max(240.0)),
        egui::Sense::hover(),
    );
    let rect = response.rect.shrink2(egui::vec2(24.0, 20.0));
    let max = series
        .iter()
        .flat_map(|(_, values)| values.iter().cloned())
        .fold(0.0f64, f64::max)
        .max(1.0);
    let palette = [
        tokens.accent,
        tokens.ok,
        tokens.warn,
        tokens.error,
        Color32::from_rgb(0x9b, 0x7c, 0xff),
    ];

    match kind.as_str() {
        "pie" => {
            let values = series.first().map(|(_, v)| v.clone()).unwrap_or_default();
            let total: f64 = values.iter().sum::<f64>().max(0.001);
            let center = rect.center();
            let radius = rect.height().min(rect.width()) * 0.35;
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
                        palette[index % palette.len()].gamma_multiply(0.85),
                        egui::Stroke::NONE,
                    ));
                }
                let angle = start + sweep / 2.0;
                painter.text(
                    center + egui::vec2(angle.cos(), angle.sin()) * (radius * 0.7),
                    egui::Align2::CENTER_CENTER,
                    labels.get(index).cloned().unwrap_or_default(),
                    egui::FontId::proportional(11.0),
                    tokens.text,
                );
                start += sweep;
                if start >= std::f32::consts::TAU {
                    break;
                }
            }
        }
        "line" | "area" => {
            for (series_index, (name, values)) in series.iter().enumerate() {
                let points: Vec<egui::Pos2> = values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let x = if values.len() <= 1 {
                            rect.left()
                        } else {
                            rect.left() + rect.width() * index as f32 / (values.len() - 1) as f32
                        };
                        let y = rect.bottom() - (value / max) as f32 * rect.height();
                        egui::pos2(x, y)
                    })
                    .collect();
                painter.add(egui::Shape::line(
                    points.clone(),
                    egui::Stroke::new(2.0, palette[series_index % palette.len()]),
                ));
                if let Some((name, _)) = Some((name, &series_index)) {
                    painter.text(
                        rect.left_top() + egui::vec2(0.0, series_index as f32 * 14.0),
                        egui::Align2::LEFT_TOP,
                        name,
                        egui::FontId::proportional(11.0),
                        palette[series_index % palette.len()],
                    );
                }
            }
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                egui::Stroke::new(1.0, tokens.stroke),
            );
        }
        _ => {
            let group_width = rect.width() / labels.len().max(1) as f32;
            let bar_width = (group_width * 0.7 / series.len().max(1) as f32).max(3.0);
            for (label_index, _label) in labels.iter().enumerate() {
                for (series_index, (_, values)) in series.iter().enumerate() {
                    let value = values.get(label_index).cloned().unwrap_or(0.0);
                    let height = (value / max) as f32 * rect.height();
                    let x = rect.left()
                        + group_width * label_index as f32
                        + group_width * 0.15
                        + bar_width * series_index as f32;
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - height),
                        egui::vec2(bar_width * 0.9, height),
                    );
                    painter.rect_filled(bar, 2.0, palette[series_index % palette.len()]);
                }
            }
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                egui::Stroke::new(1.0, tokens.stroke),
            );
            for (index, label) in labels.iter().enumerate() {
                if labels.len() > 24 && index % 2 == 1 {
                    continue;
                }
                painter.text(
                    egui::pos2(
                        rect.left() + group_width * (index as f32 + 0.5),
                        rect.bottom() + 4.0,
                    ),
                    egui::Align2::CENTER_TOP,
                    shorten(label, 14),
                    egui::FontId::proportional(10.0),
                    tokens.text_dim,
                );
            }
        }
    }
    ui.label(
        RichText::new(payload["summary"].as_str().unwrap_or(""))
            .size(11.0)
            .color(tokens.text_dim),
    );
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
    let mut requested: Option<(&str, &str)> = None;
    ui.horizontal_wrapped(|ui| {
        for (value, label) in [
            ("dashboard", "Dashboard"),
            ("processes", "Sessions"),
            ("status", "Status"),
            ("variables", "Variables"),
        ] {
            if ui.selectable_label(kind == value, label).clicked() {
                requested = Some((value, label));
            }
        }
        if let Some(conn_index) = conn {
            ui.separator();
            if ui.button("Refresh").clicked() {
                state.request_monitor(session, conn_index, &kind);
            }
            ui.label(
                RichText::new(
                    session
                        .conns
                        .get(conn_index)
                        .map(|c| c.cfg.name.clone())
                        .unwrap_or_default(),
                )
                .size(11.0)
                .color(tokens.text_dim),
            );
        }
    });
    if let (Some((value, _)), Some(conn_index)) = (requested, conn) {
        state.request_monitor(session, conn_index, value);
    }
    ui.separator();

    match kind.as_str() {
        "processes" => {
            let sessions = payload["sessions"].as_array().cloned().unwrap_or_default();
            let mut kill: Option<String> = None;
            egui::ScrollArea::vertical()
                .id_salt(format!("monitor-processes-{tab_index}"))
                .show(ui, |ui| {
                    egui::Grid::new(format!("processes-{tab_index}"))
                        .striped(true)
                        .min_col_width(70.0)
                        .show(ui, |ui| {
                            for header in
                                ["id", "user", "database", "state", "elapsed", "query", ""]
                            {
                                ui.label(RichText::new(header).strong().size(12.0));
                            }
                            ui.end_row();
                            for session_row in &sessions {
                                ui.label(session_row["id"].as_str().unwrap_or(""));
                                ui.label(session_row["user"].as_str().unwrap_or(""));
                                ui.label(session_row["database"].as_str().unwrap_or(""));
                                ui.label(session_row["state"].as_str().unwrap_or(""));
                                ui.label(
                                    session_row["elapsed_ms"]
                                        .as_i64()
                                        .map(|ms| format!("{ms} ms"))
                                        .unwrap_or_default(),
                                );
                                ui.label(shorten(session_row["query"].as_str().unwrap_or(""), 60));
                                if ui.small_button("kill").clicked() {
                                    kill = session_row["id"].as_str().map(str::to_string);
                                }
                                ui.end_row();
                            }
                        });
                });
            if let (Some(id), Some(conn_index)) = (kill, conn) {
                let params = serde_json::json!({
                    "conn": session.conns[conn_index].cfg.id,
                    "id": id,
                    "confirm": true,
                });
                match session.run("monitor.kill", params) {
                    Ok(_) => session.log(Level::Warn, format!("session {id} terminated")),
                    Err(e) => session.log(Level::Error, e.to_string()),
                }
            }
        }
        "status" | "variables" => {
            let key = if kind == "status" {
                "status"
            } else {
                "variables"
            };
            let items = payload[key].as_array().cloned().unwrap_or_default();
            let mut filter = state.monitor_filter.clone();
            ui.horizontal(|ui| {
                ui.label("Filter:");
                if ui
                    .add(egui::TextEdit::singleline(&mut filter).desired_width(200.0))
                    .changed()
                {
                    state.monitor_filter = filter.clone();
                }
            });
            let needle = state.monitor_filter.to_ascii_lowercase();
            egui::ScrollArea::vertical()
                .id_salt(format!("monitor-list-{tab_index}"))
                .show(ui, |ui| {
                    egui::Grid::new(format!("monitor-grid-{tab_index}"))
                        .striped(true)
                        .min_col_width(160.0)
                        .show(ui, |ui| {
                            for item in items.iter().take(2000) {
                                let name = item["name"].as_str().unwrap_or("");
                                if !needle.is_empty()
                                    && !name.to_ascii_lowercase().contains(&needle)
                                {
                                    continue;
                                }
                                ui.label(RichText::new(name).monospace());
                                ui.label(shorten(item["value"].as_str().unwrap_or(""), 90));
                                ui.end_row();
                            }
                        });
                });
        }
        _ => {
            let server = payload["server"].clone();
            ui.horizontal_wrapped(|ui| {
                if let Some(version) = server["version"].as_str() {
                    ui.label(RichText::new(version).strong());
                }
                if let Some(size) = server["size"].as_str() {
                    ui.label(RichText::new(format!("· {size}")).color(tokens.text_dim));
                }
                if let Some(uptime) = server["uptime_s"].as_i64() {
                    ui.label(
                        RichText::new(format!("· up {} h", uptime / 3600)).color(tokens.text_dim),
                    );
                }
                ui.label(
                    RichText::new(format!(
                        "· {} tables",
                        payload["table_count"].as_u64().unwrap_or(0)
                    ))
                    .color(tokens.text_dim),
                );
            });
            let tables = payload["tables"].as_array().cloned().unwrap_or_default();
            let max = tables
                .iter()
                .filter_map(|t| t["size_bytes"].as_f64())
                .fold(0.0f64, f64::max)
                .max(1.0);
            for table in tables.iter().take(40) {
                let size = table["size_bytes"].as_f64().unwrap_or(0.0);
                let fraction = (size / max) as f32;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(table["table"].as_str().unwrap_or(""))
                            .monospace()
                            .size(12.0),
                    );
                    let width = (ui.available_width() - 160.0).max(60.0) * fraction;
                    let (response, painter) = ui.allocate_painter(
                        egui::Vec2::new(width.max(2.0), 12.0),
                        egui::Sense::hover(),
                    );
                    painter.rect_filled(response.rect, 2.0, tokens.accent.gamma_multiply(0.8));
                    ui.label(
                        RichText::new(format!(
                            "{} rows · {}",
                            table["rows"]
                                .as_i64()
                                .map(|r| r.to_string())
                                .unwrap_or_else(|| "?".into()),
                            table["size"].as_str().unwrap_or("")
                        ))
                        .size(11.0)
                        .color(tokens.text_dim),
                    );
                });
            }
        }
    }
}

/// Automation: jobs, their schedule and their log.
pub fn jobs(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let mut run: Option<String> = None;
    let mut delete: Option<String> = None;
    let mut toggle: Option<(String, bool)> = None;
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Scheduled jobs").size(15.0));
        if ui.button("+ New job").clicked() {
            state.open_command("job.create".into());
        }
        if ui.button("Run due now").clicked() {
            match session.run("job.runDue", serde_json::json!({})) {
                Ok(value) => session.log(
                    Level::Info,
                    format!("{} job(s) were due", value["due"].as_u64().unwrap_or(0)),
                ),
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
        ui.label(
            RichText::new(format!(
                "every 30 s · store: {}",
                datacraft_engine::profiles::jobs_path().display()
            ))
            .size(11.0)
            .color(tokens.text_dim),
        );
    });
    ui.separator();
    if session.jobs.jobs.is_empty() {
        ui.label(
            RichText::new(
                "No jobs yet. A job is a command plus a schedule: nightly backup, hourly sync, a query that feeds a report.",
            )
            .color(tokens.text_dim),
        );
    } else {
        egui::Grid::new("jobs-grid")
            .striped(true)
            .min_col_width(90.0)
            .show(ui, |ui| {
                for header in ["job", "kind", "schedule", "runs", "failures", "last", ""] {
                    ui.label(RichText::new(header).strong().size(12.0));
                }
                ui.end_row();
                for job in &session.jobs.jobs {
                    ui.label(RichText::new(job.name.clone()).strong());
                    ui.label(job.kind.label());
                    ui.label(job.schedule.describe());
                    ui.label(job.runs.to_string());
                    ui.label(
                        RichText::new(job.failures.to_string()).color(if job.failures > 0 {
                            tokens.error
                        } else {
                            tokens.text
                        }),
                    );
                    ui.label(shorten(job.last_status.as_deref().unwrap_or("-"), 40));
                    ui.horizontal(|ui| {
                        if ui.small_button("run").clicked() {
                            run = Some(job.id.clone());
                        }
                        let label = if job.enabled { "disable" } else { "enable" };
                        if ui.small_button(label).clicked() {
                            toggle = Some((job.id.clone(), !job.enabled));
                        }
                        if ui.small_button("delete").clicked() {
                            delete = Some(job.id.clone());
                        }
                    });
                    ui.end_row();
                }
            });
    }

    ui.add_space(10.0);
    ui.label(RichText::new("Recent runs").strong());
    egui::ScrollArea::vertical()
        .id_salt("job-log")
        .max_height(200.0)
        .show(ui, |ui| {
            for entry in session.jobs.recent_log(None, 50) {
                ui.label(
                    RichText::new(format!(
                        "{} · {} · {} ms · {}{}",
                        entry.at,
                        entry.job,
                        entry.ms,
                        if entry.ok { "" } else { "FAILED: " },
                        shorten(&entry.message, 80)
                    ))
                    .size(11.0)
                    .monospace()
                    .color(if entry.ok {
                        tokens.text_dim
                    } else {
                        tokens.error
                    }),
                );
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
    let from = session.tabs[tab_index]
        .payload
        .get("from")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| conns.first().map(|(id, _)| id.clone()))
        .unwrap_or_default();
    let to = session.tabs[tab_index]
        .payload
        .get("to")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let mut from = from;
    let mut to = to;
    let mut tables = session.tabs[tab_index]
        .payload
        .get("tables")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();

    ui.heading(RichText::new("Data transfer").size(15.0));
    ui.label(
        RichText::new("Copy tables between any two open connections, across engines.")
            .color(tokens.text_dim),
    );
    ui.separator();
    egui::Grid::new(format!("transfer-{tab_index}"))
        .min_col_width(90.0)
        .show(ui, |ui| {
            ui.label("From");
            egui::ComboBox::from_id_salt("transfer-from")
                .selected_text(
                    conns
                        .iter()
                        .find(|(id, _)| id == &from)
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "pick a connection".into()),
                )
                .show_ui(ui, |ui| {
                    for (id, name) in &conns {
                        ui.selectable_value(&mut from, id.clone(), name);
                    }
                });
            ui.end_row();
            ui.label("To");
            egui::ComboBox::from_id_salt("transfer-to")
                .selected_text(
                    conns
                        .iter()
                        .find(|(id, _)| id == &to)
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "pick a connection".into()),
                )
                .show_ui(ui, |ui| {
                    for (id, name) in &conns {
                        ui.selectable_value(&mut to, id.clone(), name);
                    }
                });
            ui.end_row();
            ui.label("Tables");
            ui.add(
                egui::TextEdit::singleline(&mut tables)
                    .desired_width(320.0)
                    .hint_text("users, orders  ·  * for every table"),
            );
            ui.end_row();
        });

    let create = session.tabs[tab_index]
        .payload
        .get("create_tables")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let truncate = session.tabs[tab_index]
        .payload
        .get("truncate_target")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut create_mut = create;
    let mut truncate_mut = truncate;
    ui.horizontal(|ui| {
        ui.checkbox(&mut create_mut, "Create missing tables");
        ui.checkbox(&mut truncate_mut, "Empty the target first");
    });

    ui.horizontal(|ui| {
        if ui.button("Preview").clicked() {
            let params = serde_json::json!({
                "from": from, "to": to,
                "tables": tables.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>(),
            });
            match session.run("transfer.preview", params) {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    session.tabs[tab_index]
                        .payload
                        .insert("output".into(), serde_json::json!(text));
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
        if ui.button(RichText::new("Transfer").strong()).clicked() {
            let params = serde_json::json!({
                "from": from, "to": to,
                "tables": tables.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>(),
                "create_tables": create_mut,
                "truncate_target": truncate_mut,
            });
            match session.run("transfer.data", params) {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    session.tabs[tab_index]
                        .payload
                        .insert("output".into(), serde_json::json!(text));
                    session.log(
                        Level::Success,
                        format!("transfer: {}", value["summary"].as_str().unwrap_or("done")),
                    );
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
        if ui.button("Compare data (dry run)").clicked() {
            let params = serde_json::json!({
                "from": from, "to": to, "dry_run": true,
                "tables": tables.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>(),
            });
            match session.run("sync.data", params) {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    session.tabs[tab_index]
                        .payload
                        .insert("output".into(), serde_json::json!(text));
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
        if ui.button("Compare structures").clicked() {
            match session.run(
                "diff.structure",
                serde_json::json!({"from": from, "to": to}),
            ) {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    session.tabs[tab_index]
                        .payload
                        .insert("output".into(), serde_json::json!(text));
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
        if ui.button("Synchronize structures").clicked() {
            let params = serde_json::json!({"from": from, "to": to, "apply": true});
            match session.run("sync.structure", params) {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    session.tabs[tab_index]
                        .payload
                        .insert("output".into(), serde_json::json!(text));
                }
                Err(e) => session.log(Level::Error, e.to_string()),
            }
        }
    });

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
        .insert("create_tables".into(), serde_json::json!(create_mut));
    session.tabs[tab_index]
        .payload
        .insert("truncate_target".into(), serde_json::json!(truncate_mut));

    ui.separator();
    if let Some(output) = session.tabs[tab_index]
        .payload
        .get("output")
        .and_then(|v| v.as_str())
    {
        egui::ScrollArea::vertical()
            .id_salt(format!("transfer-output-{tab_index}"))
            .show(ui, |ui| {
                ui.label(RichText::new(output).monospace().size(11.0));
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
    let conn = session.tabs[tab_index].conn;
    let Some(conn_index) = conn else {
        ui.label("this tab has no connection");
        return;
    };
    let payload = session.tabs[tab_index]
        .payload
        .get("users")
        .cloned()
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Users and privileges").size(15.0));
        if ui.button("Refresh").clicked() {
            state.request_users(session, conn_index);
        }
        if ui.button("Create user").clicked() {
            state.open_command("priv.createUser".into());
        }
    });
    ui.separator();
    let mut grants_for: Option<String> = None;
    let users = payload["users"].as_array().cloned().unwrap_or_default();
    if users.is_empty() {
        ui.label(
            RichText::new(
                "No users listed. SQLite has no concept of users; Postgres and MySQL do.",
            )
            .color(tokens.text_dim),
        );
    }
    egui::Grid::new(format!("users-{tab_index}"))
        .striped(true)
        .min_col_width(120.0)
        .show(ui, |ui| {
            for header in ["user", "superuser", "can log in", "expires", ""] {
                ui.label(RichText::new(header).strong().size(12.0));
            }
            ui.end_row();
            for user in &users {
                ui.label(RichText::new(user["display"].as_str().unwrap_or("")).monospace());
                ui.label(if user["superuser"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    ""
                });
                ui.label(if user["can_login"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                });
                ui.label(user["expires"].as_str().unwrap_or(""));
                if ui.small_button("grants").clicked() {
                    grants_for = user["name"].as_str().map(str::to_string);
                }
                ui.end_row();
            }
        });
    if let Some(name) = grants_for {
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
    if let Some(grants) = session.tabs[tab_index]
        .payload
        .get("grants")
        .and_then(|v| v.as_str())
    {
        ui.separator();
        ui.label(RichText::new("Grants").strong());
        ui.label(RichText::new(grants).monospace().size(11.0));
    }
}

/// The start page: what this is, what to do first, and the measured feature coverage.
pub fn welcome(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    ui.heading(RichText::new("DataCraft").size(22.0));
    ui.label(
        RichText::new(
            "Fast, open database administration for SQLite, PostgreSQL and MySQL — native, with one command registry behind the UI, the CLI and MCP.",
        )
        .color(tokens.text_dim),
    );
    ui.add_space(8.0);
    let report = datacraft_engine::parity::measure();
    ui.label(RichText::new(report.summary()).strong());
    ui.label(
        RichText::new(
            "Coverage is measured, not claimed: `datacraft-cli parity` regenerates the checklist.",
        )
        .size(11.0)
        .color(tokens.text_dim),
    );
    ui.add_space(12.0);
    ui.columns(2, |columns| {
        columns[0].label(RichText::new("Start here").strong());
        let mut open_conn = false;
        if columns[0].button("Add a connection").clicked() {
            open_conn = true;
        }
        if columns[0].button("Load the sample database").clicked() {
            open_conn = false;
            state.want_demo = true;
        }
        if columns[0].button("Query tab").clicked() {
            let index = session.add_tab(TabKind::Query, "Query", session.active_conn);
            state.active_tab_override = Some(index);
        }
        if columns[0].button("Jobs and automation").clicked() {
            let index = session.add_tab(TabKind::Jobs, "Jobs", None);
            state.active_tab_override = Some(index);
        }
        if columns[0].button("Command palette").clicked() {
            state.palette_open = true;
        }
        if open_conn {
            state.open_connection_dialog(None);
        }
        columns[0].add_space(8.0);
        columns[0].label(
            RichText::new(
                "Cmd+K  command palette\nCmd+Enter  run the query\nCmd+S  apply grid edits\nCmd+W  close the tab\nCmd+O  open the selected connection",
            )
            .monospace()
            .size(11.0)
            .color(tokens.text_dim),
        );

        columns[1].label(RichText::new("Not there yet").strong());
        for (area, ids) in report.missing_by_area().into_iter().take(6) {
            columns[1].label(
                RichText::new(format!("{area}: {} gaps", ids.len()))
                    .size(11.0)
                    .color(tokens.text_dim),
            );
        }
        columns[1].add_space(6.0);
        columns[1].label(
            RichText::new(format!(
                "{} profiles · {} jobs · {} messages",
                session.profiles.list().len(),
                session.jobs.jobs.len(),
                session.messages.len()
            ))
            .size(11.0)
            .color(tokens.text_dim),
        );
    });
}

fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".into(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn shorten(text: &str, limit: usize) -> String {
    let flat = text.replace('\n', " ");
    if flat.chars().count() > limit {
        format!("{}…", flat.chars().take(limit - 1).collect::<String>())
    } else {
        flat
    }
}
