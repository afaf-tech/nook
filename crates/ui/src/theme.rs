//! Design tokens and the small widget helpers the rest of the UI builds from.
//! No screen in `crates/ui` invents a colour, a radius, a spacing value or a font size: it asks here.

use egui::{
    Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};

/// Spacing scale.
pub const S1: i8 = 4;
pub const S2: i8 = 8;
pub const S3: i8 = 12;
pub const S4: i8 = 16;

/// Layout heights.
pub const ROW_H: f32 = 24.0;
pub const APP_BAR_H: f32 = 38.0;
pub const TAB_H: f32 = 36.0;
pub const TOOLBAR_H: f32 = 42.0;
pub const STATUS_H: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tokens {
    pub dark: bool,
    pub radius: u8,

    pub background: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub panel_high: Color32,
    pub stroke: Color32,
    pub stroke_strong: Color32,

    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,

    pub accent: Color32,
    pub accent_dim: Color32,
    pub accent_text: Color32,

    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub info: Color32,

    pub grid_header: Color32,
    pub grid_row_alt: Color32,
    pub grid_line: Color32,
    pub selection: Color32,
    pub dirty: Color32,
    pub dirty_text: Color32,
    pub hover: Color32,

    pub chart: [Color32; 6],
}

impl Tokens {
    pub fn of(theme: &str) -> Tokens {
        match theme {
            "light" => Tokens {
                dark: false,
                radius: 6,
                background: Color32::from_rgb(0xf5, 0xf7, 0xfa),
                panel: Color32::from_rgb(0xff, 0xff, 0xff),
                panel_alt: Color32::from_rgb(0xee, 0xf2, 0xf7),
                panel_high: Color32::from_rgb(0xe4, 0xea, 0xf3),
                stroke: Color32::from_rgb(0xc4, 0xce, 0xdb),
                stroke_strong: Color32::from_rgb(0xae, 0xba, 0xca),
                text: Color32::from_rgb(0x15, 0x1c, 0x27),
                text_dim: Color32::from_rgb(0x4a, 0x55, 0x67),
                text_faint: Color32::from_rgb(0x5b, 0x66, 0x75),
                accent: Color32::from_rgb(0x25, 0x63, 0xeb),
                accent_dim: Color32::from_rgb(0xdd, 0xe7, 0xfd),
                accent_text: Color32::WHITE,
                ok: Color32::from_rgb(0x10, 0x84, 0x4d),
                warn: Color32::from_rgb(0xa8, 0x62, 0x00),
                error: Color32::from_rgb(0xc0, 0x2b, 0x2b),
                info: Color32::from_rgb(0x1c, 0x6b, 0x8f),
                grid_header: Color32::from_rgb(0xec, 0xf1, 0xf8),
                grid_row_alt: Color32::from_rgb(0xf8, 0xfa, 0xfc),
                grid_line: Color32::from_rgb(0xe7, 0xec, 0xf3),
                selection: Color32::from_rgb(0xd7, 0xe5, 0xff),
                dirty: Color32::from_rgb(0xff, 0xf1, 0xcc),
                dirty_text: Color32::from_rgb(0x7a, 0x51, 0x00),
                hover: Color32::from_rgb(0xea, 0xef, 0xf7),
                chart: [
                    Color32::from_rgb(0x25, 0x63, 0xeb),
                    Color32::from_rgb(0x0e, 0x9f, 0x6e),
                    Color32::from_rgb(0xd9, 0x7a, 0x0b),
                    Color32::from_rgb(0xcf, 0x3f, 0x3f),
                    Color32::from_rgb(0x7c, 0x5c, 0xd6),
                    Color32::from_rgb(0x1c, 0x8b, 0xa8),
                ],
            },
            _ => Tokens {
                dark: true,
                radius: 6,
                background: Color32::from_rgb(0x0d, 0x11, 0x17),
                panel: Color32::from_rgb(0x14, 0x19, 0x22),
                panel_alt: Color32::from_rgb(0x1b, 0x22, 0x2d),
                panel_high: Color32::from_rgb(0x23, 0x2c, 0x39),
                stroke: Color32::from_rgb(0x26, 0x30, 0x3e),
                stroke_strong: Color32::from_rgb(0x39, 0x47, 0x5a),
                text: Color32::from_rgb(0xe8, 0xee, 0xf7),
                text_dim: Color32::from_rgb(0x9a, 0xaa, 0xc0),
                text_faint: Color32::from_rgb(0x7f, 0x8f, 0xa6),
                accent: Color32::from_rgb(0x63, 0x94, 0xff),
                accent_dim: Color32::from_rgb(0x1e, 0x2f, 0x52),
                accent_text: Color32::from_rgb(0x08, 0x0d, 0x16),
                ok: Color32::from_rgb(0x4c, 0xd1, 0x8a),
                warn: Color32::from_rgb(0xe8, 0xb9, 0x4b),
                error: Color32::from_rgb(0xf2, 0x7a, 0x7a),
                info: Color32::from_rgb(0x63, 0xc2, 0xe8),
                grid_header: Color32::from_rgb(0x1c, 0x24, 0x30),
                grid_row_alt: Color32::from_rgb(0x11, 0x16, 0x1e),
                grid_line: Color32::from_rgb(0x1e, 0x26, 0x32),
                selection: Color32::from_rgb(0x23, 0x3a, 0x66),
                dirty: Color32::from_rgb(0x3b, 0x31, 0x14),
                dirty_text: Color32::from_rgb(0xf0, 0xc9, 0x6b),
                hover: Color32::from_rgb(0x1a, 0x22, 0x2e),
                chart: [
                    Color32::from_rgb(0x63, 0x94, 0xff),
                    Color32::from_rgb(0x4c, 0xd1, 0x8a),
                    Color32::from_rgb(0xe8, 0xb9, 0x4b),
                    Color32::from_rgb(0xf2, 0x7a, 0x7a),
                    Color32::from_rgb(0xa7, 0x8b, 0xf0),
                    Color32::from_rgb(0x63, 0xc2, 0xe8),
                ],
            },
        }
    }

