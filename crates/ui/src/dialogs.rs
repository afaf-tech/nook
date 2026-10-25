//! Modal windows: the connection editor and its confirmation on the design's own modal shell, and the
//! command runner, data generation, export, preferences and palette still on their `egui::Window`
//! frame. The redesign rebuilt the connection editor and nothing else, so two chromes stand side by
//! side until the next slice moves the rest.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind};
use nook_core::{DriverKind, SslMode};
use nook_engine::{Level, Session};

use crate::app::{Confirm, ConnStep, ConnectionForm, SshAuth, TestOutcome, UiState};
use crate::strings::tr;
use crate::table;
use crate::theme::{self as design, Tokens};

/// The frame the command runner, data generation, export, preferences and the palette still use. The
/// connection editor and its confirmation use [`dialog_shell`] instead — the design's own `.dlg`, with
/// the scrim, the 6px radius and its `--s7`/`--s6` bands.
fn dialog(ctx: &egui::Context, tokens: &Tokens) -> egui::Frame {
    egui::Frame::window(&ctx.style_of(ctx.theme()))
        .fill(tokens.panel)
        .stroke(Stroke::new(1.0, tokens.stroke_strong))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(14))
}

pub fn show(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    connection_editor(ui, session, state, tokens);
    confirmation(ui, session, state, tokens);
    command_runner(ui, session, state, tokens);
    datagen_dialog(ui, session, state, tokens);
    import_wizard_dialog(ui, session, state, tokens);
    export_dialog(ui, session, state, tokens);
    settings_dialog(ui, session, state, tokens);
    safe_prompt(ui, session, state, tokens);
    palette(ui, session, state, tokens);
}

// ------------------------------------------------------------------ the connection editor

/// `.dlg-b`'s content width: the dialog less its `--s7` padding either side.
const BODY_W: f32 = design::DIALOG_W - 2.0 * design::S7 as f32;
/// The confirmation's, at the design's narrow width.
const BODY_W_NARROW: f32 = design::DIALOG_NARROW_W - 2.0 * design::S7 as f32;
/// `.row`'s and `.dlg-b`'s gap.
const ROW_GAP: f32 = design::S5 as f32;
/// `.badge`'s box.
const BADGE: f32 = 14.0;

/// The engines the editor offers: the four with a driver, then the catalogued ones the registry knows
/// and no driver opens yet.
const ENGINES: [DriverKind; 8] = [
    DriverKind::Sqlite,
    DriverKind::Postgres,
    DriverKind::MySql,
    DriverKind::MariaDb,
    DriverKind::SqlServer,
    DriverKind::Oracle,
    DriverKind::Mongo,
    DriverKind::Redis,
];

/// The TLS step's honest line. The design's sentence promises a CA certificate file, which this app
/// has no field for and no driver reads: `verify-ca`/`verify-full` check against the OS trust store.
const TLS_HELPER: &str = "verify-ca and verify-full check the chain and the host name against this \
system's trust store on every connect. The design's certificate paths have no field in this version.";

/// What the footer's buttons asked the frame to do.
#[derive(Default)]
struct EditorAction {
    test: bool,
    save: bool,
    close: bool,
}

/// `.dlg`: one centred modal — the whole window under `tokens.scrim`, on the design's frame at the
/// design's width. `should_close()` is the backdrop click and Esc, so the × and both keys end the
/// dialog the same way.
fn dialog_shell(
    ctx: &egui::Context,
    tokens: &Tokens,
    id: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::ModalResponse<()> {
    egui::Modal::new(egui::Id::new(("nook-dialog", id)))
        .backdrop_color(tokens.scrim)
        .frame(design::dialog_frame(tokens))
        .show(ctx, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            add(ui);
        })
}

/// `.dlg-b`: the step's body — `--s7` each side, `--s6` above and below, the step's children `--s5`
/// apart. The band is content-sized, so every step's dialog is as tall as its own fields.
fn dialogue_body(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(design::S7, design::S6))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            add(ui);
        });
}

/// `.row`: the step's fields `--s5` apart.
fn conn_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = ROW_GAP;
        add(ui);
    });
}

/// `.fld`: the `--s6` label box, the `--s3` gap the caller's stack spends, and the 28px control, in a
/// column of exactly `width` — which is what makes a field the 50 the design's arithmetic names.
fn field_col<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = f32::from(design::S3);
            add(ui)
        },
    )
    .inner
}

/// The text inside an `.inp`: egui's own editor with no chrome of its own, so the box's fill, border
/// and `--s4` padding are the only ones painted, and the placeholder is `text_faint` as the design has
/// it — egui's default hint ink is not.
fn input_text(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    value: &mut String,
    width: f32,
    hint: &str,
    secret: bool,
) -> egui::Response {
    ui.visuals_mut().weak_text_color = Some(tokens.text_faint);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(width)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(tokens.font(design::T_MD))
            .text_color(tokens.text)
            .password(secret)
            .hint_text(hint.to_owned()),
    )
}

/// One `.inp` text field, its label and all.
#[allow(clippy::too_many_arguments)]
fn text_field(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    width: f32,
    value: &mut String,
    hint: &str,
    error: bool,
    enabled: bool,
) -> egui::Response {
    field_col(ui, width, |ui| {
        design::field_label(ui, tokens, label);
        design::input(ui, tokens, width, error, enabled, |ui| {
            input_text(
                ui,
                tokens,
                value,
                width - 2.0 * f32::from(design::S4),
                hint,
                false,
            )
        })
    })
}

/// A password `.inp`, with the design's `keychain` chip inside its right edge when the password is
/// kept in the keychain. The chip takes its width first and the text gets what is left, which is what
/// the design's `margin-left:auto` does.
#[allow(clippy::too_many_arguments)]
fn password_field(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    width: f32,
    value: &mut String,
    hint: &str,
    error: bool,
    enabled: bool,
    chip: bool,
) -> egui::Response {
    field_col(ui, width, |ui| {
        design::field_label(ui, tokens, label);
        design::input(ui, tokens, width, error, enabled, |ui| {
            let text_width = width - 2.0 * f32::from(design::S4);
            if chip {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    design::tag_of(ui, tokens, "keychain", design::KEYCHIP_H);
                    let room = ui.available_width();
                    input_text(ui, tokens, value, room, hint, true)
                })
                .inner
            } else {
                input_text(ui, tokens, value, text_width, hint, true)
            }
        })
    })
}

/// A numeric `.inp` with an optional trailing unit, which is how the design draws Timeout's `s` and
/// the local port's `auto`.
#[allow(clippy::too_many_arguments)]
fn number_field(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    width: f32,
    value: &mut u16,
    max: u16,
    suffix: &str,
    enabled: bool,
) -> egui::Response {
    field_col(ui, width, |ui| {
        design::field_label(ui, tokens, label);
        design::input(ui, tokens, width, false, enabled, |ui| {
            let mut number = i64::from(*value);
            let response = ui.add(
                egui::DragValue::new(&mut number)
                    .range(0..=i64::from(max))
                    .speed(1.0),
            );
            if response.changed() {
                *value = number.clamp(0, i64::from(max)) as u16;
            }
            if !suffix.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(suffix)
                            .size(design::T_MD)
                            .color(tokens.text_dim),
                    );
                });
            }
            response
        })
    })
}

/// The design's `.inp.ui-font` with its chevron: a combo wearing the token layer's own control fill,
/// stroke and radius, so it is the design's 28px control rather than a second visual language.
#[allow(clippy::too_many_arguments)]
fn select_field<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    width: f32,
    id: &str,
    options: &[(T, String)],
    value: &mut T,
    enabled: bool,
) -> egui::Response {
    field_col(ui, width, |ui| {
        design::field_label(ui, tokens, label);
        ui.add_enabled_ui(enabled, |ui| {
            let selected = options
                .iter()
                .find(|(option, _)| option == value)
                .map(|(_, text)| text.clone())
                .unwrap_or_default();
            egui::ComboBox::from_id_salt(id)
                .selected_text(selected)
                .width(width)
                .show_ui(ui, |ui| {
                    for (option, text) in options {
                        ui.selectable_value(value, option.clone(), text);
                    }
                })
                .response
        })
        .inner
    })
}

/// `.msg`: the design's badge and its sentence, in the state's own ink.
fn msg_line(ui: &mut egui::Ui, tokens: &Tokens, ink: Color32, mark: &str, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = f32::from(design::S3);
        let (badge, _) = ui.allocate_exact_size(egui::vec2(BADGE, BADGE), Sense::hover());
        let painter = ui.painter();
        painter.rect_stroke(
            badge,
            CornerRadius::same(2),
            Stroke::new(1.0, ink),
            StrokeKind::Inside,
        );
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            mark,
            tokens.mono(9.0),
            ink,
        );
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .size(design::T_SM)
                    .color(ink)
                    .line_height(Some(design::T_SM * 1.45)),
            )
            .wrap(),
        );
    });
}

/// `.para`: the confirmation's one explanatory sentence.
fn para(ui: &mut egui::Ui, tokens: &Tokens, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(design::T_MD)
                .color(tokens.text_dim)
                .line_height(Some(design::T_MD * 1.5)),
        )
        .wrap(),
    );
}

/// `.helper` for the database password: the design's sentence where there is a keychain, and the
/// truth about where the password goes where there is not.
fn password_helper(keychain: bool) -> &'static str {
    if keychain {
        "The password stays in the login keychain; the profile file keeps only a reference to it."
    } else {
        "This system has no OS keychain, so the password is kept in the profile file (mode 0600)."
    }
}

/// `.helper` for the tunnel: one sentence owned here, per authentication mode, and the engine's own
/// refusal where there is no keychain to promise.
fn ssh_helper(keychain: bool, auth: SshAuth) -> &'static str {
    if !keychain {
        return nook_engine::profiles::NO_KEYCHAIN;
    }
    match auth {
        SshAuth::Password => {
            "The jump host signs in with this password instead of a key: the tunnel feeds it to ssh \
through an askpass helper, and the profile file keeps only a reference. Unticked, you are asked on \
every connect."
        }
        SshAuth::KeyFile => "The tunnel signs in with this key file; the profile keeps its path.",
        SshAuth::Agent => {
            "The tunnel signs in through your ssh agent and ~/.ssh/config; nothing is stored here."
        }
    }
}

