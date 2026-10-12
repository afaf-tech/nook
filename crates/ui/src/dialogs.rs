//! Modal windows: connection editor, command runner, data generation, export, preferences, palette.
//! They all share one frame so a dialog looks like a dialog everywhere.

use datacraft_core::{DriverKind, SslMode};
use datacraft_engine::{Level, Session};
use egui::{CornerRadius, RichText, Stroke};

use crate::app::UiState;
use crate::table;
use crate::theme::{self as design, Tokens};

/// The one frame every dialog uses.
fn dialog(ctx: &egui::Context, tokens: &Tokens) -> egui::Frame {
    egui::Frame::window(&ctx.style_of(ctx.theme()))
        .fill(tokens.panel)
        .stroke(Stroke::new(1.0, tokens.stroke_strong))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(14))
}

pub fn show(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    connection_editor(ui, session, state, tokens);
    command_runner(ui, session, state, tokens);
    datagen_dialog(ui, session, state, tokens);
    export_dialog(ui, session, state, tokens);
    settings_dialog(ui, session, state, tokens);
    palette(ui, session, state, tokens);
}

/// One labelled field row, so the connection form reads like a form and not like a grid.
fn field(ui: &mut egui::Ui, tokens: &Tokens, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(label).size(11.5).color(tokens.text_dim));
    ui.horizontal(|ui| add(ui));
    ui.add_space(2.0);
}

