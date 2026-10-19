//! The connection sidebar: search, connection cards, schema and table lists.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Vec2};
use nook_engine::{ConnState, Session, TabKind, TableRef};

use crate::app::UiState;
use crate::icons::{self, Glyph};
use crate::theme::{self as design, Tokens};

/// The design's `.seg` in the tree: which objects the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    Connections,
    Tables,
    Views,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::All, Scope::Connections, Scope::Tables, Scope::Views];

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "All",
            Scope::Connections => "Connections",
            Scope::Tables => "Tables",
            Scope::Views => "Views",
        }
    }

    /// Whether an object of this kind is listed. The table loop and the count share it, so the
    /// match line can never disagree with the rows under it.
    fn shows(self, kind: nook_model::TableKind) -> bool {
        match self {
            Scope::All => true,
            Scope::Connections => false,
            Scope::Tables => !is_view(kind),
            Scope::Views => is_view(kind),
        }
    }
}

/// A view is not a table, and both the badge and the scope control say so.
fn is_view(kind: nook_model::TableKind) -> bool {
    matches!(
        kind,
        nook_model::TableKind::View | nook_model::TableKind::MatView
    )
}

/// What the filter currently finds, for the design's `.matchline`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Found {
    connections: usize,
    tables: usize,
    views: usize,
}

impl Found {
    /// How many objects the current scope lists. The match line and the rows share this, so the
    /// count can never disagree with the list under it.
    pub fn count(self, scope: Scope) -> usize {
        match scope {
            Scope::All => self.tables + self.views,
            Scope::Connections => self.connections,
            Scope::Tables => self.tables,
            Scope::Views => self.views,
        }
    }

    /// The match line counts what the search finds across every catalog, not only the expanded
    /// connections: it answers "did my filter match anything", which expansion does not change.
    /// The design's `.matchline .n` reads "3 objects" for the All scope and "N matches" otherwise.
    pub fn line(self, scope: Scope) -> String {
        let count = self.count(scope) as u64;
        let (one, many) = match scope {
            Scope::All => ("object", "objects"),
            _ => ("match", "matches"),
        };
        format!(
            "{} {}",
            crate::table::grouped(count),
            crate::table::plural(count, one, many)
        )
    }
}

/// Count what the filter finds for the match line.
fn counts(session: &Session, needle: &str, scope: Scope) -> Found {
    let mut found = Found {
        connections: session.conns.len(),
        ..Found::default()
    };
    if scope == Scope::Connections {
        return found;
    }
    for conn in &session.conns {
        let Some(catalog) = conn.catalog.as_ref() else {
            continue;
        };
        for schema in &catalog.schemas {
            for table in &schema.tables {
                if !haystack(needle, &table.name) && !haystack(needle, &schema.name) {
                    continue;
                }
                if !scope.shows(table.kind) {
                    continue;
                }
                if is_view(table.kind) {
                    found.views += 1;
                } else {
                    found.tables += 1;
                }
            }
        }
    }
    found
}

/// The filter matches a name that contains the needle, case-insensitively.
fn haystack(needle: &str, name: &str) -> bool {
    needle.is_empty() || name.to_ascii_lowercase().contains(needle)
}

/// What the tree wants the app to do next, so the tree itself stays read-only.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeAction {
    None,
    OpenConnection(usize),
    CloseConnection(usize),
    SelectConnection(usize),
    Refresh(usize),
    /// A single click on an object: the inspector describes it, nothing opens.
    SelectTable(usize, TableRef),
    OpenData(usize, TableRef),
    OpenDesign(usize, TableRef),
    OpenDdl(usize, TableRef),
    OpenDiagram(usize),
    NewQuery(usize),
    Monitor(usize),
    Users(usize),
}

/// The tree's title band, from the design's `.reg-tree .panehead`.
pub const HEADER_H: f32 = 34.0;
/// `.filterblk`: 6 + 28 (field) + 6 + 28 (scope) + 6 + 16 (match line) + 8, so the pane head —
/// [`HEADER_H`] + 98 — is the design's `--h-panehead`, the line the inspector also starts its body
/// on.
pub const FILTER_BLOCK_H: f32 = 98.0;