    pub fn level_color(&self, level: datacraft_engine::Level) -> Color32 {
        match level {
            datacraft_engine::Level::Info => self.text_dim,
            datacraft_engine::Level::Warn => self.warn,
            datacraft_engine::Level::Error => self.error,
            datacraft_engine::Level::Success => self.ok,
        }
    }

    pub fn corner(&self) -> CornerRadius {
        CornerRadius::same(self.radius)
    }

    pub fn font(&self, size: f32) -> FontId {
        FontId::proportional(size)
    }

    pub fn mono(&self, size: f32) -> FontId {
        FontId::monospace(size)
    }

    /// Colours, spacing, rounding and text styles for the whole app.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.window_stroke = Stroke::new(1.0, self.stroke);
        visuals.extreme_bg_color = self.background;
        visuals.faint_bg_color = self.panel_alt;
        visuals.override_text_color = Some(self.text);
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.selection;
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.widgets.noninteractive.bg_fill = self.panel;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.stroke);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.inactive.bg_fill = self.panel_alt;
        visuals.widgets.inactive.weak_bg_fill = self.panel_alt;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.stroke);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.hovered.bg_fill = self.panel_high;
        visuals.widgets.hovered.weak_bg_fill = self.panel_high;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.stroke_strong);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.active.bg_fill = self.accent_dim;
        visuals.widgets.active.weak_bg_fill = self.accent_dim;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.text);
        for rounding in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            rounding.corner_radius = CornerRadius::same(5);
        }
        ctx.set_visuals(visuals);

        let theme = ctx.theme();
        let mut style = (*ctx.style_of(theme)).clone();
        for (text_style, size) in [
            (egui::TextStyle::Body, 13.0),
            (egui::TextStyle::Button, 13.0),
            (egui::TextStyle::Small, 11.5),
            (egui::TextStyle::Heading, 17.0),
            (egui::TextStyle::Monospace, 12.5),
        ] {
            let mut font = text_style.resolve(&style);
            font.size = size;
            style.text_styles.insert(text_style, font);
        }
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 3.0);
        style.spacing.scroll.bar_width = 10.0;
        style.spacing.window_margin = Margin::same(12);
        style.spacing.menu_margin = Margin::same(4);
        style.spacing.indent = 16.0;
        style.spacing.interact_size.y = 24.0;
        ctx.set_style_of(theme, style);
    }
}

/// Frame for a top panel: panel fill, comfortable horizontal padding.
pub fn top_panel_frame(tokens: &Tokens, _height: f32) -> Frame {
    Frame::new()
        .fill(tokens.panel)
        .inner_margin(Margin {
            left: S3,
            right: S3,
            top: 0,
            bottom: 0,
        })
        .outer_margin(Margin::ZERO)
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::ZERO)
}

/// Frame for a side or bottom panel.
pub fn side_panel_frame(tokens: &Tokens) -> Frame {
    Frame::new()
        .fill(tokens.panel)
        .inner_margin(Margin::same(0))
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::ZERO)
}

/// Card container: welcome page, monitor tiles, job list.
pub fn card_frame(tokens: &Tokens) -> Frame {
    Frame::new()
        .fill(tokens.panel_alt)
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(S3))
}