fn connection_editor(
    ui: &mut egui::Ui,
    session: &mut Session,
    state: &mut UiState,
    tokens: &Tokens,
) {
    if !state.connection_open {
        return;
    }
    let editing = state.connection_editing;
    let title = if editing.is_some() {
        "Edit connection"
    } else {
        "New connection"
    };
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    egui::Window::new(title)
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .default_width(540.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(500.0);
            let form = &mut state.connection_form;

            design::card_frame(tokens).show(ui, |ui| {
                field(ui, tokens, "Name", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut form.name)
                            .desired_width(320.0)
                            .hint_text("shop production"),
                    );
                });
                field(ui, tokens, "Engine", |ui| {
                    egui::ComboBox::from_id_salt("connection-kind")
                        .selected_text(form.kind.label())
                        .width(320.0)
                        .show_ui(ui, |ui| {
                            for kind in [
                                DriverKind::Sqlite,
                                DriverKind::Postgres,
                                DriverKind::MySql,
                                DriverKind::MariaDb,
                            ] {
                                ui.selectable_value(&mut form.kind, kind, kind.label());
                            }
                            ui.separator();
                            for kind in [
                                DriverKind::SqlServer,
                                DriverKind::Oracle,
                                DriverKind::Mongo,
                                DriverKind::Redis,
                            ] {
                                ui.selectable_value(
                                    &mut form.kind,
                                    kind,
                                    format!("{} (catalogued, no driver)", kind.label()),
                                );
                            }
                        });
                    if form.kind.is_file_based() {
                        ui.add(
                            egui::TextEdit::singleline(&mut form.file)
                                .desired_width(220.0)
                                .hint_text("/path/to/file.db"),
                        );
                        if design::ghost_button(ui, tokens, "…", true).clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_title("SQLite database")
                                .save_file()
                            {
                                form.file = path.to_string_lossy().into_owned();
                            }
                        }
                    }
                });
                field(ui, tokens, "Group", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut form.group)
                            .desired_width(320.0)
                            .hint_text("production, staging, local"),
                    );
                });
            });

            if !form.kind.is_file_based() {
                ui.add_space(8.0);
                design::card_frame(tokens).show(ui, |ui| {
                    ui.label(RichText::new("Server").size(11.5).color(tokens.text_dim));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut form.host)
                                .desired_width(220.0)
                                .hint_text("127.0.0.1"),
                        );
                        let mut port = form.port as i32;
                        if ui
                            .add(egui::DragValue::new(&mut port).range(1..=65535))
                            .changed()
                        {
                            form.port = port.clamp(1, 65535) as u16;
                        }
                        egui::ComboBox::from_id_salt("connection-ssl")
                            .selected_text(format!("SSL {}", form.ssl.label()))
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for mode in [
                                    SslMode::Disable,
                                    SslMode::Prefer,
                                    SslMode::Require,
                                    SslMode::VerifyCa,
                                    SslMode::VerifyFull,
                                ] {
                                    ui.selectable_value(&mut form.ssl, mode, mode.label());
                                }
                            });
                    });
                    ui.add_space(4.0);
                    field(ui, tokens, "User", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut form.user).desired_width(180.0));
                        ui.label(RichText::new("Password").size(11.5).color(tokens.text_dim));
                        ui.add(
                            egui::TextEdit::singleline(&mut form.password)
                                .password(true)
                                .desired_width(180.0),
                        );
                    });
                    field(ui, tokens, "Database", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut form.database).desired_width(320.0));
                    });
                });
            }

            ui.add_space(8.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut form.ssh_enabled, "SSH tunnel");
                    ui.label(
                        RichText::new("through an ssh jump host, using your ssh config and agent")
                            .size(11.0)
                            .color(tokens.text_faint),
                    );
                });
                if form.ssh_enabled {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut form.ssh_host)
                                .hint_text("jump.example.com")
                                .desired_width(200.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut form.ssh_user)
                                .hint_text("user")
                                .desired_width(100.0),
                        );
                        let mut ssh_port = form.ssh_port as i32;
                        if ui
                            .add(egui::DragValue::new(&mut ssh_port).range(1..=65535))
                            .changed()
                        {
                            form.ssh_port = ssh_port.clamp(1, 65535) as u16;
                        }
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut form.ssh_key)
                            .hint_text("~/.ssh/id_ed25519 (empty: agent and config)")
                            .desired_width(340.0),
                    );
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut form.read_only, "read-only");
                ui.checkbox(
                    &mut form.keychain,
                    "keep the password in the macOS keychain",
                );
                if !form.kind.implemented() {
                    design::chip(ui, tokens, "no driver yet — catalogued only", tokens.warn);
                }
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Save", true).clicked() {
                    let name = form.name.clone();
                    let mut params = serde_json::json!({
                        "name": name,
                        "kind": format!("{:?}", form.kind).to_lowercase(),
                        "host": form.host,
                        "port": form.port,
                        "user": form.user,
                        "database": form.database,
                        "file": form.file,
                        "group": form.group,
                        "ssl": form.ssl.label(),
                        "read_only": form.read_only,
                        "keychain": form.keychain,
                        "ssh": {
                            "enabled": form.ssh_enabled,
                            "host": form.ssh_host,
                            "port": form.ssh_port,
                            "user": form.ssh_user,
                            "key_file": form.ssh_key,
                        },
                    });
                    if !form.password.is_empty() {
                        params["password"] = serde_json::json!(form.password.clone());
                    }
                    if let Some(index) = editing {
                        if let Some(conn) = session.conns.get_mut(index) {
                            if let Some(obj) = params.as_object_mut() {
                                obj.insert("conn".into(), serde_json::json!(conn.cfg.id.clone()));
                            }
                        }
                        match session.run("conn.edit", params.clone()) {
                            Ok(_) => {
                                if !form.ssh_host.is_empty() {
                                    let _ = session.run("conn.setSshTunnel", params["ssh"].clone());
                                }
                                if form.ssh_enabled {
                                    let id = session.conns[index].cfg.id.clone();
                                    let _ = session.run(
                                        "conn.setSshTunnel",
                                        serde_json::json!({"conn": id, "enabled": true}),
                                    );
                                }
                                session.log(Level::Success, "connection updated");
                                close = true;
                            }
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    } else {
                        match session.run("conn.new", params.clone()) {
                            Ok(_) => {
                                if form.ssh_enabled {
                                    let id = session
                                        .conns
                                        .last()
                                        .map(|c| c.cfg.id.clone())
                                        .unwrap_or_default();
                                    let _ = session.run(
                                        "conn.setSshTunnel",
                                        serde_json::json!({"conn": id, "enabled": true}),
                                    );
                                }
                                session.log(Level::Success, format!("connection '{name}' saved"));
                                // Connect straight away: that is what the user asked for.
                                let index = session.conns.len().saturating_sub(1);
                                if let Err(e) = datacraft_engine::cmd::open_now(session, index) {
                                    session.log(Level::Warn, format!("saved, not connected: {e}"));
                                }
                                close = true;
                            }
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    }
                }
                if let Some(index) = editing {
                    if design::ghost_button(ui, tokens, "Connect", true).clicked() {
                        if let Err(e) = datacraft_engine::cmd::open_now(session, index) {
                            session.log(Level::Warn, e.to_string());
                        }
                    }
                    if design::ghost_button(ui, tokens, "Test", true).clicked() {
                        let params =
                            serde_json::json!({"conn": session.conns[index].cfg.id.clone()});
                        match session.run("conn.test", params) {
                            Ok(value) => session.log(
                                Level::Success,
                                format!(
                                    "ok: {} · {} tables · {} ms",
                                    value["version"].as_str().unwrap_or(""),
                                    value["tables"].as_u64().unwrap_or(0),
                                    value["elapsed_ms"].as_u64().unwrap_or(0)
                                ),
                            ),
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    }
                    if design::ghost_button(ui, tokens, "Delete", true)
                        .on_hover_text("Remove the profile")
                        .clicked()
                    {
                        let params =
                            serde_json::json!({"conn": session.conns[index].cfg.id.clone()});
                        match session.run("conn.delete", params) {
                            Ok(_) => close = true,
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if design::ghost_button(ui, tokens, "Cancel", true).clicked() {
                        close = true;
                    }
                });
            });
        });
    if close || !open {
        state.connection_open = false;
    }
}