pub fn show(
    ui: &mut egui::Ui,
    session: &Session,
    state: &mut UiState,
    tokens: &Tokens,
) -> TreeAction {
    let mut action = TreeAction::None;

    // Header: title and the one action that starts everything.
    // The pane's bands are contiguous in the design — 34 + 98, rows 32 — so the default
    // `item_spacing` must not add a gap between them.
    ui.spacing_mut().item_spacing.y = 0.0;
    let header = design::band_h(ui, tokens, HEADER_H, |ui| {
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

    let needle = state.tree_filter.trim().to_ascii_lowercase();
    let found = counts(session, &needle, state.tree_scope);
    // `band_fill` lays its content out left to right, so a child as wide as the band leaves nothing
    // for the next one: the inner width is measured once, before the band claims it.
    let inner_width = (ui.available_width() - 2.0 * f32::from(design::S4)).max(0.0);
    // `.filterblk` stacks its three rows; a band's own layout is left to right, so the block gets a
    // child of its own rather than a closure that would lay the field and the control out sideways.
    let (filter, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), FILTER_BLOCK_H),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(filter, CornerRadius::ZERO, tokens.panel);
    let filter_inner = egui::Rect::from_min_max(
        egui::pos2(filter.left() + f32::from(design::S4), filter.top()),
        egui::pos2(filter.right() - f32::from(design::S4), filter.bottom()),
    );
    {
        let mut block = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(filter_inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut block;
        {
            ui.add_space(f32::from(design::S2 + design::S1));
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.tree_filter)
                    .hint_text(
                        RichText::new("Filter connections and objects")
                            .size(design::T_SM)
                            .color(tokens.text_faint),
                    )
                    .font(tokens.font(design::T_SM))
                    .text_color(tokens.text)
                    .desired_width(inner_width)
                    // `.field` is 28 tall: a 12px galley, 1px of frame each side and 7 above and
                    // below, with the magnifier inside at the design's 8px lead-in.
                    .margin(egui::Margin {
                        left: design::S4 + design::GLYPH_SM as i8 + design::S3,
                        right: design::S4,
                        top: 7,
                        bottom: 7,
                    }),
            );
            icons::glyph(
                ui,
                egui::Rect::from_min_size(
                    egui::pos2(
                        field.rect.left() + f32::from(design::S4),
                        field.rect.center().y - design::GLYPH_SM / 2.0,
                    ),
                    Vec2::splat(design::GLYPH_SM),
                ),
                Glyph::Search,
                tokens.text_faint,
            );
            ui.add_space(f32::from(design::S2 + design::S1));
            let options = Scope::ALL.map(|scope| (scope.label(), state.tree_scope == scope));
            // `.filterblk .seg{height:--h-head(28); padding:1px}`, against the inspector's 26 and 2.
            if let Some(index) = design::pane_segmented(
                ui,
                tokens,
                &options,
                inner_width,
                design::HEAD_H,
                1.0,
                tokens.text,
            ) {
                state.tree_scope = Scope::ALL[index];
            }
            ui.add_space(f32::from(design::S2 + design::S1));
            let (line, _) = ui.allocate_exact_size(Vec2::new(inner_width, 16.0), Sense::hover());
            // `.matchline`: the scope label on the left in `--muted`, the count on the right in
            // `--fg` — or `--warn` when the filter found nothing.
            let painter = ui.painter();
            painter.text(
                egui::pos2(line.left(), line.center().y),
                Align2::LEFT_CENTER,
                format!("scope {}", state.tree_scope.label()),
                tokens.mono(design::T_XS),
                tokens.text_dim,
            );
            painter.text(
                egui::pos2(line.right(), line.center().y),
                Align2::RIGHT_CENTER,
                found.line(state.tree_scope),
                tokens.mono(design::T_XS),
                if found.count(state.tree_scope) == 0 {
                    tokens.warn
                } else {
                    tokens.text
                },
            );
            ui.add_space(f32::from(design::S4));
        }
    }
    ui.painter().hline(
        filter.left()..=filter.right(),
        filter.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    egui::ScrollArea::vertical()
        .id_salt("connection_tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
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

            // Pins first, then the rest in profile order. The pin set is the stored order; an id
            // that names no connection (a deleted profile) is skipped, never painted as a ghost.
            let pinned: Vec<&str> = session.pins.iter().map(|s| s.as_str()).collect();
            let order: Vec<usize> = session
                .pins
                .iter()
                .filter_map(|id| session.conns.iter().position(|c| &c.cfg.id == id))
                .chain(
                    (0..session.conns.len())
                        .filter(|i| !pinned.contains(&session.conns[*i].cfg.id.as_str())),
                )
                .collect();
            for index in order {
                let conn = &session.conns[index];
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
                // The profile's palette colour, finally painted: a filled dot before the name when
                // the field is set. The field has been carried since the profile editor shipped.
                if !conn.cfg.color.is_empty() {
                    let (color_ok, color) = {
                        let c = egui::Color32::from_hex(&conn.cfg.color);
                        (c.is_ok(), c.unwrap_or(tokens.text_faint))
                    };
                    if color_ok {
                        ui.painter().circle_filled(
                            egui::pos2(rect.left() + 18.0, rect.center().y),
                            4.0,
                            color,
                        );
                    }
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
                        tokens.font(design::T_XS),
                        tokens.text_faint,
                    );
                }
                ui.painter().text(
                    egui::pos2(rect.right() - 12.0, rect.center().y + 10.0),
                    Align2::RIGHT_CENTER,
                    format!("{tables} tables"),
                    tokens.font(design::T_XS),
                    tokens.text_faint,
                );

                if response.clicked() {
                    action = TreeAction::SelectConnection(index);
                    if !expanded {
                        state.tree_expanded.insert(key.clone());
                        expanded = true;
                    }
                    // One click is enough: a closed connection connects on the first click — the
                    // old double-click gate made the common path two clicks for no reason.
                    if !conn.is_open() {
                        action = TreeAction::OpenConnection(index);
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
                            ui.add_space(design::INDENT_STEP);
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
                            if state.tree_scope == Scope::Connections {
                                continue;
                            }
                            let schema_key = format!("schema:{index}:{}", schema.name);
                            let mut schema_open = state.tree_expanded.contains(&schema_key);
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), design::TREE_H),
                                Sense::click(),
                            );
                            if response.hovered() {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::ZERO, tokens.hover);
                            }
                            ui.painter().text(
                                egui::pos2(
                                    rect.left() + design::INDENT_STEP + 6.0,
                                    rect.center().y,
                                ),
                                Align2::LEFT_CENTER,
                                if schema_open { "▾" } else { "▸" },
                                tokens.font(design::T_XS),
                                tokens.text_dim,
                            );
                            ui.painter().text(
                                egui::pos2(
                                    rect.left() + design::INDENT_STEP + 22.0,
                                    rect.center().y,
                                ),
                                Align2::LEFT_CENTER,
                                &schema.name,
                                tokens.font(design::T_SM),
                                tokens.text,
                            );
                            ui.painter().text(
                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                format!("{}", schema.tables.len()),
                                tokens.font(design::T_XS),
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
                                if !haystack(&needle, &table.name)
                                    && !haystack(&needle, &schema.name)
                                {
                                    continue;
                                }
                                if !state.tree_scope.shows(table.kind) {
                                    continue;
                                }
                                let reference = TableRef {
                                    schema: table.schema.clone(),
                                    table: table.name.clone(),
                                };
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), design::TREE_H),
                                    Sense::click(),
                                );
                                // The design marks the selected row with `--selection`; `.row.sel`
                                // is declared after `.row:hover`, so a hovered selected row keeps the
                                // selection ink rather than turning into a hover one.
                                let selected = state.inspector.selected.as_ref().is_some_and(
                                    |(conn_id, chosen)| {
                                        *conn_id == conn.cfg.id && *chosen == reference
                                    },
                                );
                                if selected {
                                    ui.painter().rect_filled(
                                        rect,
                                        CornerRadius::ZERO,
                                        tokens.selection,
                                    );
                                } else if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        CornerRadius::ZERO,
                                        tokens.hover,
                                    );
                                }
                                let is_view = is_view(table.kind);
                                let badge = egui::Rect::from_center_size(
                                    egui::pos2(
                                        rect.left() + design::INDENT_STEP + 34.0,
                                        rect.center().y,
                                    ),
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
                                // The design's `.rbadge` carries a strong border, and it earns its
                                // keep here: the view fill is the row's own hover colour, so without
                                // the outline a hovered row would swallow the badge's shape.
                                if is_view {
                                    ui.painter().rect_stroke(
                                        badge,
                                        CornerRadius::same(3),
                                        Stroke::new(1.0, tokens.stroke_strong),
                                        StrokeKind::Inside,
                                    );
                                }
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
                                    egui::pos2(
                                        rect.left() + design::INDENT_STEP + 48.0,
                                        rect.center().y,
                                    ),
                                    Align2::LEFT_CENTER,
                                    crate::table::clip(&table.name, 30),
                                    tokens.font(design::T_SM),
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
                                // A click selects: the inspector is a read, not a navigation. A
                                // double click opens the object, as the context menu's first
                                // entry does.
                                if response.clicked() {
                                    action = TreeAction::SelectTable(index, reference.clone());
                                }
                                if response.double_clicked() {
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
                                        if ui.button("Import into this table…").clicked() {
                                            let mut wizard =
                                                crate::app::ImportWizard::new(Some(index));
                                            wizard.table = reference.key();
                                            state.import_wizard = Some(wizard);
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
                            ui.add_space(design::INDENT_STEP + 20.0);
                            ui.label(RichText::new("not connected").size(11.0).color(tokens.warn));
                            if design::ghost_button(ui, tokens, "Connect", true).clicked() {
                                action = TreeAction::OpenConnection(index);
                            }
                        });
                    }
                    (Some(_), None) => {
                        ui.horizontal(|ui| {
                            ui.add_space(design::INDENT_STEP + 20.0);
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
    if let Err(e) = nook_engine::cmd::bind_grid_table(session, tab, reference) {
        session.log(nook_engine::Level::Error, e.to_string());
        return;
    }
    if let Err(e) = nook_engine::cmd::load_grid(session, tab) {
        session.log(nook_engine::Level::Error, e.to_string());
    }
}

/// Open a design tab for a table: the tab carries the catalog copy when the connection has one, so
/// the designer buttons act on what the user sees. Used by the tree's Design action and by the
/// data grid's no-primary-key banner, which routes the user here to give the table a key.
pub fn open_design_tab(session: &mut Session, index: usize, reference: TableRef) {
    let table = session
        .conns
        .get(index)
        .and_then(|c| c.catalog.as_ref())
        .and_then(|cat| cat.find_table(reference.schema.as_deref(), &reference.table))
        .cloned();
    let tab = session.add_tab(TabKind::Design, reference.key(), Some(index));
    if let Some(tab_ref) = session.tabs.get_mut(tab) {
        tab_ref.table = table;
        tab_ref.grid.table = Some(reference);
    }
}

/// Colour for a cell value type, used by the results view.
pub fn value_color(value: &nook_core::Value, tokens: &Tokens) -> Color32 {
    match value {
        nook_core::Value::Null => tokens.text_dim,
        nook_core::Value::Int(_) | nook_core::Value::UInt(_) | nook_core::Value::Float(_) => {
            tokens.accent
        }
        nook_core::Value::Bool(_) => tokens.warn,
        _ => tokens.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nook_model::TableKind;

    #[test]
    fn a_scope_admits_exactly_the_kinds_it_names() {
        assert!(Scope::All.shows(TableKind::Table));
        assert!(Scope::All.shows(TableKind::View));
        assert!(!Scope::Connections.shows(TableKind::Table));
        assert!(!Scope::Connections.shows(TableKind::View));
        assert!(Scope::Tables.shows(TableKind::Table));
        assert!(!Scope::Tables.shows(TableKind::View));
        assert!(!Scope::Tables.shows(TableKind::MatView));
        assert!(Scope::Views.shows(TableKind::View));
        assert!(Scope::Views.shows(TableKind::MatView));
        assert!(!Scope::Views.shows(TableKind::Table));
    }

    /// One click on a disconnected connection card must ask to open it — the fix for "connect
    /// needs two clicks". An already-open card keeps selecting instead of re-connecting.
    #[test]
    fn a_single_click_on_a_closed_connection_opens_it() {
        let mut session = Session::new();
        session
            .conns
            .push(nook_engine::Conn::new(nook_core::ConnectionConfig::new(
                "mem",
                nook_core::DriverKind::Sqlite,
            )));
        let mut state = UiState::default();
        let tokens = Tokens::of("dark");
        let ctx = egui::Context::default();
        tokens.apply(&ctx);
        let screen = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(300.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut action = TreeAction::None;
        let mut frame = |events: Vec<egui::Event>| {
            ctx.run_ui(screen(events), |ui| {
                action = show(ui, &session, &mut state, &tokens);
            })
            .drop_without_applying_deltas()
        };
        frame(Vec::new());
        // The first connection card sits 46px under the header + 98px filter block; click its middle.
        let card = egui::pos2(150.0, HEADER_H + 98.0 + 23.0);
        frame(vec![
            egui::Event::PointerMoved(card),
            egui::Event::PointerButton {
                pos: card,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: card,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        assert_eq!(
            action,
            TreeAction::OpenConnection(0),
            "one click on a closed card is the connect gesture"
        );
    }

    #[test]
    fn the_match_line_counts_what_its_scope_lists() {
        let found = Found {
            connections: 2,
            tables: 3,
            views: 1,
        };
        assert_eq!(found.count(Scope::All), 4);
        assert_eq!(found.count(Scope::Connections), 2);
        assert_eq!(found.count(Scope::Tables), 3);
        assert_eq!(found.count(Scope::Views), 1);
        assert_eq!(found.line(Scope::All), "4 objects");
        assert_eq!(found.line(Scope::Tables), "3 matches");
        assert_eq!(found.line(Scope::Views), "1 match");
        assert_eq!(Found::default().line(Scope::All), "0 objects");
        assert_eq!(Found::default().line(Scope::Views), "0 matches");
    }
}