/// The editor's `.dlg-sub`: the connection and its engine.
fn connection_subtitle(state: &UiState) -> String {
    let form = &state.connection_form;
    if state.connection_editing.is_some() {
        format!("{} · {}", form.name, form.kind.label())
    } else {
        format!("New profile · {}", form.kind.label())
    }
}

/// The window this slice exists for: the design's `.dlg` with its four steps, its pinned test row and
/// its two-row action band.
fn connection_editor(
    ui: &mut egui::Ui,
    session: &mut Session,
    state: &mut UiState,
    tokens: &Tokens,
) {
    if !state.connection_open || state.confirm.is_some() {
        return;
    }
    // The form is rebuilt once per open: an edit dialog that kept the previous form would show — and
    // save — another connection's fields.
    let key = Some(state.connection_editing);
    if state.connection_form_id != key {
        state.connection_form = match state
            .connection_editing
            .and_then(|index| session.conns.get(index))
        {
            Some(conn) => ConnectionForm::from_config(&conn.cfg),
            None => ConnectionForm::default(),
        };
        state.connection_form_id = key;
        state.test_state = None;
    }

    let mut action = EditorAction::default();
    let response = dialog_shell(
        ui.ctx(),
        tokens,
        "connection-editor",
        design::DIALOG_W,
        |ui| {
            let title = if state.connection_editing.is_some() {
                "Edit connection"
            } else {
                "New connection"
            };
            let subtitle = connection_subtitle(state);
            if design::dialogue_head(ui, tokens, None, title, &subtitle, false).clicked() {
                action.close = true;
            }
            let labels: Vec<&str> = ConnStep::ALL.iter().map(|step| step.label()).collect();
            let active = ConnStep::ALL
                .iter()
                .position(|step| *step == state.connection_form.step)
                .unwrap_or(0);
            if let Some(clicked) = design::dialogue_tabs(ui, tokens, &labels, active) {
                state.connection_form.step = ConnStep::ALL[clicked];
            }
            dialogue_body(ui, |ui| match state.connection_form.step {
                ConnStep::General => general_step(ui, session, state, tokens),
                ConnStep::Tls => tls_step(ui, tokens, state),
                ConnStep::Tunnel => tunnel_step(ui, tokens, state),
                ConnStep::Advanced => advanced_step(ui, session, state, tokens),
            });
            editor_footer(ui, tokens, state, &mut action);
        },
    );

    if action.test {
        test_connection(session, state);
    }
    if action.save {
        match save_connection(session, state) {
            Ok(()) => {
                session.log(
                    Level::Success,
                    format!("connection '{}' saved", state.connection_form.name.trim()),
                );
                action.close = true;
            }
            Err(error) => {
                state.test_state = Some(TestOutcome {
                    ok: false,
                    line: format!("failed · {error}"),
                });
                session.log(Level::Error, error.to_string());
            }
        }
    }
    if action.close || response.should_close() {
        state.connection_open = false;
        state.connection_form_id = None;
        state.test_state = None;
    }
}

/// The design's General step: the profile's own fields, in the design's rows and widths.
fn general_step(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let editing = state.connection_editing.is_some();
    let file_based = state.connection_form.kind.is_file_based();
    let keychain = state.connection_form.keychain && nook_engine::profiles::keychain_available();
    let form = &mut state.connection_form;

    // Connection name (`.fld.grow`) and the engine at the design's 180.
    let name_w = BODY_W - ROW_GAP - 180.0;
    conn_row(ui, |ui| {
        text_field(
            ui,
            tokens,
            "Connection name",
            name_w,
            &mut form.name,
            "shop production",
            false,
            true,
        );
        let engines: Vec<(DriverKind, String)> = ENGINES
            .iter()
            .map(|kind| {
                let text = if kind.implemented() {
                    kind.label().to_string()
                } else {
                    format!("{} (catalogued, no driver)", kind.label())
                };
                (*kind, text)
            })
            .collect();
        let engine = select_field(
            ui,
            tokens,
            "Engine",
            180.0,
            "connection-kind",
            &engines,
            &mut form.kind,
            !editing,
        );
        if editing {
            engine.on_hover_text(
                "the engine is part of the profile's identity; create a new connection to change it",
            );
        }
    });

    if file_based {
        // A SQLite profile's target: the design draws host/port/database because its frame is
        // PostgreSQL, so the file takes that row.
        let file_w = BODY_W - ROW_GAP - 28.0;
        conn_row(ui, |ui| {
            let file = text_field(
                ui,
                tokens,
                "File",
                file_w,
                &mut form.file,
                "/path/to/file.db",
                false,
                !editing,
            );
            if editing {
                file.on_hover_text("the file is the profile's identity once saved");
            }
            field_col(ui, 28.0, |ui| {
                ui.add_space(f32::from(design::S6) + f32::from(design::S3));
                if design::ghost_button(ui, tokens, "…", !editing).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("SQLite database")
                        .save_file()
                    {
                        form.file = path.to_string_lossy().into_owned();
                    }
                }
            });
        });
    } else {
        let grow = (BODY_W - 2.0 * ROW_GAP - 110.0) / 2.0;
        conn_row(ui, |ui| {
            text_field(
                ui,
                tokens,
                "Host",
                grow,
                &mut form.host,
                "db.internal",
                false,
                true,
            );
            number_field(ui, tokens, "Port", 110.0, &mut form.port, 65535, "", true);
            text_field(
                ui,
                tokens,
                "Database",
                grow,
                &mut form.database,
                "analytics",
                false,
                true,
            );
        });
        let half = (BODY_W - ROW_GAP) / 2.0;
        conn_row(ui, |ui| {
            text_field(
                ui,
                tokens,
                "User",
                half,
                &mut form.user,
                "analytics_ro",
                false,
                true,
            );
            let password = password_field(
                ui,
                tokens,
                "Password",
                half,
                &mut form.password,
                "••••••••",
                false,
                true,
                keychain,
            );
            if !keychain {
                password.on_hover_text(password_helper(keychain));
            }
        });
        design::helper_line(ui, tokens, password_helper(keychain));
    }

    // Group (`.fld.grow`) and Timeout at the design's 120. The design's Colour picker is the fourth
    // control of this row and is not shipped: nothing in the app paints `cfg.color` yet, so a picker
    // here would change nothing (recorded in `docs/architecture.md`).
    let group_w = BODY_W - ROW_GAP - 120.0;
    let mut groups: Vec<String> = session
        .conns
        .iter()
        .map(|conn| conn.cfg.group.clone())
        .filter(|group| !group.is_empty())
        .collect();
    groups.sort();
    groups.dedup();
    if !form.group.trim().is_empty() && !groups.contains(&form.group) {
        groups.push(form.group.clone());
    }
    let mut options: Vec<(String, String)> = vec![(String::new(), "No group".to_string())];
    options.extend(groups.into_iter().map(|group| (group.clone(), group)));
    conn_row(ui, |ui| {
        select_field(
            ui,
            tokens,
            "Group",
            group_w,
            "connection-group",
            &options,
            &mut form.group,
            true,
        );
        number_field(
            ui,
            tokens,
            "Timeout",
            120.0,
            &mut form.timeout,
            300,
            "s",
            true,
        );
    });

    design::checkbox(
        ui,
        tokens,
        "Read-only — never write through this connection",
        &mut form.read_only,
        true,
    );
}

/// The design's TLS step: the five modes and the truth about what this app verifies. The design's
/// three certificate paths are a ceiling, not a task — `ConnectionConfig` has no field for them and
/// the drivers verify against the OS trust store.
fn tls_step(ui: &mut egui::Ui, tokens: &Tokens, state: &mut UiState) {
    let form = &mut state.connection_form;
    field_col(ui, BODY_W, |ui| {
        design::field_label(ui, tokens, "SSL mode");
        let options: Vec<(&str, bool)> = SslMode::ALL
            .iter()
            .map(|mode| (mode.label(), *mode == form.ssl))
            .collect();
        if let Some(clicked) = design::dialogue_segmented(ui, tokens, &options) {
            form.ssl = SslMode::ALL[clicked];
        }
    });
    design::helper_line(ui, tokens, TLS_HELPER);
}

/// The design's Tunnel step: the jump host, the authentication control, the secret and the local
/// port. Where there is no keychain the password field and its toggle wear the engine's own refusal.
fn tunnel_step(ui: &mut egui::Ui, tokens: &Tokens, state: &mut UiState) {
    let keychain = nook_engine::profiles::keychain_available();
    let form = &mut state.connection_form;
    let auth = form.ssh_auth();

    let host_w = BODY_W - 2.0 * ROW_GAP - 110.0 - 150.0;
    conn_row(ui, |ui| {
        text_field(
            ui,
            tokens,
            "SSH host",
            host_w,
            &mut form.ssh_host,
            "jump.example.com",
            false,
            true,
        );
        number_field(
            ui,
            tokens,
            "Port",
            110.0,
            &mut form.ssh_port,
            65535,
            "",
            true,
        );
        text_field(
            ui,
            tokens,
            "SSH user",
            150.0,
            &mut form.ssh_user,
            "ops",
            false,
            true,
        );
    });

    field_col(ui, BODY_W, |ui| {
        design::field_label(ui, tokens, "Authentication");
        let options: Vec<(&str, bool)> = SshAuth::ALL
            .iter()
            .map(|mode| (mode.label(), *mode == auth))
            .collect();
        if let Some(clicked) = design::dialogue_segmented(ui, tokens, &options) {
            form.ssh_auth = SshAuth::ALL[clicked];
        }
    });

    let secret_w = BODY_W - ROW_GAP - 150.0;
    conn_row(ui, |ui| {
        match auth {
            SshAuth::KeyFile => {
                text_field(
                    ui,
                    tokens,
                    "Key file",
                    secret_w,
                    &mut form.ssh_key,
                    "~/.ssh/id_ed25519",
                    false,
                    true,
                );
            }
            SshAuth::Agent => {
                field_col(ui, secret_w, |ui| {
                    design::field_label(ui, tokens, "Key");
                    design::input(ui, tokens, secret_w, false, false, |ui| {
                        ui.label(
                            RichText::new("the ssh agent and ~/.ssh/config")
                                .size(design::T_MD)
                                .color(tokens.text_dim),
                        )
                    });
                });
            }
            SshAuth::Password => {
                let password = password_field(
                    ui,
                    tokens,
                    "SSH password",
                    secret_w,
                    &mut form.ssh_password,
                    "••••••••",
                    false,
                    keychain,
                    keychain,
                );
                if !keychain {
                    password.on_hover_text(nook_engine::profiles::NO_KEYCHAIN);
                }
            }
        }
        number_field(
            ui,
            tokens,
            "Local port",
            150.0,
            &mut form.ssh_local_port,
            65535,
            "auto",
            true,
        );
    });

    if auth == SshAuth::Password {
        let toggle = design::checkbox(
            ui,
            tokens,
            "Save the SSH password in the login keychain",
            &mut form.ssh_save_password,
            keychain,
        );
        if !keychain {
            toggle.on_hover_text(nook_engine::profiles::NO_KEYCHAIN);
        }
    }
    design::helper_line(ui, tokens, ssh_helper(keychain, auth));
}