fn command_runner(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    if !state.command_open {
        return;
    }
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    egui::Window::new("Run a command")
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .default_width(620.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(580.0);
            ui.label(
                RichText::new("Every action in DataCraft is a command with an id and JSON parameters — this is the same registry the palette, the CLI and MCP use.")
                    .size(11.0)
                    .color(tokens.text_faint),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Command").size(11.5).color(tokens.text_dim));
                egui::ComboBox::from_id_salt("command-id")
                    .selected_text(state.command_id.clone())
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        let mut group = "";
                        for spec in datacraft_engine::registry() {
                            if spec.group != group {
                                group = spec.group;
                                ui.label(
                                    RichText::new(group)
                                        .size(10.5)
                                        .color(tokens.text_faint),
                                );
                            }
                            if ui
                                .selectable_label(
                                    state.command_id == spec.id,
                                    format!("{}  ·  {}", spec.id, spec.label),
                                )
                                .clicked()
                            {
                                state.command_id = spec.id.to_string();
                                state.command_params = if spec.params.is_empty() {
                                    "{}".into()
                                } else {
                                    spec.params.to_string()
                                };
                            }
                        }
                    });
                if let Some(spec) = datacraft_engine::find(&state.command_id) {
                    design::chip(ui, tokens, spec.group, tokens.info);
                }
            });
            if let Some(spec) = datacraft_engine::find(&state.command_id) {
                if !spec.help.is_empty() {
                    ui.label(RichText::new(spec.help).size(11.0).color(tokens.text_faint));
                }
            }
            ui.add_space(4.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("Parameters").size(11.5).color(tokens.text_dim));
                ui.add(
                    egui::TextEdit::multiline(&mut state.command_params)
                        .code_editor()
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Run", true).clicked() {
                    match serde_json::from_str::<serde_json::Value>(&state.command_params) {
                        Ok(params) => {
                            let id = state.command_id.clone();
                            match session.run(&id, params) {
                                Ok(value) => {
                                    let text = serde_json::to_string_pretty(&value)
                                        .unwrap_or_else(|_| value.to_string());
                                    state.command_result = Some(text);
                                    session.log(Level::Success, format!("{id} ok"));
                                }
                                Err(e) => {
                                    state.command_result = Some(format!("error: {e}"));
                                    session.log(Level::Error, format!("{id}: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            state.command_result =
                                Some(format!("parameters are not valid JSON: {e}"));
                        }
                    }
                }
                if design::ghost_button(ui, tokens, "Insert the connection id", true).clicked() {
                    if let Some(index) = session.active_conn {
                        let id = session.conns[index].cfg.id.clone();
                        if let Ok(mut value) =
                            serde_json::from_str::<serde_json::Value>(&state.command_params)
                        {
                            if let Some(obj) = value.as_object_mut() {
                                obj.insert("conn".into(), serde_json::json!(id));
                                state.command_params =
                                    serde_json::to_string_pretty(&value).unwrap_or_default();
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if design::ghost_button(ui, tokens, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
            if let Some(result) = &state.command_result {
                ui.add_space(6.0);
                design::card_frame(tokens).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("command-result")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(result)
                                    .monospace()
                                    .size(11.5)
                                    .color(tokens.text_dim),
                            );
                        });
                });
            }
        });
    if close || !open {
        state.command_open = false;
    }
}

fn datagen_dialog(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let Some((conn, table_ref)) = state.datagen_target.clone() else {
        return;
    };
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    // The form is edited on a copy: the closures below also borrow the rest of the state.
    let mut form = state.datagen_form.clone();
    let mut preview: Option<String> = None;
    egui::Window::new(format!("Generate data · {}", table_ref.key()))
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(560.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Rows").size(11.5).color(tokens.text_dim));
                    let mut rows = form.rows as i64;
                    if ui
                        .add(egui::DragValue::new(&mut rows).range(1..=100_000))
                        .changed()
                    {
                        form.rows = rows.clamp(1, 100_000) as usize;
                    }
                    ui.label(RichText::new("Seed").size(11.5).color(tokens.text_dim));
                    let mut seed = form.seed as i64;
                    if ui
                        .add(egui::DragValue::new(&mut seed).range(1..=1_000_000))
                        .changed()
                    {
                        form.seed = seed.max(1) as u64;
                    }
                    design::chip(ui, tokens, "deterministic: same seed, same rows", tokens.text_faint);
                });
            });
            ui.add_space(8.0);
            design::section(ui, tokens, "Per column", Some("leave a column on auto to let the generator decide"));
            ui.add_space(4.0);
            let columns = session
                .conns
                .get(conn)
                .and_then(|c| c.catalog.as_ref())
                .and_then(|cat| cat.find_table(table_ref.schema.as_deref(), &table_ref.table))
                .map(|t| t.columns.clone())
                .unwrap_or_default();
            egui::ScrollArea::vertical()
                .id_salt("datagen-columns")
                .max_height(240.0)
                .show(ui, |ui| {
                    for column in &columns {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(table::clip(&column.name, 22))
                                    .monospace()
                                    .size(12.0)
                                    .color(tokens.text),
                            );
                            ui.label(
                                RichText::new(table::clip(&column.type_name, 18))
                                    .size(11.0)
                                    .color(tokens.text_faint),
                            );
                            let spec = form
                                .overrides
                                .entry(column.name.clone())
                                .or_insert_with(|| "auto".to_string());
                            ui.add(
                                egui::TextEdit::singleline(spec)
                                    .desired_width(280.0)
                                    .hint_text(
                                        "auto|int:1-100|pick:a|b|pattern:INV-####|email|sentence|uuid|null",
                                    ),
                            );
                        });
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Insert", true).clicked() {
                    let mut params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "table": table_ref.key(),
                        "rows": form.rows,
                        "seed": form.seed,
                        "apply": true,
                    });
                    params["overrides"] = serde_json::json!(form.overrides);
                    match session.run("tools.datagen", params) {
                        Ok(value) => session.log(
                            Level::Success,
                            format!("inserted {} rows", value["inserted"].as_u64().unwrap_or(0)),
                        ),
                        Err(e) => session.log(Level::Error, e.to_string()),
                    }
                }
                if design::ghost_button(ui, tokens, "Preview", true).clicked() {
                    let mut params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "table": table_ref.key(),
                        "rows": form.rows,
                        "seed": form.seed,
                    });
                    params["overrides"] = serde_json::json!(form.overrides);
                    match session.run("tools.datagen", params) {
                        Ok(value) => {
                            preview = Some(serde_json::to_string_pretty(&value).unwrap_or_default());
                        }
                        Err(e) => session.log(Level::Error, e.to_string()),
                    }
                }
                if design::ghost_button(ui, tokens, "Write .sql…", true).clicked() {
                    if let Some(path) =
                        state.pick_save_path(&format!("{}.sql", table_ref.table))
                    {
                        let mut params = serde_json::json!({
                            "conn": session.conns[conn].cfg.id,
                            "table": table_ref.key(),
                            "rows": form.rows,
                            "seed": form.seed,
                            "out": path,
                        });
                        params["overrides"] = serde_json::json!(form.overrides);
                        match session.run("tools.datagen", params) {
                            Ok(value) => session.log(
                                Level::Success,
                                format!("wrote {}", value["path"].as_str().unwrap_or("")),
                            ),
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if design::ghost_button(ui, tokens, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
        });
    state.datagen_form = form;
    if let Some(preview) = preview {
        state.command_result = Some(preview);
        state.command_open = true;
        state.palette_open = false;
    }
    if close || !open {
        state.datagen_target = None;
    }
}

fn export_dialog(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let is_grid = state.export_grid_format.is_some();
    let Some(conn) = state.export_conn.or(session.active_conn) else {
        return;
    };
    if state.export_table.is_none() && !is_grid {
        return;
    }
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    let table_ref = state.export_table.clone();
    let format = state
        .export_grid_format
        .clone()
        .unwrap_or_else(|| "sql".into());
    egui::Window::new("Export")
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .default_width(520.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(480.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(match &table_ref {
                        Some(t) => format!("Table {}", t.key()),
                        None => "The visible grid page".into(),
                    })
                    .size(13.0)
                    .color(tokens.text),
                );
                design::chip(
                    ui,
                    tokens,
                    &format!("opened for {format}"),
                    tokens.text_faint,
                );
            });
            ui.add_space(6.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("Format").size(11.5).color(tokens.text_dim));
                let names = ["csv", "json", "sql", "markdown"];
                let options: Vec<(&str, bool)> = names
                    .iter()
                    .map(|candidate| (*candidate, state.export_format == *candidate))
                    .collect();
                if let Some(index) = design::segmented(ui, tokens, &options) {
                    state.export_format = names[index].to_string();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Choose file and export", true).clicked() {
                    let default = match &table_ref {
                        Some(t) => format!("{}.{}", t.table, extension(&state.export_format)),
                        None => format!("grid.{}", extension(&state.export_format)),
                    };
                    if let Some(path) = state.pick_save_path(&default) {
                        let result = match (&table_ref, is_grid) {
                            (Some(t), _) => session.run(
                                "grid.exportTable",
                                serde_json::json!({
                                    "conn": session.conns[conn].cfg.id,
                                    "table": t.key(),
                                    "format": state.export_format,
                                    "path": path,
                                }),
                            ),
                            (None, true) => session.run(
                                "grid.export",
                                serde_json::json!({"format": state.export_format, "path": path}),
                            ),
                            _ => Err(datacraft_core::Error::config("nothing to export")),
                        };
                        match result {
                            Ok(value) => {
                                session.log(
                                    Level::Success,
                                    format!(
                                        "exported {} rows to {}",
                                        value["lines"].as_u64().unwrap_or(0),
                                        value["path"].as_str().unwrap_or("")
                                    ),
                                );
                                close = true;
                            }
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    }
                }
                if design::ghost_button(ui, tokens, "Import a file…", true).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("CSV, TSV or JSON to import")
                        .add_filter("data", &["csv", "json", "tsv", "txt"])
                        .pick_file()
                    {
                        let target = table_ref
                            .as_ref()
                            .map(|t| t.key())
                            .unwrap_or_else(|| "imported".into());
                        let command = if path
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .ends_with(".json")
                        {
                            "tools.importJson"
                        } else {
                            "tools.importCsv"
                        };
                        match session.run(
                            command,
                            serde_json::json!({
                                "conn": session.conns[conn].cfg.id,
                                "path": path.to_string_lossy(),
                                "table": target,
                                "create_table": true,
                            }),
                        ) {
                            Ok(value) => session.log(
                                Level::Success,
                                format!(
                                    "imported {} rows into {}",
                                    value["inserted"].as_u64().unwrap_or(0),
                                    value["table"].as_str().unwrap_or("")
                                ),
                            ),
                            Err(e) => session.log(Level::Error, e.to_string()),
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if design::ghost_button(ui, tokens, "Cancel", true).clicked() {
                        close = true;
                    }
                });
            });
        });
    if close || !open {
        state.export_table = None;
        state.export_grid_format = None;
    }
}