/// Vertical separator between toolbar clusters.
pub fn divider(ui: &mut Ui, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(11.0, 18.0), Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        rect.top()..=rect.bottom(),
        Stroke::new(1.0, tokens.stroke),
    );
}

/// Horizontal band of a fixed height: panel fill, content centred vertically, and — important — a
/// bounded rect, so a band can never consume the panel it sits in the way `horizontal_centered`
/// does inside an unbounded layout. Returns the band rect so the caller can draw its own hairline.
pub fn band_h(ui: &mut Ui, tokens: &Tokens, height: f32, add: impl FnOnce(&mut Ui)) -> egui::Rect {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, tokens.panel);
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(S3), rect.top()),
        egui::pos2(rect.right() - f32::from(S3), rect.bottom()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    add(&mut child);
    rect
}

/// Toolbar-height band: the shape every view header uses.
pub fn band(ui: &mut Ui, tokens: &Tokens, add: impl FnOnce(&mut Ui)) -> egui::Rect {
    band_h(ui, tokens, TOOLBAR_H, add)
}

/// Filled, high-emphasis button.
pub fn primary_button(ui: &mut Ui, tokens: &Tokens, label: &str, enabled: bool) -> Response {
    let text = RichText::new(label).size(12.5).color(if enabled {
        tokens.accent_text
    } else {
        tokens.text_dim
    });
    let fill = if enabled {
        tokens.accent
    } else {
        tokens.panel_alt
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(0.0, 24.0)),
    )
}

/// Quiet button for toolbars: nothing painted until the pointer is over it.
pub fn ghost_button(ui: &mut Ui, tokens: &Tokens, label: &str, enabled: bool) -> Response {
    // Disabled has to read as "not now", not as "not drawn".
    let text = RichText::new(label).size(12.5).color(if enabled {
        tokens.text
    } else {
        tokens.text_dim
    });
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(0.0, 24.0)),
    );
    if response.hovered() && enabled {
        let rect = response.rect;
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), tokens.panel_high);
    }
    response
}

/// Segmented control: one rounded container, small mutually exclusive buttons inside.
/// Returns the index that was clicked.
pub fn segmented(ui: &mut Ui, tokens: &Tokens, options: &[(&str, bool)]) -> Option<usize> {
    let mut clicked = None;
    Frame::new()
        .fill(tokens.panel_alt)
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (index, (label, selected)) in options.iter().enumerate() {
                    let text = RichText::new(*label).size(12.0).color(if *selected {
                        tokens.text
                    } else {
                        tokens.text_dim
                    });
                    let response = ui.add(
                        egui::Button::new(text)
                            .fill(if *selected {
                                tokens.panel_high
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(4))
                            .min_size(Vec2::new(0.0, 22.0)),
                    );
                    if response.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    clicked
}

/// Small label in a rounded box: counts, states, versions.
pub fn chip(ui: &mut Ui, tokens: &Tokens, text: &str, colour: Color32) -> Response {
    let label = RichText::new(text).size(11.5).color(colour);
    Frame::new()
        .fill(tokens.panel_alt)
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin {
            left: 7,
            right: 7,
            top: 2,
            bottom: 2,
        })
        .show(ui, |ui| ui.label(label))
        .response
}

/// Coloured dot used for connection state and message levels.
pub fn dot(ui: &mut Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 9.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, colour);
}

/// Section heading with an optional subtitle.
pub fn section(ui: &mut Ui, tokens: &Tokens, title: &str, subtitle: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(13.5).color(tokens.text));
        if let Some(subtitle) = subtitle {
            ui.label(RichText::new(subtitle).size(11.5).color(tokens.text_faint));
        }
    });
}

/// Hand-drawn progress bar: parity meter, monitor size bars, chart bars.
pub fn meter(ui: &mut Ui, tokens: &Tokens, fraction: f32, width: f32, colour: Color32) -> Response {
    let fraction = fraction.clamp(0.0, 1.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), tokens.panel_alt);
    let filled =
        egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height()));
    ui.painter()
        .rect_filled(filled, CornerRadius::same(4), colour);
    response
}

/// Key cap, e.g. `Enter`, `Esc`.
pub fn keycap(ui: &mut Ui, tokens: &Tokens, text: &str) {
    Frame::new()
        .fill(tokens.panel_alt)
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin {
            left: 5,
            right: 5,
            top: 1,
            bottom: 1,
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(10.5)
                    .monospace()
                    .color(tokens.text_dim),
            )
        });
}

/// Filled rectangle with a hairline border.
pub fn outlined(ui: &Ui, rect: egui::Rect, tokens: &Tokens, radius: CornerRadius, fill: Color32) {
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke),
        StrokeKind::Inside,
    );
}