/// The design's Advanced step. The mockup paints the tab and never opens it, so there is no body to
/// build from: it carries the one control the editor cannot do without — the destructive action — and
/// nothing else. A new profile has nothing to delete yet.
fn advanced_step(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let Some(index) = state.connection_editing else {
        design::helper_line(
            ui,
            tokens,
            "These options appear once the connection is saved.",
        );
        return;
    };
    let Some(conn) = session.conns.get(index) else {
        return;
    };
    let (id, name, engine, open, group) = (
        conn.cfg.id.clone(),
        conn.cfg.name.clone(),
        conn.cfg.kind.label(),
        conn.is_open(),
        conn.cfg.group.clone(),
    );
    let mut facts = vec![
        engine.to_string(),
        if open {
            "connected".into()
        } else {
            "not connected".into()
        },
    ];
    if !group.is_empty() {
        facts.push(format!("group {group}"));
    }

    design::helper_line(
        ui,
        tokens,
        "Deleting a connection removes its profile and the passwords saved with it. The database server is not touched.",
    );
    let danger = design::dialogue_button(
        ui,
        tokens,
        "Delete connection",
        design::Emphasis::Danger,
        true,
    );
    if danger.clicked() {
        state.confirm = Some(Confirm {
            command: "conn.delete".to_string(),
            params: serde_json::json!({ "conn": id }),
            title: "Delete connection".to_string(),
            subject: name.clone(),
            sentence: format!(
                "Deleting {name} removes the profile and the passwords saved with it. The database \
server is not touched."
            ),
            facts,
            action: "Delete".to_string(),
            typed: String::new(),
            expected: name,
        });
        // The confirmation takes over the window: one scrim, never two.
        state.connection_open = false;
    }
}

/// The editor's pinned footer: the test row, then the reason, Cancel and Save.
fn editor_footer(ui: &mut egui::Ui, tokens: &Tokens, state: &UiState, action: &mut EditorAction) {
    let blocked = state.connection_form.blocker();
    let editing = state.connection_editing.is_some();
    let outcome = state.test_state.clone();
    design::dialogue_foot(ui, tokens, design::DLG_FOOT_H, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = f32::from(design::S3);
            let (ink, line) = match &outcome {
                Some(outcome) if outcome.ok => (tokens.ok, outcome.line.clone()),
                Some(outcome) => (tokens.error, outcome.line.clone()),
                None => (tokens.text_faint, "not tested yet".to_string()),
            };
            design::status_dot(ui, ink, design::STATUS_DOT);
            ui.label(RichText::new(line).size(design::T_SM).color(ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let retry = matches!(&outcome, Some(outcome) if !outcome.ok);
                let label = if retry { "Retry" } else { "Test connection" };
                let test =
                    design::dialogue_button(ui, tokens, label, design::Emphasis::Ghost, editing);
                if editing {
                    if test.clicked() {
                        action.test = true;
                    }
                } else {
                    test.on_hover_text(
                        "save the connection first — conn.test names a saved profile",
                    );
                }
            });
        });
        ui.horizontal(|ui| {
            let reason = blocked
                .clone()
                .unwrap_or_else(|| "The profile is complete.".to_string());
            ui.label(
                RichText::new(reason)
                    .size(design::T_XS)
                    .color(tokens.text_dim),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = design::dialogue_button(
                    ui,
                    tokens,
                    "Save",
                    design::Emphasis::Primary,
                    blocked.is_none(),
                );
                if let Some(reason) = &blocked {
                    save.clone().on_hover_text(reason.clone());
                }
                if save.clicked() {
                    action.save = true;
                }
                if design::dialogue_button(ui, tokens, tr("Cancel"), design::Emphasis::Plain, true)
                    .clicked()
                {
                    action.close = true;
                }
            });
        });
    });
}

/// Run `conn.test` and keep its facts for the pinned row: a test is a fact about the connection, not a
/// log line, which is why it survives the frame that produced it.
fn test_connection(session: &mut Session, state: &mut UiState) {
    let Some(index) = state.connection_editing else {
        state.test_state = Some(TestOutcome {
            ok: false,
            line: "save the connection first — conn.test names a saved profile".to_string(),
        });
        return;
    };
    let Some(id) = session.conns.get(index).map(|conn| conn.cfg.id.clone()) else {
        return;
    };
    state.test_state = Some(
        match session.run("conn.test", serde_json::json!({ "conn": id })) {
            Ok(value) => TestOutcome {
                ok: true,
                line: test_line(&value),
            },
            Err(error) => TestOutcome {
                ok: false,
                line: format!("failed · {error}"),
            },
        },
    );
}

/// The pinned row's sentence from `conn.test`'s summary: the design states the hop when a tunnel ran,
/// and the server otherwise.
fn test_line(value: &serde_json::Value) -> String {
    let ms = value["elapsed_ms"].as_u64().unwrap_or(0);
    match value.get("tunnel").filter(|tunnel| !tunnel.is_null()) {
        Some(tunnel) => format!(
            "tunnel ok · {}:{} · local {} · {} ms",
            tunnel["host"].as_str().unwrap_or(""),
            tunnel["port"].as_u64().unwrap_or(0),
            tunnel["local_port"].as_u64().unwrap_or(0),
            ms
        ),
        None => format!(
            "ok · {} {} · {} ms",
            value["driver"].as_str().unwrap_or(""),
            value["version"].as_str().unwrap_or(""),
            ms
        ),
    }
}

/// Save the form, addressing every call to the connection it edits. The profile's id is resolved once
/// and then carried by `conn.edit`, `conn.setSshTunnel` and both password commands, so editing
/// connection 1 while connection 0 is active can never write to the active one — which is the defect
/// review 3 found at the old `dialogs.rs:260`.
fn save_connection(session: &mut Session, state: &mut UiState) -> nook_core::Result<()> {
    let form = state.connection_form.clone();
    let editing = state.connection_editing;
    let name = form.name.trim().to_string();
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
        "timeout": form.timeout,
        "color": form.color,
        "keychain": form.keychain,
    });
    if !form.password.is_empty() {
        params["password"] = serde_json::json!(form.password);
    }

    let id = match editing {
        Some(index) => {
            let id = session
                .conns
                .get(index)
                .map(|conn| conn.cfg.id.clone())
                .ok_or_else(|| nook_core::Error::config("the connection is gone"))?;
            params["conn"] = serde_json::json!(id);
            session.run("conn.edit", params)?;
            id
        }
        None => {
            session.run("conn.new", params)?;
            session
                .conns
                .iter()
                .find(|conn| conn.cfg.name == name)
                .map(|conn| conn.cfg.id.clone())
                .ok_or_else(|| nook_core::Error::config("the connection was not saved"))?
        }
    };

    // One addressed `setSshTunnel` with all six fields: a new profile's typed tunnel is written where
    // the dialog says it is instead of being dropped, and an edit's goes to the connection it names.
    session.run(
        "conn.setSshTunnel",
        serde_json::json!({
            "conn": id,
            "enabled": form.ssh_enabled(),
            "host": form.ssh_host,
            "port": form.ssh_port,
            "user": form.ssh_user,
            "key_file": form.ssh_key,
            "local_port": form.ssh_local_port,
        }),
    )?;

    // The database password: the keychain where there is one, the profile file (mode 0600) where
    // there is not — and an empty field leaves whatever is stored alone.
    if !form.password.is_empty() {
        if form.keychain && nook_engine::profiles::keychain_available() {
            session.run(
                "conn.savePassword",
                serde_json::json!({ "conn": id, "password": form.password }),
            )?;
        } else if editing.is_some() {
            session.run(
                "conn.edit",
                serde_json::json!({ "conn": id, "password": form.password }),
            )?;
        }
    }

    // The SSH password lives under its own account, and only while the toggle says so.
    if nook_engine::profiles::keychain_available() {
        if form.ssh_save_password && !form.ssh_password.is_empty() {
            session.run(
                "conn.sshPassword",
                serde_json::json!({ "conn": id, "password": form.ssh_password }),
            )?;
        } else if !form.ssh_save_password {
            let _ = session.run(
                "conn.sshPassword",
                serde_json::json!({ "conn": id, "remove": true }),
            );
        }
    }

    if editing.is_none() {
        // Connect straight away: that is what the user asked for.
        let index = session.conn_index(&id).unwrap_or(0);
        if let Err(error) = nook_engine::cmd::open_now(session, index) {
            session.log(Level::Warn, format!("saved, not connected: {error}"));
        }
    }
    Ok(())
}

