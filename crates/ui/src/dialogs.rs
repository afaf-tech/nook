//! Modal windows: connection editor, command runner, data generation, export, settings.

use datacraft_core::{DriverKind, SslMode};
use datacraft_engine::{Level, Session};
use egui::RichText;

use crate::app::UiState;
use crate::theme::Tokens;

pub fn show(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    connection_editor(ui, session, state, tokens);
    command_runner(ui, session, state, tokens);
    datagen_dialog(ui, session, state, tokens);
    export_dialog(ui, session, state, tokens);
    settings_dialog(ui, session, state, tokens);
    palette(ui, session, state, tokens);
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
    let mut open = true;
    let mut close = false;
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .show(ui.ctx(), |ui| {
            let form = &mut state.connection_form;
            egui::Grid::new("connection-form")
                .min_col_width(110.0)
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut form.name);
                    ui.end_row();

                    ui.label("Engine");
                    egui::ComboBox::from_id_salt("connection-kind")
                        .selected_text(form.kind.label())
                        .show_ui(ui, |ui| {
                            for kind in [
                                DriverKind::Sqlite,
                                DriverKind::Postgres,
                                DriverKind::MySql,
                                DriverKind::MariaDb,
                            ] {
                                ui.selectable_value(&mut form.kind, kind, kind.label());
                            }
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
                    ui.end_row();

                    if form.kind.is_file_based() {
                        ui.label("File");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut form.file);
                            if ui.button("…").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title("SQLite database")
                                    .save_file()
                                {
                                    form.file = path.to_string_lossy().into_owned();
                                }
                            }
                        });
                        ui.end_row();
                    } else {
                        ui.label("Host");
                        ui.text_edit_singleline(&mut form.host);
                        ui.end_row();
                        ui.label("Port");
                        let mut port = form.port as i32;
                        if ui
                            .add(egui::DragValue::new(&mut port).range(1..=65535))
                            .changed()
                        {
                            form.port = port.clamp(1, 65535) as u16;
                        }
                        ui.end_row();
                        ui.label("User");
                        ui.text_edit_singleline(&mut form.user);
                        ui.end_row();
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                        ui.end_row();
                        ui.label("Database");
                        ui.text_edit_singleline(&mut form.database);
                        ui.end_row();
                        ui.label("SSL");
                        egui::ComboBox::from_id_salt("connection-ssl")
                            .selected_text(form.ssl.label())
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
                        ui.end_row();
                    }

                    ui.label("Group");
                    ui.text_edit_singleline(&mut form.group);
                    ui.end_row();

                    ui.label("SSH tunnel");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut form.ssh_enabled, "via jump host");
                        ui.add(
                            egui::TextEdit::singleline(&mut form.ssh_host)
                                .hint_text("jump.example.com")
                                .desired_width(160.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut form.ssh_user)
                                .hint_text("user")
                                .desired_width(90.0),
                        );
                        let mut ssh_port = form.ssh_port as i32;
                        if ui
                            .add(egui::DragValue::new(&mut ssh_port).range(1..=65535))
                            .changed()
                        {
                            form.ssh_port = ssh_port.clamp(1, 65535) as u16;
                        }
                    });
                    ui.end_row();
                    ui.label("SSH key");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.ssh_key)
                            .hint_text("~/.ssh/id_ed25519 (empty: agent and config)")
                            .desired_width(320.0),
                    );
                    ui.end_row();

                    ui.label("Options");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut form.read_only, "read-only");
                        ui.checkbox(&mut form.keychain, "store password in the keychain");
                    });
                    ui.end_row();
                });

            if !form.kind.implemented() {
                ui.colored_label(
                    tokens.warn,
                    format!(
                        "{} is catalogued for the parity checklist but has no driver yet.",
                        form.kind.label()
                    ),
                );
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Save").strong()).clicked() {
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
                    if ui.button("Connect").clicked() {
                        if let Err(e) = datacraft_engine::cmd::open_now(session, index) {
                            session.log(Level::Warn, e.to_string());
                        }
                    }
                    if ui
                        .button("Delete")
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
                    if ui.button("Test").clicked() {
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
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
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
    let mut open = true;
    let mut close = false;
    egui::Window::new("Run a command")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .show(ui.ctx(), |ui| {
            ui.label(
                RichText::new(
                    "Every action in DataCraft is a command with an id and JSON parameters.",
                )
                .size(11.0)
                .color(tokens.text_dim),
            );
            ui.horizontal(|ui| {
                ui.label("Command");
                egui::ComboBox::from_id_salt("command-id")
                    .selected_text(state.command_id.clone())
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        let mut group = "";
                        for spec in datacraft_engine::registry() {
                            if spec.group != group {
                                group = spec.group;
                                ui.label(RichText::new(group).strong().size(11.0));
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
            });
            if let Some(spec) = datacraft_engine::find(&state.command_id) {
                if !spec.help.is_empty() {
                    ui.label(RichText::new(spec.help).size(11.0).color(tokens.text_dim));
                }
            }
            ui.label("Parameters (JSON)");
            ui.add(
                egui::TextEdit::multiline(&mut state.command_params)
                    .code_editor()
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Run").strong()).clicked() {
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
                if ui.button("Insert the connection id").clicked() {
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
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
            if let Some(result) = &state.command_result {
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("command-result")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new(result).monospace().size(11.0));
                    });
            }
        });
    if close || !open {
        state.command_open = false;
    }
}

fn datagen_dialog(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let Some((conn, table)) = state.datagen_target.clone() else {
        return;
    };
    let mut open = true;
    let mut close = false;
    // The form is edited on a copy: the closures below also borrow the rest of the state.
    let mut form = state.datagen_form.clone();
    let mut preview: Option<String> = None;
    egui::Window::new(format!("Generate data · {}", table.key()))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Rows");
                let mut rows = form.rows as i64;
                if ui
                    .add(egui::DragValue::new(&mut rows).range(1..=100_000))
                    .changed()
                {
                    form.rows = rows.clamp(1, 100_000) as usize;
                }
                ui.label("Seed");
                let mut seed = form.seed as i64;
                if ui
                    .add(egui::DragValue::new(&mut seed).range(1..=1_000_000))
                    .changed()
                {
                    form.seed = seed.max(1) as u64;
                }
                ui.label(
                    RichText::new("deterministic: the same seed gives the same rows")
                        .size(11.0)
                        .color(tokens.text_dim),
                );
            });
            ui.separator();
            ui.label(RichText::new("Per column").strong());
            let columns = session
                .conns
                .get(conn)
                .and_then(|c| c.catalog.as_ref())
                .and_then(|cat| cat.find_table(table.schema.as_deref(), &table.table))
                .map(|t| t.columns.clone())
                .unwrap_or_default();
            for column in &columns {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(column.name.clone()).monospace().size(12.0));
                    ui.label(
                        RichText::new(column.type_name.clone())
                            .size(11.0)
                            .color(tokens.text_dim),
                    );
                    let spec = form
                        .overrides
                        .entry(column.name.clone())
                        .or_insert_with(|| "auto".to_string());
                    ui.add(
                        egui::TextEdit::singleline(spec)
                            .desired_width(240.0)
                            .hint_text(
                                "auto|int:1-100|pick:a|b|pattern:INV-####|email|sentence|uuid|null",
                            ),
                    );
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Preview").clicked() {
                    let mut params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "table": table.key(),
                        "rows": form.rows,
                        "seed": form.seed,
                    });
                    params["overrides"] = serde_json::json!(form.overrides);
                    match session.run("tools.datagen", params) {
                        Ok(value) => {
                            preview =
                                Some(serde_json::to_string_pretty(&value).unwrap_or_default());
                        }
                        Err(e) => session.log(Level::Error, e.to_string()),
                    }
                }
                if ui.button(RichText::new("Insert").strong()).clicked() {
                    let mut params = serde_json::json!({
                        "conn": session.conns[conn].cfg.id,
                        "table": table.key(),
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
                if ui.button("Write .sql…").clicked() {
                    if let Some(path) = state.pick_save_path(&format!("{}.sql", table.table)) {
                        let mut params = serde_json::json!({
                            "conn": session.conns[conn].cfg.id,
                            "table": table.key(),
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
                if ui.button("Close").clicked() {
                    close = true;
                }
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
    let mut open = true;
    let mut close = false;
    let table = state.export_table.clone();
    let format = state
        .export_grid_format
        .clone()
        .unwrap_or_else(|| "sql".into());
    egui::Window::new("Export")
        .open(&mut open)
        .collapsible(false)
        .default_width(480.0)
        .show(ui.ctx(), |ui| {
            ui.label(
                RichText::new(match &table {
                    Some(t) => format!("Table {}", t.key()),
                    None => "The visible grid page".into(),
                })
                .strong(),
            );
            ui.horizontal(|ui| {
                ui.label("Format");
                for candidate in ["csv", "json", "sql", "markdown"] {
                    let selected = state.export_format == candidate;
                    if ui.selectable_label(selected, candidate).clicked() {
                        state.export_format = candidate.to_string();
                    }
                }
            });
            ui.label(
                RichText::new(format!(
                    "This dialog was opened for {format}; it exports {}",
                    state.export_format
                ))
                .size(11.0)
                .color(tokens.text_dim),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Choose file and export").strong())
                    .clicked()
                {
                    let default = match &table {
                        Some(t) => format!("{}.{}", t.table, extension(&state.export_format)),
                        None => format!("grid.{}", extension(&state.export_format)),
                    };
                    if let Some(path) = state.pick_save_path(&default) {
                        let result = match (&table, is_grid) {
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
                if ui.button("Import a file…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("CSV or JSON to import")
                        .add_filter("data", &["csv", "json", "tsv", "txt"])
                        .pick_file()
                    {
                        let target = table
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
                if ui.button("Cancel").clicked() {
                    close = true;
                }
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
    let mut open = true;
    let mut close = false;
    egui::Window::new("Preferences")
        .open(&mut open)
        .collapsible(false)
        .default_width(460.0)
        .show(ui.ctx(), |ui| {
            let mut theme = session.settings.theme.clone();
            ui.horizontal(|ui| {
                ui.label("Theme");
                for candidate in ["dark", "light"] {
                    if ui.selectable_label(theme == candidate, candidate).clicked() {
                        theme = candidate.to_string();
                        let _ = session.run(
                            "app.setSetting",
                            serde_json::json!({"key": "theme", "value": candidate}),
                        );
                    }
                }
            });
            let mut page_size = session.settings.page_size as i64;
            ui.horizontal(|ui| {
                ui.label("Rows per page");
                if ui
                    .add(egui::DragValue::new(&mut page_size).range(10..=100_000))
                    .changed()
                {
                    let _ = session.run(
                        "app.setSetting",
                        serde_json::json!({"key": "page_size", "value": page_size}),
                    );
                }
            });
            let mut max_rows = session.settings.max_rows as i64;
            ui.horizontal(|ui| {
                ui.label("Row limit per query");
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
            ui.separator();
            ui.label(RichText::new("Files").strong());
            ui.label(
                RichText::new(format!(
                    "profiles: {}\njobs: {}\ncontrol key: {}",
                    datacraft_engine::profiles::profiles_path().display(),
                    datacraft_engine::profiles::jobs_path().display(),
                    state
                        .control
                        .as_ref()
                        .map(|c| c.key_path().display().to_string())
                        .unwrap_or_else(|| "not serving".into())
                ))
                .monospace()
                .size(11.0)
                .color(tokens.text_dim),
            );
            ui.separator();
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    if close || !open {
        state.settings_open = false;
    }
}

/// Cmd+K palette: type to filter every command by id or label.
fn palette(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    if !state.palette_open {
        return;
    }
    let mut open = true;
    let mut close = false;
    egui::Window::new("Command palette")
        .open(&mut open)
        .collapsible(false)
        .default_width(520.0)
        .show(ui.ctx(), |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.palette_query)
                    .hint_text("type a command id or label")
                    .desired_width(f32::INFINITY),
            );
            response.request_focus();
            let needle = state.palette_query.to_ascii_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("palette-list")
                .max_height(360.0)
                .show(ui, |ui| {
                    let mut shown = 0;
                    for spec in datacraft_engine::registry() {
                        if !needle.is_empty()
                            && !spec.id.to_ascii_lowercase().contains(&needle)
                            && !spec.label.to_ascii_lowercase().contains(&needle)
                        {
                            continue;
                        }
                        shown += 1;
                        if shown > 60 {
                            break;
                        }
                        let label = format!("{}  ·  {}", spec.id, spec.label);
                        if ui.selectable_label(false, label).clicked() {
                            state.open_command(spec.id.to_string());
                            close = true;
                        }
                    }
                    if shown == 0 {
                        ui.label(RichText::new("no command matches").color(tokens.text_dim));
                    }
                });
        });
    if close || !open {
        state.palette_open = false;
    }
    let _ = session;
}