fn extension(format: &str) -> &'static str {
    match format {
        "json" => "json",
        "sql" => "sql",
        "markdown" => "md",
        _ => "csv",
    }
}

fn settings_dialog(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    if !state.settings_open {
        return;
    }
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    egui::Window::new("Preferences")
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .default_width(500.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(460.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(
                    RichText::new("Appearance")
                        .size(11.5)
                        .color(tokens.text_dim),
                );
                let dark = session.settings.theme != "light";
                if let Some(index) =
                    design::segmented(ui, tokens, &[("Dark", dark), ("Light", !dark)])
                {
                    let next = if index == 0 { "dark" } else { "light" };
                    let _ = session.run(
                        "app.setSetting",
                        serde_json::json!({"key": "theme", "value": next}),
                    );
                }
            });
            ui.add_space(8.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("Data").size(11.5).color(tokens.text_dim));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Rows per page")
                            .size(12.0)
                            .color(tokens.text_dim),
                    );
                    let mut page_size = session.settings.page_size as i64;
                    if ui
                        .add(egui::DragValue::new(&mut page_size).range(10..=100_000))
                        .changed()
                    {
                        let _ = session.run(
                            "app.setSetting",
                            serde_json::json!({"key": "page_size", "value": page_size}),
                        );
                    }
                    ui.label(
                        RichText::new("Row limit per query")
                            .size(12.0)
                            .color(tokens.text_dim),
                    );
                    let mut max_rows = session.settings.max_rows as i64;
                    if ui
                        .add(egui::DragValue::new(&mut max_rows).range(100..=5_000_000))
                        .changed()
                    {
                        let _ = session.run(
                            "app.setSetting",
                            serde_json::json!({"key": "max_rows", "value": max_rows}),
                        );
                    }
                });
            });
            ui.add_space(8.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("Files").size(11.5).color(tokens.text_dim));
                for (label, path) in [
                    (
                        "profiles",
                        datacraft_engine::profiles::profiles_path()
                            .display()
                            .to_string(),
                    ),
                    (
                        "jobs",
                        datacraft_engine::profiles::jobs_path()
                            .display()
                            .to_string(),
                    ),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).size(11.5).color(tokens.text_faint));
                        ui.label(
                            RichText::new(path)
                                .monospace()
                                .size(11.0)
                                .color(tokens.text_dim),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("control key")
                            .size(11.5)
                            .color(tokens.text_faint),
                    );
                    ui.label(
                        RichText::new(
                            state
                                .control
                                .as_ref()
                                .map(|c| c.key_path().display().to_string())
                                .unwrap_or_else(|| "not serving".into()),
                        )
                        .monospace()
                        .size(11.0)
                        .color(tokens.text_dim),
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Close", true).clicked() {
                    close = true;
                }
            });
        });
    if close || !open {
        state.settings_open = false;
    }
}