/// The design's destructive confirmation: the `.ig` badge, the sentence, the typed name and the one
/// footer row. It is a `Modal` on the same shell, and the editor has closed before it opens, so the
/// scrim is painted once.
fn confirmation(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let Some(mut confirm) = state.confirm.clone() else {
        return;
    };
    let mut cancel = false;
    let mut run = false;
    let response = dialog_shell(
        ui.ctx(),
        tokens,
        "confirmation",
        design::DIALOG_NARROW_W,
        |ui| {
            if design::dialogue_head(
                ui,
                tokens,
                Some("!"),
                &confirm.title,
                &confirm.subject,
                true,
            )
            .clicked()
            {
                cancel = true;
            }
            dialogue_body(ui, |ui| {
                para(ui, tokens, &confirm.sentence);
                // `.facts`: the chips under the sentence, `--s3` apart, wrapped — a dialog whose
                // content has none (the design's own typed-name variant) paints nothing.
                if !confirm.facts.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing =
                            egui::vec2(f32::from(design::S3), f32::from(design::S3));
                        for fact in &confirm.facts {
                            design::tag_of(ui, tokens, fact, design::DLG_CHIP_H);
                        }
                    });
                }
                field_col(ui, BODY_W_NARROW, |ui| {
                    design::field_label(ui, tokens, "Connection name");
                    design::input(
                        ui,
                        tokens,
                        BODY_W_NARROW,
                        !confirm.satisfied(),
                        true,
                        |ui| {
                            input_text(
                                ui,
                                tokens,
                                &mut confirm.typed,
                                BODY_W_NARROW - 2.0 * f32::from(design::S4),
                                &confirm.expected,
                                false,
                            )
                        },
                    );
                });
                msg_line(
                    ui,
                    tokens,
                    if confirm.satisfied() {
                        tokens.ok
                    } else {
                        tokens.error
                    },
                    if confirm.satisfied() { "✓" } else { "!" },
                    &confirm.why(),
                );
            });
            design::dialogue_foot(ui, tokens, design::DLG_FOOT_ONE_H, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(confirm.why())
                            .size(design::T_XS)
                            .color(tokens.text_dim),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if design::dialogue_button(
                            ui,
                            tokens,
                            &confirm.action,
                            design::Emphasis::Danger,
                            confirm.satisfied(),
                        )
                        .clicked()
                        {
                            run = true;
                        }
                        if design::dialogue_button(
                            ui,
                            tokens,
                            "Cancel",
                            design::Emphasis::Plain,
                            true,
                        )
                        .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
            });
        },
    );

    // The typed name is the dialog's own state: it has to survive the frame it was typed in.
    state.confirm = Some(confirm);
    if cancel || response.should_close() {
        state.confirm = None;
        return;
    }
    if run {
        let Some(mut pending) = state.confirm.take() else {
            return;
        };
        if let Some(object) = pending.params.as_object_mut() {
            object.insert("confirm".to_string(), serde_json::json!(true));
        }
        match session.run(&pending.command, pending.params) {
            Ok(_) => {
                session.log(Level::Success, format!("{} removed", pending.subject));
                state.connection_open = false;
                state.connection_editing = None;
                state.connection_form_id = None;
                state.test_state = None;
            }
            Err(error) => {
                session.log(Level::Error, error.to_string());
                state.connection_editing = None;
                state.connection_form_id = None;
            }
        }
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
                RichText::new("Every action in Nook is a command with an id and JSON parameters — this is the same registry the palette, the CLI and MCP use.")
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
                        for spec in nook_engine::registry() {
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
                if let Some(spec) = nook_engine::find(&state.command_id) {
                    design::chip(ui, tokens, spec.group, tokens.info);
                }
            });
            if let Some(spec) = nook_engine::find(&state.command_id) {
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
                if design::primary_button(ui, tokens, tr("Run"), true).clicked() {
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
                    if design::ghost_button(ui, tokens, tr("Close"), true).clicked() {
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
                    if design::ghost_button(ui, tokens, tr("Close"), true).clicked() {
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

/// The import wizard: pick a file and a table, run the dry run, read the mapping, then commit.
///
/// The dry run is the point. `tools.importWizard` writes nothing, so a person can see the inferred
/// types, the columns that would be skipped and the first rows before a 2 GB file lands in the wrong
/// table. The commit sends the same parameters plus `mode` and the edited mapping.
fn import_wizard_dialog(
    ui: &mut egui::Ui,
    session: &mut Session,
    state: &mut UiState,
    tokens: &Tokens,
) {
    let Some(wizard) = state.import_wizard.clone() else {
        return;
    };
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    let mut next = wizard.clone();
    let mut run_dry = false;
    let mut commit = false;
    egui::Window::new("Import")
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .default_width(660.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(620.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("File").size(11.5).color(tokens.text_dim));
                    ui.add(
                        egui::TextEdit::singleline(&mut next.path)
                            .desired_width(300.0)
                            .hint_text("~/data/users.csv"),
                    );
                    if design::ghost_button(ui, tokens, "Browse…", true).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Import")
                            .add_filter("Data", &["csv", "json", "jsonl", "xml", "txt", "xlsx"])
                            .pick_file()
                        {
                            next.path = path.to_string_lossy().into_owned();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Table").size(11.5).color(tokens.text_dim));
                    ui.add(
                        egui::TextEdit::singleline(&mut next.table)
                            .desired_width(240.0)
                            .hint_text("users"),
                    );
                    ui.label(RichText::new("Mode").size(11.5).color(tokens.text_dim));
                    egui::ComboBox::from_id_salt("import-mode")
                        .selected_text(next.mode.clone())
                        .show_ui(ui, |ui| {
                            for mode in ["append", "truncate", "upsert"] {
                                ui.selectable_value(&mut next.mode, mode.to_string(), mode);
                            }
                        });
                });
            });

            if let Some(error) = &next.error {
                ui.add_space(6.0);
                ui.label(RichText::new(error).size(11.5).color(tokens.error));
            }

            if let Some(report) = &next.report {
                ui.add_space(8.0);
                let exists = report["target_exists"].as_bool().unwrap_or(false);
                let sampled = report["rows_sampled"].as_u64().unwrap_or(0);
                design::section(
                    ui,
                    tokens,
                    "Mapping",
                    Some(&format!(
                        "{sampled} rows sampled · {} · {}",
                        report["encoding"].as_str().unwrap_or("utf-8"),
                        if exists {
                            "target exists"
                        } else {
                            "would create the table"
                        }
                    )),
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("import-columns")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        if let Some(columns) = report["columns"].as_array() {
                            for column in columns {
                                let source =
                                    column["source"].as_str().unwrap_or_default().to_string();
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(table::clip(&source, 22))
                                            .monospace()
                                            .size(12.0)
                                            .color(tokens.text),
                                    );
                                    let inferred = column["inferred"].as_str().unwrap_or("text");
                                    design::chip(ui, tokens, inferred, tokens.text_faint);
                                    if column["nullable"].as_bool().unwrap_or(false) {
                                        design::chip(ui, tokens, "nullable", tokens.text_faint);
                                    }
                                    let target =
                                        next.mapping.entry(source.clone()).or_insert_with(|| {
                                            column["target"].as_str().unwrap_or(&source).to_string()
                                        });
                                    ui.add(
                                        egui::TextEdit::singleline(target)
                                            .desired_width(160.0)
                                            .hint_text("target column"),
                                    );
                                    if let Some(sample) =
                                        column["sample"].as_array().and_then(|s| s.first())
                                    {
                                        ui.label(
                                            RichText::new(table::clip(
                                                sample.as_str().unwrap_or(""),
                                                28,
                                            ))
                                            .size(11.0)
                                            .color(tokens.text_faint),
                                        );
                                    }
                                });
                            }
                        }
                    });
                if let Some(preview) = report["preview"].as_array() {
                    ui.add_space(8.0);
                    design::section(ui, tokens, "First rows", None);
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt("import-preview")
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for row in preview.iter().take(20) {
                                let cells: Vec<String> = row
                                    .as_array()
                                    .map(|c| {
                                        c.iter()
                                            .map(|v| v.as_str().unwrap_or_default().to_string())
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                ui.label(
                                    RichText::new(cells.join("  ·  "))
                                        .monospace()
                                        .size(11.5)
                                        .color(tokens.text_dim),
                                );
                            }
                        });
                }
                if let Some(notes) = report["notes"].as_array() {
                    if !notes.is_empty() {
                        ui.add_space(6.0);
                        for note in notes {
                            ui.label(
                                RichText::new(format!("· {}", note.as_str().unwrap_or("")))
                                    .size(11.0)
                                    .color(tokens.text_faint),
                            );
                        }
                    }
                }
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Dry run", true).clicked() {
                    run_dry = true;
                }
                let ready = next.report.is_some()
                    && !next.table.trim().is_empty()
                    && !next.path.trim().is_empty();
                if design::ghost_button(ui, tokens, "Import", ready).clicked() && ready {
                    commit = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if design::ghost_button(ui, tokens, tr("Close"), true).clicked() {
                        close = true;
                    }
                });
            });
        });

    if run_dry {
        next.report = None;
        next.error = None;
        let conn = next
            .conn
            .or(session.active_conn)
            .and_then(|i| session.conns.get(i).map(|c| c.cfg.id.clone()));
        if let (Some(conn), false) = (
            conn,
            next.path.trim().is_empty() || next.table.trim().is_empty(),
        ) {
            let mut params = serde_json::json!({
                "conn": conn,
                "path": next.path.trim(),
                "table": next.table.trim(),
            });
            if !next.mode.is_empty() {
                params["mode"] = serde_json::json!(next.mode);
            }
            match session.run("tools.importWizard", params) {
                Ok(value) => {
                    next.report = Some(value);
                    next.mapping.clear();
                }
                Err(e) => next.error = Some(e.message().to_string()),
            }
        }
    }
    if commit {
        let conn = next
            .conn
            .or(session.active_conn)
            .and_then(|i| session.conns.get(i).map(|c| c.cfg.id.clone()));
        if let Some(conn) = conn {
            let lower = next.path.to_ascii_lowercase();
            let command = if lower.ends_with(".xml") {
                "tools.importXml"
            } else if lower.ends_with(".dbf") {
                "tools.importDbf"
            } else if lower.ends_with(".txt") || lower.ends_with(".text") {
                "tools.textImporter"
            } else if lower.ends_with(".json") || lower.ends_with(".jsonl") {
                "tools.importJson"
            } else if lower.ends_with(".xlsx") {
                "tools.importExcel"
            } else {
                "tools.importCsv"
            };
            let mapping = next.mapping_entries();
            let mut params = serde_json::json!({
                "conn": conn,
                "path": next.path.trim(),
                "table": next.table.trim(),
                "mode": next.mode,
            });
            if !mapping.is_empty() {
                params["columnMapping"] = serde_json::json!(mapping);
            }
            match session.run(command, params) {
                Ok(value) => {
                    let inserted = value["inserted"].as_u64().unwrap_or(0);
                    let failed = value["failed_at_row"].as_u64();
                    match failed {
                        Some(row) => session.log(
                            Level::Error,
                            format!("import stopped at row {row}; {inserted} rows are committed"),
                        ),
                        None => session.log(Level::Success, format!("imported {inserted} rows")),
                    }
                    close = true;
                }
                Err(e) => next.error = Some(e.message().to_string()),
            }
        }
    }
    state.import_wizard = if close || !open { None } else { Some(next) };
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
                            _ => Err(nook_core::Error::config("nothing to export")),
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
                    if design::ghost_button(ui, tokens, tr("Cancel"), true).clicked() {
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

/// The active connection's id, which is the one a commit will be written through.
fn active_conn_id(session: &Session) -> Option<String> {
    session
        .active_conn
        .and_then(|i| session.conns.get(i))
        .map(|c| c.cfg.id.clone())
}

fn settings_dialog(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    if !state.settings_open {
        return;
    }
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    egui::Window::new(tr("Preferences"))
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
                // Each segment pair gets its own label: two unlabelled switches stacked are a guess,
                // not a setting.
                ui.label(
                    RichText::new(tr("Theme"))
                        .size(11.0)
                        .color(tokens.text_faint),
                );
                if let Some(index) =
                    design::segmented(ui, tokens, &[("Dark", dark), ("Light", !dark)])
                {
                    let next = if index == 0 { "dark" } else { "light" };
                    let _ = session.run(
                        "app.setSetting",
                        serde_json::json!({"key": "theme", "value": next}),
                    );
                }
                ui.add_space(6.0);
                // The chrome's language. It goes through `platform.i18n`, so the same switch works
                // from the CLI and the control channel.
                ui.label(
                    RichText::new(tr("Language"))
                        .size(11.0)
                        .color(tokens.text_faint),
                );
                let locale = session.settings.locale.clone();
                let options: Vec<(String, bool)> = crate::strings::LOCALES
                    .iter()
                    .map(|(code, name)| ((*name).to_string(), *code == locale))
                    .collect();
                let pairs: Vec<(&str, bool)> = options
                    .iter()
                    .map(|(name, on)| (name.as_str(), *on))
                    .collect();
                if let Some(index) = design::segmented(ui, tokens, &pairs) {
                    let next = crate::strings::LOCALES[index].0;
                    let _ = session.run("platform.i18n", serde_json::json!({ "locale": next }));
                }
                ui.add_space(6.0);
                let compact = session.settings.density == nook_engine::Density::Compact;
                ui.label(
                    RichText::new(tr("Density"))
                        .size(11.0)
                        .color(tokens.text_faint),
                );
                if let Some(index) = design::segmented(
                    ui,
                    tokens,
                    &[("Compact", compact), ("Comfortable", !compact)],
                ) {
                    let _ = session.run(
                        "app.setSetting",
                        serde_json::json!({
                            "key": "density",
                            "value": if index == 0 { "compact" } else { "comfortable" },
                        }),
                    );
                }
            });
            ui.add_space(8.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("Data").size(11.5).color(tokens.text_dim));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr("Page size"))
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
                ui.label(RichText::new("Safety").size(11.5).color(tokens.text_dim));
                ui.add_space(f32::from(design::S2));
                let on = session.settings.safe_mode;
                if let Some(index) =
                    design::segmented(ui, tokens, &[("Safe mode on", on), ("Off", !on)])
                {
                    let enabled = index == 0;
                    if let Err(e) =
                        session.run("editor.safeMode", serde_json::json!({ "enabled": enabled }))
                    {
                        session.log(nook_engine::Level::Error, format!("editor.safeMode: {e}"));
                    } else if enabled {
                        // Turning it on is not a flag the app can honour by itself: the first
                        // commit needs the token, so ask for the credential now rather than
                        // letting the user discover the gate at the moment they press Apply.
                        if let Some(conn) = active_conn_id(session) {
                            state.safe_prompt = Some(crate::app::SafePrompt {
                                conn,
                                ..Default::default()
                            });
                        }
                    }
                }
                ui.label(
                    RichText::new(
                        "A commit or a destructive command needs the connection's token, which is \
                         handed out only after a live connection test with the password you type. \
                         It stops a stray script and a slip of the mouse; someone who can already \
                         read the keychain has your credentials anyway.",
                    )
                    .size(11.0)
                    .color(tokens.text_faint),
                );
            });
            ui.add_space(8.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("AI").size(11.5).color(tokens.text_dim));
                ui.add_space(f32::from(design::S2));
                crate::ai::preferences(ui, session, state, tokens);
            });
            ui.add_space(8.0);
            design::card_frame(tokens).show(ui, |ui| {
                ui.label(RichText::new("Files").size(11.5).color(tokens.text_dim));
                for (label, path) in [
                    (
                        "profiles",
                        nook_engine::profiles::profiles_path().display().to_string(),
                    ),
                    (
                        "jobs",
                        nook_engine::profiles::jobs_path().display().to_string(),
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
                if design::primary_button(ui, tokens, tr("Close"), true).clicked() {
                    close = true;
                }
            });
        });
    if close || !open {
        state.settings_open = false;
    }
}

/// The safe-mode confirmation: the credential, then a live `conn.test` with it, and only then the
/// token. The dialog is the gate's own UI — a commit that was refused leaves `safe_prompt` open so
/// the user can answer it instead of guessing what the log meant.
fn safe_prompt(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    let Some(mut prompt) = state.safe_prompt.clone() else {
        return;
    };
    let frame = dialog(ui.ctx(), tokens);
    let mut open = true;
    let mut close = false;
    let mut submit = false;
    egui::Window::new("Confirm safe mode")
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .default_width(420.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(380.0);
            ui.label(
                RichText::new(format!(
                    "Type the password for {} — the token is handed out only after a live test \
                     with it.",
                    prompt.conn
                ))
                .size(11.5)
                .color(tokens.text_dim),
            );
            ui.add_space(f32::from(design::S4));
            let field = ui.add(
                egui::TextEdit::singleline(&mut prompt.password)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );
            if prompt.error.is_none() {
                field.request_focus();
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            if let Some(error) = prompt.error.clone() {
                ui.add_space(f32::from(design::S2));
                ui.label(RichText::new(error).size(11.0).color(tokens.error));
            }
            ui.add_space(f32::from(design::S4));
            ui.horizontal(|ui| {
                if design::primary_button(ui, tokens, "Confirm", !prompt.password.is_empty())
                    .clicked()
                {
                    submit = true;
                }
                if design::ghost_button(ui, tokens, tr("Cancel"), true).clicked() {
                    close = true;
                }
            });
        });

    if submit {
        match session.run(
            "editor.safeMode",
            serde_json::json!({
                "enabled": true,
                "confirm": true,
                "conn": prompt.conn,
                "password": prompt.password,
            }),
        ) {
            Ok(answer) => {
                state.safe_token = answer["token"].as_str().map(str::to_owned);
                session.log(
                    nook_engine::Level::Info,
                    format!("safe mode: {} confirmed", prompt.conn),
                );
                close = true;
            }
            // The driver's own words stay in the dialog: a wrong password and an unreachable
            // server are different problems and both are worth reading.
            Err(e) => prompt.error = Some(e.to_string()),
        }
    }
    state.safe_prompt = if close || !open { None } else { Some(prompt) };
    // The commit that opened this dialog is still staged; confirming should run it rather than
    // making the user find the Apply button again.
    if state.safe_prompt.is_none() && state.safe_token.is_some() {
        if let Some(tab_index) = state.apply_after_confirm.take() {
            let conn = session
                .tabs
                .get(tab_index)
                .and_then(|t| t.conn)
                .and_then(|i| session.conns.get(i))
                .map(|c| c.cfg.id.clone());
            let mut params = serde_json::json!({});
            if let (Some(conn), Some(map)) = (conn, params.as_object_mut()) {
                map.insert("conn".into(), serde_json::json!(conn));
                map.insert(
                    "safe_token".into(),
                    serde_json::json!(state.safe_token.clone().unwrap_or_default()),
                );
            }
            match session.run("grid.apply", params) {
                Ok(_) => session.log(
                    nook_engine::Level::Info,
                    "safe mode: the waiting commit ran".to_owned(),
                ),
                Err(e) => session.log(nook_engine::Level::Error, format!("grid.apply: {e}")),
            }
        }
    }
}