/// Cmd+K palette: type to filter every command by id or label, Enter runs the first match.
fn palette(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    if !state.palette_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let frame = dialog(&ctx, tokens);
    let mut open = true;
    let mut close = false;
    let mut run: Option<&'static str> = None;
    let screen = ctx.viewport_rect();
    egui::Window::new("Command palette")
        .open(&mut open)
        .frame(frame)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_pos(egui::pos2(screen.center().x - 280.0, screen.top() + 90.0))
        .default_width(560.0)
        .show(&ctx, |ui| {
            ui.set_min_width(520.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.palette_query)
                    .hint_text("search a command by id or label…")
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(8, 4)),
            );
            response.request_focus();
            ui.add_space(4.0);
            let needle = state.palette_query.to_ascii_lowercase();
            let matches: Vec<&'static datacraft_engine::CommandSpec> = datacraft_engine::registry()
                .iter()
                .filter(|spec| {
                    needle.is_empty()
                        || spec.id.to_ascii_lowercase().contains(&needle)
                        || spec.label.to_ascii_lowercase().contains(&needle)
                        || spec.group.to_ascii_lowercase().contains(&needle)
                })
                .collect();
            let entered = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if entered {
                if let Some(first) = matches.first() {
                    run = Some(first.id);
                }
            }
            egui::ScrollArea::vertical()
                .id_salt("palette-list")
                .max_height(360.0)
                .show(ui, |ui| {
                    let mut group = "";
                    for spec in matches.iter().take(60) {
                        if spec.group != group {
                            group = spec.group;
                            ui.add_space(2.0);
                            ui.label(RichText::new(group).size(10.5).color(tokens.text_faint));
                        }
                        let response = ui.add(
                            egui::Button::new(
                                RichText::new(format!("{}  ·  {}", spec.id, spec.label))
                                    .size(12.0)
                                    .color(tokens.text),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .min_size(egui::Vec2::new(ui.available_width(), 22.0)),
                        );
                        if response.hovered() {
                            let rect = response.rect;
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(4),
                                tokens.panel_high,
                            );
                        }
                        if response.clicked() {
                            run = Some(spec.id);
                        }
                    }
                    if matches.is_empty() {
                        ui.label(
                            RichText::new("no command matches")
                                .size(12.0)
                                .color(tokens.text_faint),
                        );
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                design::keycap(ui, tokens, "Enter");
                ui.label(
                    RichText::new("run the first match")
                        .size(11.0)
                        .color(tokens.text_faint),
                );
                design::chip(
                    ui,
                    tokens,
                    &format!("{} matches", matches.len()),
                    tokens.text_dim,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    design::keycap(ui, tokens, "Esc");
                });
            });
        });
    if let Some(id) = run {
        state.open_command(id.to_string());
        close = true;
    }
    if close || !open {
        state.palette_open = false;
    }
    let _ = session;
}