/// The palette overlay: one overlay, two modes. `Cmd K` (or a `>` prefix) searches commands;
/// `Cmd P` (or a `#` prefix, or plain typing) searches objects, saved queries and tabs through
/// `quickOpen.search`. Enter runs the first match in either mode.
fn palette(ui: &mut egui::Ui, session: &mut Session, state: &mut UiState, tokens: &Tokens) {
    if !state.palette_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let frame = dialog(&ctx, tokens);
    let mut open = true;
    let mut close = false;
    let mut run: Option<&'static str> = None;
    let mut open_hit: Option<serde_json::Value> = None;
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
            // The `>` and `#` prefixes switch mode from the field — the convention every editor
            // already taught the user.
            if state.palette_query.starts_with('>') {
                state.palette_objects = false;
            } else if state.palette_query.starts_with('#') {
                state.palette_objects = true;
            }
            let field_query = state
                .palette_query
                .trim_start_matches(['>', '#'])
                .to_string();
            let hint = if state.palette_objects {
                "search tables, saved queries, tabs…  (  # objects   > commands )"
            } else {
                "search a command by id or label…  (  > commands   # objects )"
            };
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.palette_query)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(8, 4)),
            );
            response.request_focus();
            ui.add_space(4.0);
            let mut obj_hits: Vec<serde_json::Value> = Vec::new();
            if state.palette_objects {
                // The search is the engine's, not a reimplementation: one source of ranking.
                if let Ok(out) = session.run(
                    "quickOpen.search",
                    serde_json::json!({"query": field_query, "limit": 60}),
                ) {
                    if let Some(list) = out["hits"].as_array() {
                        obj_hits = list.clone();
                    }
                }
            }
            let needle = field_query.to_ascii_lowercase();
            let matches: Vec<&'static nook_engine::CommandSpec> = if state.palette_objects {
                Vec::new()
            } else {
                nook_engine::registry()
                    .iter()
                    .filter(|spec| {
                        needle.is_empty()
                            || spec.id.to_ascii_lowercase().contains(&needle)
                            || spec.label.to_ascii_lowercase().contains(&needle)
                            || spec.group.to_ascii_lowercase().contains(&needle)
                    })
                    .collect()
            };
            let entered = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if entered {
                if let Some(first) = matches.first() {
                    run = Some(first.id);
                } else if let Some(first) = obj_hits.first() {
                    open_hit = Some(first.clone());
                }
            }
            let one_conn = session.conns.iter().filter(|c| c.catalog.is_some()).count() <= 1;
            egui::ScrollArea::vertical()
                .id_salt("palette-list")
                .max_height(360.0)
                .show(ui, |ui| {
                    if state.palette_objects {
                        for hit in obj_hits.iter().take(60) {
                            let name = hit["name"].as_str().unwrap_or_default().to_string();
                            let kind = hit["kind"].as_str().unwrap_or_default().to_string();
                            let conn = hit["conn"].as_str().unwrap_or_default().to_string();
                            let schema = hit["schema"].as_str().unwrap_or_default();
                            let label = if schema.is_empty() {
                                name.clone()
                            } else {
                                format!("{schema}.{name}")
                            };
                            // The connection chip: with more than one open catalog, the same
                            // table name in three databases must not be a coin flip.
                            let text = if one_conn || conn.is_empty() {
                                format!("{kind}  ·  {label}")
                            } else {
                                format!("{kind}  ·  {label}  ·  {conn}")
                            };
                            // `.small()` keeps the row at 22px — the theme bumps normal buttons
                            // to 28, which would leave 6px of dead hover. The button's own fill
                            // paints the hover tint under its text (painting after `ui.add` is
                            // what made the text disappear before).
                            let row_rect = egui::Rect::from_min_size(
                                ui.cursor().min,
                                egui::Vec2::new(ui.available_width(), 22.0),
                            );
                            let hovered = ui.rect_contains_pointer(row_rect);
                            let response = ui.add(
                                egui::Button::new(
                                    RichText::new(text).size(12.0).color(tokens.text),
                                )
                                .small()
                                .fill(if hovered {
                                    tokens.selection
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE)
                                .min_size(egui::Vec2::new(ui.available_width(), 22.0)),
                            );
                            if response.clicked() {
                                open_hit = Some(hit.clone());
                            }
                        }
                        if obj_hits.is_empty() {
                            ui.label(
                                RichText::new("nothing matches")
                                    .size(12.0)
                                    .color(tokens.text_faint),
                            );
                        }
                    } else {
                        let mut group = "";
                        for spec in matches.iter().take(60) {
                            if spec.group != group {
                                group = spec.group;
                                ui.add_space(2.0);
                                ui.label(RichText::new(group).size(10.5).color(tokens.text_faint));
                            }
                            // Same as the objects list: `.small()` keeps 22px, and the button's
                            // own fill paints the hover under its text.
                            let row_rect = egui::Rect::from_min_size(
                                ui.cursor().min,
                                egui::Vec2::new(ui.available_width(), 22.0),
                            );
                            let hovered = ui.rect_contains_pointer(row_rect);
                            let response = ui.add(
                                egui::Button::new(
                                    RichText::new(format!("{}  ·  {}", spec.id, spec.label))
                                        .size(12.0)
                                        .color(tokens.text),
                                )
                                .small()
                                .fill(if hovered {
                                    tokens.selection
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE)
                                .min_size(egui::Vec2::new(ui.available_width(), 22.0)),
                            );
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
                    }
                });
            ui.separator();
            let count = if state.palette_objects {
                obj_hits.len()
            } else {
                matches.len()
            };
            ui.horizontal(|ui| {
                design::keycap(ui, tokens, "Enter");
                ui.label(
                    RichText::new(if state.palette_objects {
                        "open the first match"
                    } else {
                        "run the first match"
                    })
                    .size(11.0)
                    .color(tokens.text_faint),
                );
                design::chip(ui, tokens, &format!("{count} matches"), tokens.text_dim);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    design::keycap(ui, tokens, "Esc");
                });
            });
        });
    if let Some(id) = run {
        state.open_command(id.to_string());
        close = true;
    }
    if let Some(hit) = open_hit {
        let mut params = serde_json::json!({
            "kind": hit["kind"].clone(),
            "name": hit["name"].clone(),
        });
        if let Some(conn) = hit["conn"].as_str() {
            if !conn.is_empty() {
                params["conn"] = serde_json::json!(conn);
            }
        }
        if let Some(schema) = hit["schema"].as_str() {
            if !schema.is_empty() {
                params["schema"] = serde_json::json!(schema);
            }
        }
        if let Err(e) = session.run("quickOpen.open", params) {
            session.log(Level::Warn, e.to_string());
        }
        close = true;
    }
    if close || !open {
        state.palette_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ssh_auth_of;

    /// The design's window, the way the app drives one: one screen rect, no texture uploads, on the
    /// clock the caller sets.
    fn screen(events: Vec<egui::Event>, time: f64) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// One click at `pos`: the pointer arrives and then presses and releases.
    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            press(pos, true),
            press(pos, false),
        ]
    }

    /// The profile store in a scratch directory, the way the engine's own tests do it: `conn.new`
    /// writes a profile, and a UI test must not touch the user's.
    fn scratch() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
        let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = std::env::temp_dir().join("nook-ui-test-config");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::remove_file(dir.join("profiles.json"));
        std::env::set_var("NOOK_CONFIG_DIR", dir);
        guard
    }

    /// Every shape one frame painted, the nested ones unpacked.
    fn shapes(out: &egui::FullOutput) -> Vec<egui::Shape> {
        fn walk(shape: &egui::Shape, flat: &mut Vec<egui::Shape>) {
            match shape {
                egui::Shape::Vec(parts) => parts.iter().for_each(|part| walk(part, flat)),
                shape => flat.push(shape.clone()),
            }
        }
        let mut flat = Vec::new();
        for clipped in &out.shapes {
            walk(&clipped.shape, &mut flat);
        }
        flat
    }

    /// The box a painted string occupies — its anchor, since `painter.text` places the galley by the
    /// alignment it was given.
    fn text_rect(painted: &[egui::Shape], wanted: &str) -> Option<egui::Rect> {
        painted.iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                Some(shape.visual_bounding_rect())
            }
            _ => None,
        })
    }

    /// Where a painted string was anchored: for a centred label that is the label's own centre, which
    /// is what a click has to aim at.
    fn text_pos(painted: &[egui::Shape], wanted: &str) -> Option<egui::Pos2> {
        painted.iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.pos),
            _ => None,
        })
    }

    /// The ink a painted string wears, off the galley's own job: a `painter.text` call lays the colour
    /// down in the layout it built, while the shape's fallback is the placeholder egui would use.
    fn ink(painted: &[egui::Shape], wanted: &str) -> Option<Color32> {
        painted.iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                text.galley.job.sections.first().map(|s| s.format.color)
            }
            _ => None,
        })
    }

    /// The band the design names by its own size — the head 68, the footer 89 — both `DIALOG_W` wide
    /// and filled with the panel.
    fn band(painted: &[egui::Shape], tokens: &Tokens, height: f32) -> egui::Rect {
        painted
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.fill == tokens.panel
                        && (rect.rect.width() - design::DIALOG_W).abs() <= 0.5
                        && (rect.rect.height() - height).abs() <= 0.5 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "a {height}px band of {DIALOG_W} width",
                    DIALOG_W = design::DIALOG_W
                )
            })
    }

    /// Save's own rect and face: the 28px button in the footer's lower row, `accent` when the form is
    /// complete and the blocked one (`.off`'s `panel_high`) when it is not. Cancel is `panel_alt`, and
    /// the first row's own action is not the Save at all, so the row and the fill together say which.
    fn save_button(
        painted: &[egui::Shape],
        tokens: &Tokens,
        footer: egui::Rect,
    ) -> (egui::Rect, Color32) {
        painted
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if (rect.rect.height() - design::BUTTON_H).abs() <= 0.5
                        && footer.contains(rect.rect.center())
                        && rect.rect.center().y > footer.center().y
                        && (rect.fill == tokens.accent || rect.fill == tokens.panel_high) =>
                {
                    Some((rect.rect, rect.fill))
                }
                _ => None,
            })
            .expect("a Save button in the footer")
    }

    /// The app's own frame loop, headless: one context, one state and one session, with the dialog
    /// stack under the design's tokens.
    struct Editor {
        ctx: egui::Context,
        tokens: Tokens,
        state: UiState,
        session: Session,
        /// The clock. An `Area` fades in over `animation_time`, so a frame that ran with no time on it
        /// would be photographed half-transparent — and half a scrim is not the design's colour.
        clock: f64,
    }

    impl Editor {
        fn new() -> Editor {
            let ctx = egui::Context::default();
            let tokens = Tokens::of("dark");
            tokens.apply(&ctx);
            let mut state = UiState::default();
            state.connection_open = true;
            Editor {
                ctx,
                tokens,
                state,
                session: Session::new(),
                clock: 0.0,
            }
        }

        /// One pass on the editor's clock.
        fn pass(&mut self, events: Vec<egui::Event>) -> Vec<egui::Shape> {
            self.clock += 1.0;
            let (clock, events) = (self.clock, events);
            let Editor {
                ctx,
                tokens,
                state,
                session,
                ..
            } = self;
            let mut out = ctx.run_ui(screen(events, clock), |ui| show(ui, session, state, tokens));
            // The frame paints into a texture delta the headless context never applies.
            out.textures_delta.clear();
            shapes(&out)
        }

        /// One frame: the events, then two quiet passes. The first pass only lays the modal out — an
        /// `Area`'s frame is a placeholder until the area knows its own size, and the position it
        /// derives from that placeholder is 68px low, enough to put the footer's buttons outside the
        /// footer — so the caller gets the third pass, by which the layout has settled. The events
        /// still land on the first, where the widget rects are the settled ones of the last frame.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::Shape> {
            let _ = self.pass(events);
            let _ = self.pass(Vec::new());
            self.pass(Vec::new())
        }
    }

    /// The editor is the design's `.dlg`: the window under the scrim, a 560px dialog centred in the
    /// 1440 the design draws it in, the 68px head at its top with its `--s7` inset — which is the
    /// body's content edge as well — and the 89px footer below it.
    #[test]
    fn the_editor_is_a_centred_scrimmed_shell() {
        let mut editor = Editor::new();
        let painted = editor.frame(Vec::new());

        // The scrim covers the window.
        let scrim = painted
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect) if rect.fill == editor.tokens.scrim => Some(rect.rect),
                _ => None,
            })
            .expect("the backdrop");
        assert!((scrim.width() - 1440.0).abs() <= 0.5, "{scrim:?}");
        assert!((scrim.height() - 900.0).abs() <= 0.5, "{scrim:?}");

        // The dialog is the design's 560, centred the way a 1440 window centres it.
        let head = band(&painted, &editor.tokens, design::DLG_HEAD_H);
        assert!((head.left() - 440.0).abs() <= 0.5, "{head:?}");
        assert!((head.right() - 1000.0).abs() <= 0.5, "{head:?}");

        // The head's `--s7` inset, and the footer's own 8px dot in the 89 below the head.
        let title = text_pos(&painted, "New connection").expect("the head's title");
        assert!((title.x - 460.0).abs() <= 0.5, "{title:?}");
        let foot = band(&painted, &editor.tokens, design::DLG_FOOT_H);
        assert!(foot.top() > head.bottom(), "{foot:?} is not below {head:?}");
        let dot = painted.iter().find_map(|shape| match shape {
            egui::Shape::Circle(circle) if circle.fill == editor.tokens.text_faint => {
                Some((circle.center, circle.radius))
            }
            _ => None,
        });
        let (centre, radius) = dot.expect("the footer's status dot");
        assert!((radius - design::STATUS_DOT / 2.0).abs() <= 0.5, "{radius}");
        assert!(foot.contains(centre), "{centre:?} is not in {foot:?}");
    }

    /// A form row is one row: every field's label line sits on a single y, and so does every `.inp`
    /// under them. A column that lets the row place it by its own height staggers the row — the label
    /// block is 22 tall, so a column that starts at the row's own baseline drifts by 18.
    #[test]
    fn every_field_in_a_row_starts_at_the_same_y() {
        let mut editor = Editor::new();
        editor.state.connection_form.kind = DriverKind::Postgres;
        let painted = editor.frame(Vec::new());

        let labels = ["Host", "Port", "Database"].map(|label| {
            (
                label,
                text_pos(&painted, label).unwrap_or_else(|| panic!("the {label} label")),
            )
        });
        for (label, position) in labels {
            assert!(
                (position.y - labels[0].1.y).abs() <= 0.5,
                "the {label} label is not level with the row: {labels:?}"
            );
        }

        // A `TextEdit`'s hint lives in a clipped galley the frame does not hand out, so the row's own
        // evidence is the boxes: each `.inp` is 28 of `panel_alt` directly under its column's label,
        // and the label's anchor names the column. The box looked for is the first one *below* the
        // label — the row above's fields share the column's x and would answer first otherwise.
        let inputs = labels.map(|(label, position)| {
            let below = |shape: &egui::Shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.fill == editor.tokens.panel_alt
                        && (rect.rect.height() - design::INPUT_H).abs() <= 0.5
                        && rect.rect.x_range().contains(position.x)
                        && rect.rect.top() >= position.y =>
                {
                    Some((rect.rect.top() - position.y, rect.rect))
                }
                _ => None,
            };
            let rect = painted
                .iter()
                .filter_map(below)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap_or_else(|| panic!("the {label} field's .inp"))
                .1;
            (label, rect, position)
        });
        for (label, rect, _) in inputs {
            assert!(
                (rect.top() - inputs[0].1.top()).abs() <= 0.5,
                "the {label} field is not level with the row: {inputs:?}"
            );
        }
    }

    /// The tabs answer to the form's step and mark it, and a press off the band moves nothing —
    /// which is what `dialogue_tabs` returning `None` means.
    #[test]
    fn the_tabs_mark_the_active_step_and_move_it() {
        // The marks: the active step is the one at full ink, the rest are the design's dim.
        let mut editor = Editor::new();
        let painted = editor.frame(Vec::new());
        assert_eq!(ink(&painted, "General"), Some(editor.tokens.text));
        assert_eq!(ink(&painted, "TLS"), Some(editor.tokens.text_dim));
        assert_eq!(ink(&painted, "Tunnel"), Some(editor.tokens.text_dim));
        assert_eq!(ink(&painted, "Advanced"), Some(editor.tokens.text_dim));
        let tls = text_pos(&painted, "TLS").expect("the TLS tab");

        // A click on the tab moves the step, and the marks follow it.
        let mut clicked = Editor::new();
        let _ = clicked.frame(Vec::new());
        let _ = clicked.frame(click(tls));
        assert_eq!(clicked.state.connection_form.step, ConnStep::Tls);
        let painted = clicked.frame(Vec::new());
        assert_eq!(ink(&painted, "TLS"), Some(clicked.tokens.text));
        assert_eq!(ink(&painted, "General"), Some(clicked.tokens.text_dim));

        // A press off the band — in the head above it, where nothing is sensed — is no click: the
        // step stays and the dialog stays open, so the press was the dialog's, not the backdrop's.
        let mut missed = Editor::new();
        let _ = missed.frame(Vec::new());
        let _ = missed.frame(click(egui::pos2(tls.x, tls.y - 40.0)));
        assert!(missed.state.connection_open, "the press closed the editor");
        assert_eq!(missed.state.connection_form.step, ConnStep::General);
    }

    /// Save is live exactly while the form is complete, and the sentence beside it is the one
    /// `conn.new` would refuse the same input with — so the line and the button cannot disagree with
    /// the engine.
    #[test]
    fn save_is_off_exactly_while_the_form_is_blocked() {
        let _guard = scratch();
        let mut editor = Editor::new();
        let painted = editor.frame(Vec::new());
        let foot = band(&painted, &editor.tokens, design::DLG_FOOT_H);
        let blocked = editor
            .state
            .connection_form
            .blocker()
            .expect("a form with no name is blocked");
        assert!(text_rect(&painted, &blocked).is_some(), "the `.why` line");
        assert_eq!(
            save_button(&painted, &editor.tokens, foot).1,
            editor.tokens.panel_high
        );
        // The blocked Save carries that sentence as its hover text too, so the refusal is on the
        // control that would run the command and not only on the line beside it.
        let lines = |shapes: &[egui::Shape], want: &str| {
            shapes
                .iter()
                .filter(
                    |shape| matches!(shape, egui::Shape::Text(text) if text.galley.text() == want),
                )
                .count()
        };
        assert_eq!(
            lines(&painted, &blocked),
            1,
            "the `.why` line, and no tooltip"
        );
        let save = save_button(&painted, &editor.tokens, foot).0;
        let _ = editor.frame(vec![egui::Event::PointerMoved(save.center())]);
        let hovered = editor.frame(vec![egui::Event::PointerMoved(save.center())]);
        assert_eq!(
            lines(&hovered, &blocked),
            2,
            "Save's hover text is the blocker's own sentence"
        );
        // The engine refuses the same input with the very sentence the footer prints.
        let refused = editor
            .session
            .run("conn.new", serde_json::json!({"kind": "postgres"}))
            .expect_err("no name");
        assert!(refused.to_string().ends_with(&blocked), "{refused}");

        // And the file-based profile's own sentence, again the engine's.
        editor.state.connection_form.name = "shop".into();
        editor.state.connection_form.kind = DriverKind::Sqlite;
        let blocked = editor
            .state
            .connection_form
            .blocker()
            .expect("a SQLite profile with no file is blocked");
        let refused = editor
            .session
            .run(
                "conn.new",
                serde_json::json!({"name": "shop", "kind": "sqlite", "file": ""}),
            )
            .expect_err("no file");
        assert!(refused.to_string().ends_with(&blocked), "{refused}");

        // With a name and no file needed, the design's own sentence replaces it and Save wears the
        // primary face.
        editor.state.connection_form.name = "shop".into();
        editor.state.connection_form.kind = DriverKind::Postgres;
        let painted = editor.frame(Vec::new());
        assert_eq!(editor.state.connection_form.blocker(), None);
        assert!(text_rect(&painted, "The profile is complete.").is_some());
        assert!(text_rect(&painted, &blocked).is_none());
        let foot = band(&painted, &editor.tokens, design::DLG_FOOT_H);
        assert_eq!(
            save_button(&painted, &editor.tokens, foot).1,
            editor.tokens.accent
        );
    }

    /// The design's one-of-three: the tunnel's auth mode is derived from the profile's own fields,
    /// and the user's own choice wins once it disagrees.
    #[test]
    fn the_auth_mode_comes_from_the_profile_and_the_users_choice_wins() {
        assert_eq!(ssh_auth_of("/keys/id", false), SshAuth::KeyFile);
        assert_eq!(ssh_auth_of("", true), SshAuth::Password);
        assert_eq!(ssh_auth_of("", false), SshAuth::Agent);
        // A key file and a stored password both set: the key file is what the field says.
        assert_eq!(ssh_auth_of("/keys/id", true), SshAuth::KeyFile);

        let mut form = ConnectionForm {
            ssh_key: "/keys/id".into(),
            ..ConnectionForm::default()
        };
        assert_eq!(form.ssh_auth(), SshAuth::KeyFile);
        form.ssh_auth = SshAuth::Password;
        assert_eq!(form.ssh_auth(), SshAuth::Password);
        form.ssh_auth = SshAuth::Agent;
        assert_eq!(form.ssh_auth(), SshAuth::KeyFile);

        // And a profile: the tunnel is on when it has a host, which is the only toggle it has.
        let mut cfg = nook_core::ConnectionConfig::new("jump", DriverKind::Postgres);
        cfg.ssh.host = "jump.example.com".into();
        cfg.ssh.key_file = "/keys/id".into();
        let form = ConnectionForm::from_config(&cfg);
        assert_eq!(form.ssh_auth(), SshAuth::KeyFile);
        assert!(form.ssh_enabled());
        cfg.ssh.host = String::new();
        let form = ConnectionForm::from_config(&cfg);
        assert!(!form.ssh_enabled());
        assert_eq!(form.ssh_auth(), SshAuth::KeyFile);
    }

    /// The password helpers are where the keychain state is spoken, and the engine's own boolean is
    /// what the form starts from — so a field cannot promise a keychain the engine says is absent.
    #[test]
    fn the_keychain_state_is_what_the_password_helpers_report() {
        assert!(password_helper(true).contains("keychain"));
        assert!(password_helper(false).contains("no OS keychain"));
        assert!(password_helper(false).contains("mode 0600"));

        // The SSH helper hands the engine's own refusal back where there is nothing to promise.
        assert_eq!(
            ssh_helper(false, SshAuth::Password),
            nook_engine::profiles::NO_KEYCHAIN
        );
        assert_eq!(
            ssh_helper(false, SshAuth::KeyFile),
            nook_engine::profiles::NO_KEYCHAIN
        );
        for auth in [SshAuth::KeyFile, SshAuth::Agent, SshAuth::Password] {
            assert_ne!(ssh_helper(true, auth), nook_engine::profiles::NO_KEYCHAIN);
        }
        assert_eq!(
            ConnectionForm::default().keychain,
            nook_engine::profiles::keychain_available()
        );
    }

    /// The confirmation waits for the typed name, and the command it runs is the one the engine
    /// gates behind `confirm`.
    #[test]
    fn the_confirmation_waits_for_the_name_and_runs_the_gated_command() {
        let mut confirm = Confirm {
            command: "conn.delete".into(),
            params: serde_json::json!({"conn": "doomed"}),
            title: "Delete connection".into(),
            subject: "doomed".into(),
            sentence: "Deleting doomed removes the profile and the passwords saved with it.".into(),
            facts: vec!["SQLite".into(), "not connected".into()],
            action: "Delete".into(),
            typed: String::new(),
            expected: "doomed".into(),
        };
        assert!(!confirm.satisfied());
        assert!(confirm.why().contains("doomed"));
        assert!(confirm.why().contains("Delete"));
        // A pasted name with the whitespace a terminal adds still satisfies it.
        confirm.typed = "  doomed ".into();
        assert!(confirm.satisfied());
        assert!(confirm.why().contains("matches"));

        // The gate this answers: without the flag the engine refuses the same command.
        let _guard = scratch();
        let mut session = Session::new();
        let refused = session
            .run(&confirm.command, confirm.params.clone())
            .expect_err("the engine's own gate");
        assert!(refused.to_string().contains("confirm"), "{refused}");
    }

    /// The confirmation is the design's narrow shell: 440 wide — x 500..940 in the 1440 the design
    /// draws it in — the 20px `.ig` badge in the head, its `.facts` chips 20 tall `--s3` apart under
    /// the sentence, one 53px footer band, and a `.btn.danger` that is `.off` until the typed name
    /// matches the object's own.
    #[test]
    fn the_confirmation_is_the_narrow_shell_with_its_facts_and_the_risk_face() {
        let mut editor = Editor::new();
        editor.state.connection_open = false;
        editor.state.confirm = Some(Confirm {
            command: "conn.delete".into(),
            params: serde_json::json!({"conn": "shop"}),
            title: "Delete connection".into(),
            subject: "shop".into(),
            sentence: "Deleting shop removes the profile and the passwords saved with it.".into(),
            facts: vec![
                "SQLite".into(),
                "not connected".into(),
                "group local".into(),
            ],
            action: "Delete".into(),
            typed: String::new(),
            expected: "shop".into(),
        });
        let painted = editor.frame(Vec::new());
        let tokens = editor.tokens;

        // The narrow shell, centred the way the design centres a 440 in 1440.
        let head = painted
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.fill == tokens.panel
                        && (rect.rect.height() - design::DLG_HEAD_H).abs() <= 0.5 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("the confirmation's head");
        assert!(
            (head.left() - 500.0).abs() <= 0.5 && (head.right() - 940.0).abs() <= 0.5,
            "the confirmation is 440 wide at x 500..940: {head:?}"
        );

        // The `.ig` badge: the design's 20px `error`-stroked box, `--s1` into the head.
        let badge = painted
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.stroke.color == tokens.error
                        && (rect.rect.height() - design::DLG_CHIP_H).abs() <= 0.5
                        && (rect.rect.width() - design::DLG_CHIP_H).abs() <= 0.5 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("the head's .ig badge");
        assert_eq!(badge.width(), design::DLG_CHIP_H, "{badge:?}");
        assert!(badge.top() > head.top() && badge.bottom() < head.bottom());

        // The `.facts` chips: one per fact, 20 tall, `--s3` apart, in the body.
        let mut chips: Vec<egui::Rect> = painted
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.fill == tokens.panel_alt
                        && (rect.rect.height() - design::DLG_CHIP_H).abs() <= 0.5 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect();
        chips.sort_by(|a, b| a.left().total_cmp(&b.left()));
        assert_eq!(chips.len(), 3, "one chip per fact: {chips:?}");
        for pair in chips.windows(2) {
            assert!(
                (pair[1].left() - pair[0].right() - f32::from(design::S3)).abs() <= 0.5,
                "the chips are --s3 apart: {pair:?}"
            );
        }

        // The single 53px footer band, and the danger face: `.off` until the name matches.
        assert!(
            painted
                .iter()
                .any(|shape| matches!(shape, egui::Shape::Rect(rect)
                if rect.fill == tokens.panel
                    && (rect.rect.height() - design::DLG_FOOT_ONE_H).abs() <= 0.5
                    && (rect.rect.width() - design::DIALOG_NARROW_W).abs() <= 0.5)),
            "the confirmation's one-row footer"
        );
        assert!(
            painted
                .iter()
                .any(|shape| matches!(shape, egui::Shape::Rect(rect)
                if rect.fill == tokens.panel_high)),
            "an untyped name leaves the danger button `.off`"
        );
        assert!(
            !painted
                .iter()
                .any(|shape| matches!(shape, egui::Shape::Rect(rect)
                if rect.fill == tokens.error)),
            "and nothing wears the risk fill yet"
        );

        editor
            .state
            .confirm
            .as_mut()
            .expect("the pending dialog")
            .typed = "shop".into();
        let painted = editor.frame(Vec::new());
        assert!(
            painted
                .iter()
                .any(|shape| matches!(shape, egui::Shape::Rect(rect)
                if rect.fill == tokens.error)),
            "the matching name arms the danger button"
        );
    }

    /// The pinned row states the hop when a tunnel ran and the server otherwise.
    /// The Test row states the engine's own outcome: the dot is the design's 8 (`STATUS_DOT`, so a
    /// radius of 4) and wears `ok` or `error`, and the action offers the second try once the first
    /// failed. Driven over the control channel this row cannot be reached — `conn.test` writes the
    /// session's log, not the editor's pinned state — so the frame is read here.
    #[test]
    fn the_test_row_wears_the_outcome_and_offers_the_second_try() {
        let _guard = scratch();
        for (ok, name, action) in [(true, "ok", "Test connection"), (false, "error", "Retry")] {
            let mut editor = Editor::new();
            editor.state.connection_editing = Some(0);
            editor.pass(vec![]);
            editor.pass(vec![]);
            // The editor clears the pinned row on the frame it opens, so the outcome goes on after it
            // has settled — which is also the order the app itself has (a dialog opens, a test runs).
            editor.state.test_state = Some(TestOutcome {
                ok,
                line: format!("{name} · PostgreSQL 16.3 · 12 ms"),
            });
            let painted = editor.pass(vec![]);
            let want = if ok {
                editor.tokens.ok
            } else {
                editor.tokens.error
            };
            let dot = painted.iter().find_map(|shape| match shape {
                egui::Shape::Circle(circle) if circle.fill == want => Some(circle.radius),
                _ => None,
            });
            assert_eq!(
                dot,
                Some(design::STATUS_DOT / 2.0),
                "{name}: the dot's radius"
            );
            assert!(
                text_pos(&painted, action).is_some(),
                "{name}: the row offers {action}"
            );
        }
    }

    #[test]
    fn the_pinned_row_states_the_hop_or_the_server() {
        let server =
            serde_json::json!({"driver": "PostgreSQL", "version": "16.3", "elapsed_ms": 12});
        assert_eq!(test_line(&server), "ok · PostgreSQL 16.3 · 12 ms");
        let tunnel = serde_json::json!({
            "driver": "PostgreSQL", "version": "16.3", "elapsed_ms": 40,
            "tunnel": {"host": "jump.example.com", "port": 22, "local_port": 15432},
        });
        assert_eq!(
            test_line(&tunnel),
            "tunnel ok · jump.example.com:22 · local 15432 · 40 ms"
        );
    }
}
