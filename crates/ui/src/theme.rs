//! Design tokens and the small widget helpers the rest of the UI builds from.
//! No screen in `crates/ui` invents a colour, a radius, a spacing value or a font size: it asks here.

use egui::{
    Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};

/// Spacing scale — the eight steps the design defines. Nothing in the UI invents a value between
/// them; `--s1..--s8` in the design reference are these numbers.
pub const S1: i8 = 2;
pub const S2: i8 = 4;
pub const S3: i8 = 6;
pub const S4: i8 = 8;
pub const S5: i8 = 12;
pub const S6: i8 = 16;
pub const S7: i8 = 20;
pub const S8: i8 = 24;

/// Layout heights. A row is 26px compact; a table head 28; a tree row 32; a control 28.
pub const ROW_H: f32 = 26.0;
pub const ROW_COMFORT_H: f32 = 30.0;
pub const HEAD_H: f32 = 28.0;
pub const TREE_H: f32 = 32.0;
pub const BUTTON_H: f32 = 28.0;
/// The top bar's bordered controls, as the design's `.iconbtn` and `.ws` are 26.
pub const ICON_BUTTON_H: f32 = 26.0;
/// The box every bar control gives its 14-unit glyph.
pub const GLYPH_BOX: f32 = 14.0;
pub const SEG_H: f32 = 26.0;
pub const SEG_ITEM_H: f32 = 24.0;
pub const CHIP_H: f32 = 24.0;
pub const APP_BAR_H: f32 = 40.0;
pub const TAB_H: f32 = 34.0;
/// The tab bar's own height: the design writes `calc(--h-tabs + 1px)`, so the bar is one pixel taller
/// than its tabs and the active tab's bottom edge covers that row.
pub const TAB_BAR_H: f32 = TAB_H + 1.0;
pub const TOOLBAR_H: f32 = 40.0;
pub const STATUS_H: f32 = 26.0;

/// Panel widths and the small geometry the shell and the lists share.
pub const TREE_W: f32 = 300.0;
pub const INDENT_STEP: f32 = 14.0;
pub const INDICATOR_W: f32 = 2.0;
/// Activity rail: a 44px column of 32px workspace buttons.
pub const RAIL_W: f32 = 44.0;
pub const RAIL_BUTTON: f32 = 32.0;
/// A tab is never narrower than 96 nor wider than 260, and its close button is 24px.
pub const TAB_MIN_W: f32 = 96.0;
pub const TAB_MAX_W: f32 = 260.0;
pub const TAB_CLOSE: f32 = 24.0;
/// The tab bar's control cluster: `.tabctl` is a full-height left border, 8px of padding, its 24px
/// button and 8px more, and the design leaves the bar no right padding, so the cluster's right edge
/// is the bar's right edge.
pub const TAB_CTL_W: f32 = 1.0 + S4 as f32 + TAB_CLOSE + S4 as f32;

/// Corner radii: 4 for controls, 6 for containers, a pill for chips.
pub const R_CTL: u8 = 4;
pub const R_BOX: u8 = 6;
/// `--r-chip: 999px`. epaint clamps a corner radius to half the rect's short side, so this renders
/// as a true pill at any control height and as a square corner on a hairline.
pub const R_CHIP: u8 = 255;

/// Type scale.
pub const T_XS: f32 = 11.0;
pub const T_SM: f32 = 12.0;
pub const T_MD: f32 = 13.0;
pub const T_LG: f32 = 14.0;
pub const T_XL: f32 = 18.0;
pub const T_MONO: f32 = 12.0;

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
    pub accent_line: Color32,
    pub accent_text: Color32,

    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub info: Color32,
    /// Soft fills for a semantic state: a chip, a badge, a selected-but-modified cell.
    pub ok_fill: Color32,
    pub warn_fill: Color32,
    pub error_fill: Color32,

    pub grid_header: Color32,
    pub grid_row_alt: Color32,
    pub grid_line: Color32,
    pub selection: Color32,
    pub dirty: Color32,
    pub dirty_text: Color32,
    pub hover: Color32,
    /// Dimming layer under a modal.
    pub scrim: Color32,

    pub chart: [Color32; 6],
}

impl Tokens {
    pub fn of(theme: &str) -> Tokens {
        match theme {
            "light" => Tokens {
                dark: false,
                radius: R_BOX,
                background: Color32::from_rgb(0xf4, 0xf6, 0xf9),
                panel: Color32::from_rgb(0xff, 0xff, 0xff),
                panel_alt: Color32::from_rgb(0xee, 0xf2, 0xf7),
                panel_high: Color32::from_rgb(0xe3, 0xe9, 0xf1),
                stroke: Color32::from_rgb(0xd3, 0xdb, 0xe6),
                stroke_strong: Color32::from_rgb(0xb9, 0xc4, 0xd3),
                text: Color32::from_rgb(0x13, 0x1a, 0x24),
                text_dim: Color32::from_rgb(0x4d, 0x58, 0x66),
                text_faint: Color32::from_rgb(0x5a, 0x65, 0x75),
                accent: Color32::from_rgb(0x1d, 0x4e, 0xd8),
                accent_dim: Color32::from_rgb(0xe2, 0xea, 0xfd),
                accent_line: Color32::from_rgb(0xb9, 0xcd, 0xf8),
                accent_text: Color32::WHITE,
                ok: Color32::from_rgb(0x0f, 0x7a, 0x49),
                warn: Color32::from_rgb(0x8a, 0x5a, 0x00),
                error: Color32::from_rgb(0xb3, 0x2a, 0x2a),
                info: Color32::from_rgb(0x14, 0x63, 0x7f),
                ok_fill: Color32::from_rgb(0xe6, 0xf5, 0xee),
                warn_fill: Color32::from_rgb(0xfd, 0xf3, 0xdd),
                error_fill: Color32::from_rgb(0xfb, 0xea, 0xea),
                grid_header: Color32::from_rgb(0xee, 0xf2, 0xf7),
                grid_row_alt: Color32::from_rgb(0xf7, 0xf9, 0xfc),
                grid_line: Color32::from_rgb(0xe7, 0xec, 0xf3),
                selection: Color32::from_rgb(0xd6, 0xe4, 0xff),
                dirty: Color32::from_rgb(0xff, 0xf2, 0xce),
                dirty_text: Color32::from_rgb(0x7a, 0x51, 0x00),
                hover: Color32::from_rgb(0xe3, 0xe9, 0xf1),
                // rgba(19,26,36,.35) — the design's light scrim.
                scrim: Color32::from_rgba_unmultiplied(0x13, 0x1a, 0x24, 89),
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
                radius: R_BOX,
                background: Color32::from_rgb(0x1f, 0x1f, 0x1f),
                panel: Color32::from_rgb(0x18, 0x18, 0x18),
                panel_alt: Color32::from_rgb(0x20, 0x20, 0x20),
                panel_high: Color32::from_rgb(0x2b, 0x2b, 0x2b),
                stroke: Color32::from_rgb(0x2f, 0x2f, 0x2f),
                stroke_strong: Color32::from_rgb(0x3c, 0x3c, 0x3c),
                text: Color32::from_rgb(0xcc, 0xcc, 0xcc),
                text_dim: Color32::from_rgb(0x9d, 0x9d, 0x9d),
                text_faint: Color32::from_rgb(0x96, 0x96, 0x96),
                accent: Color32::from_rgb(0x4d, 0xaa, 0xfc),
                accent_dim: Color32::from_rgb(0x04, 0x2a, 0x45),
                accent_line: Color32::from_rgb(0x00, 0x78, 0xd4),
                accent_text: Color32::from_rgb(0x06, 0x26, 0x3f),
                ok: Color32::from_rgb(0x4e, 0xc9, 0xb0),
                warn: Color32::from_rgb(0xcc, 0xa7, 0x00),
                error: Color32::from_rgb(0xf2, 0x6b, 0x6b),
                info: Color32::from_rgb(0x37, 0x94, 0xff),
                ok_fill: Color32::from_rgb(0x12, 0x33, 0x2a),
                warn_fill: Color32::from_rgb(0x33, 0x2a, 0x00),
                error_fill: Color32::from_rgb(0x3a, 0x1d, 0x1d),
                grid_header: Color32::from_rgb(0x20, 0x20, 0x20),
                grid_row_alt: Color32::from_rgb(0x24, 0x24, 0x24),
                grid_line: Color32::from_rgb(0x30, 0x30, 0x30),
                selection: Color32::from_rgb(0x17, 0x32, 0x4f),
                dirty: Color32::from_rgb(0x33, 0x2a, 0x00),
                dirty_text: Color32::from_rgb(0xe5, 0xc0, 0x7b),
                hover: Color32::from_rgb(0x2b, 0x2b, 0x2b),
                // rgba(15,15,15,.6) — the design's dark scrim.
                scrim: Color32::from_rgba_unmultiplied(0x0f, 0x0f, 0x0f, 153),
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
            rounding.corner_radius = CornerRadius::same(R_CTL);
        }
        ctx.set_visuals(visuals);

        let theme = ctx.theme();
        let mut style = (*ctx.style_of(theme)).clone();
        for (text_style, size) in [
            (egui::TextStyle::Body, T_MD),
            (egui::TextStyle::Button, T_MD),
            (egui::TextStyle::Small, T_SM),
            (egui::TextStyle::Heading, T_XL),
            (egui::TextStyle::Monospace, T_MONO),
        ] {
            let mut font = text_style.resolve(&style);
            font.size = size;
            style.text_styles.insert(text_style, font);
        }
        style.spacing.item_spacing = Vec2::new(f32::from(S4), f32::from(S4));
        style.spacing.button_padding = Vec2::new(f32::from(S4), f32::from(S1));
        style.spacing.scroll.bar_width = 10.0;
        style.spacing.window_margin = Margin::same(S6);
        style.spacing.menu_margin = Margin::same(S2);
        style.spacing.indent = INDENT_STEP;
        style.spacing.interact_size.y = BUTTON_H;
        ctx.set_style_of(theme, style);
    }
}

/// Frame for a top or bottom bar. The bar paints its own fill and hairline inside `band_h`, so the
/// frame adds nothing at all: with a stroke or a margin here the bar would be taller than its token,
/// which is exactly what the previous frame did.
pub fn bar_frame(tokens: &Tokens) -> Frame {
    Frame::new()
        .fill(tokens.panel)
        .inner_margin(Margin::same(0))
        .outer_margin(Margin::ZERO)
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
        .corner_radius(CornerRadius::same(R_BOX))
        .inner_margin(Margin::same(S5))
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

/// Horizontal band of a fixed height: a fill, content aligned vertically, and — important — a bounded
/// rect, so a band can never consume the panel it sits in the way `horizontal_centered` does inside
/// an unbounded layout. Returns the band rect, so the caller can draw its own hairline inside it.
pub fn band_h(ui: &mut Ui, tokens: &Tokens, height: f32, add: impl FnOnce(&mut Ui)) -> egui::Rect {
    band_fill(ui, height, tokens.panel, S5, egui::Align::Center, add)
}

/// `band_h` with an explicit fill, horizontal inset and alignment: the tab strip sits on `panel_alt`
/// with the design's 8px lead-in and its tabs against the band's top edge, while every other bar uses
/// the panel fill, 12px and a centred row.
pub fn band_fill(
    ui: &mut Ui,
    height: f32,
    fill: Color32,
    inset: i8,
    align: egui::Align,
    add: impl FnOnce(&mut Ui),
) -> egui::Rect {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(inset), rect.top()),
        egui::pos2(rect.right() - f32::from(inset), rect.bottom()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(align)),
    );
    add(&mut child);
    rect
}

/// 1px separator drawn *inside* the bottom edge of a band. A bar that draws its own border outside
/// its rect grows by it, which is how the shell bars ended up one pixel taller than their tokens.
pub fn band_hairline(ui: &Ui, tokens: &Tokens, rect: egui::Rect) {
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
}

/// `band_hairline` for a bar the design gives a `border-top`: the status bar's line sits at the top
/// of its 26px band, not at the bottom where it would also fall on the window's last row.
pub fn band_top_hairline(ui: &Ui, tokens: &Tokens, rect: egui::Rect) {
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.top() + 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
}

/// Toolbar-height band: the shape every view header uses.
pub fn band(ui: &mut Ui, tokens: &Tokens, add: impl FnOnce(&mut Ui)) -> egui::Rect {
    band_h(ui, tokens, TOOLBAR_H, add)
}

/// Filled, high-emphasis button: 28px tall, `--r-ctl`, accent fill.
pub fn primary_button(ui: &mut Ui, tokens: &Tokens, label: &str, enabled: bool) -> Response {
    let text = RichText::new(label).size(T_MD).color(if enabled {
        tokens.accent_text
    } else {
        tokens.text_dim
    });
    let fill = if enabled {
        tokens.accent
    } else {
        tokens.panel_high
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(R_CTL))
            .min_size(Vec2::new(0.0, BUTTON_H)),
    )
}

/// Quiet button for toolbars: nothing painted until the pointer is over it.
pub fn ghost_button(ui: &mut Ui, tokens: &Tokens, label: &str, enabled: bool) -> Response {
    // Disabled has to read as "not now", not as "not drawn".
    let text = RichText::new(label).size(T_MD).color(if enabled {
        tokens.text
    } else {
        tokens.text_dim
    });
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(R_CTL))
            .min_size(Vec2::new(0.0, BUTTON_H)),
    );
    if response.hovered() && enabled {
        let rect = response.rect;
        ui.painter()
            .rect_filled(rect, CornerRadius::same(R_CTL), tokens.hover);
    }
    response
}

/// Segmented control: a 26px bordered container holding 24px items split by hairlines, the selected
/// item on `accent_dim` with a 2px accent indicator along its bottom edge — the shape the design
/// defines for `.seg`. Returns the index that was clicked.
pub fn segmented(ui: &mut Ui, tokens: &Tokens, options: &[(&str, bool)]) -> Option<usize> {
    if options.is_empty() {
        return None;
    }
    let font = tokens.font(T_XS);
    let widths: Vec<f32> = options
        .iter()
        .map(|(label, _)| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
                + 2.0 * f32::from(S4)
        })
        .collect();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(widths.iter().sum::<f32>() + 2.0, SEG_H),
        Sense::hover(),
    );
    let radius = CornerRadius::same(R_CTL);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tokens.panel_alt);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke),
        StrokeKind::Inside,
    );

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(1.0))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 0.0;
    let mut clicked = None;
    for (index, ((label, selected), width)) in options.iter().zip(&widths).enumerate() {
        let (item, response) =
            child.allocate_exact_size(Vec2::new(*width, SEG_ITEM_H), Sense::click());
        // Clip the first and last fill to the container's rounding, the way `overflow: hidden` does.
        let fill_radius = match (index, options.len()) {
            (0, 1) => radius,
            (0, _) => CornerRadius {
                nw: R_CTL,
                sw: R_CTL,
                ..CornerRadius::ZERO
            },
            (last, len) if last == len - 1 => CornerRadius {
                ne: R_CTL,
                se: R_CTL,
                ..CornerRadius::ZERO
            },
            _ => CornerRadius::ZERO,
        };
        let painter = child.painter();
        if *selected {
            painter.rect_filled(item, fill_radius, tokens.accent_dim);
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(item.left(), item.bottom() - INDICATOR_W),
                    item.max,
                ),
                CornerRadius::ZERO,
                tokens.accent,
            );
        } else if response.hovered() {
            painter.rect_filled(item, fill_radius, tokens.hover);
        }
        if index > 0 {
            painter.vline(
                item.left(),
                item.top()..=item.bottom(),
                Stroke::new(1.0, tokens.stroke),
            );
        }
        painter.text(
            item.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if *selected {
                tokens.text
            } else {
                tokens.text_dim
            },
        );
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// Small label in a rounded box: counts, states, versions.
///
/// The chip allocates its own 24px box rather than wrapping a `Frame`: a frame inside a horizontal
/// layout stretches to the row height the parent offers (40px inside a toolbar band), which is how a
/// chip stops being a chip.
///
/// ponytail: the design's chips are 600-weight; egui's default proportional family has no weight
/// axis, so the chip reads by colour, size and shape alone. Add a bold font family if one is ever
/// bundled.
pub fn chip(ui: &mut Ui, tokens: &Tokens, text: &str, colour: Color32) -> Response {
    let font = tokens.font(T_XS);
    let text_width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(text_width + 2.0 * f32::from(S4), CHIP_H),
        Sense::hover(),
    );
    let radius = CornerRadius::same(R_CHIP);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tokens.panel_alt);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        colour,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    response
}

/// Coloured dot used for connection state and message levels.
pub fn dot(ui: &mut Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 9.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, colour);
}

/// Section heading with an optional subtitle.
pub fn section(ui: &mut Ui, tokens: &Tokens, title: &str, subtitle: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(T_MD).color(tokens.text));
        if let Some(subtitle) = subtitle {
            ui.label(RichText::new(subtitle).size(T_XS).color(tokens.text_faint));
        }
    });
}

/// Hand-drawn progress bar: parity meter, monitor size bars, chart bars.
pub fn meter(ui: &mut Ui, tokens: &Tokens, fraction: f32, width: f32, colour: Color32) -> Response {
    let fraction = fraction.clamp(0.0, 1.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(R_CTL), tokens.panel_alt);
    let filled =
        egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height()));
    ui.painter()
        .rect_filled(filled, CornerRadius::same(R_CTL), colour);
    response
}

/// Key cap, e.g. `Enter`, `Esc`.
pub fn keycap(ui: &mut Ui, tokens: &Tokens, text: &str) {
    Frame::new()
        .fill(tokens.panel_alt)
        .stroke(Stroke::new(1.0, tokens.stroke))
        .corner_radius(CornerRadius::same(R_CTL))
        .inner_margin(Margin {
            left: S2,
            right: S2,
            top: S1,
            bottom: S1,
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(T_XS)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.1 relative luminance of an sRGB colour.
    fn luminance(colour: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
    }

    /// WCAG contrast ratio between two opaque colours.
    fn contrast(one: Color32, other: Color32) -> f32 {
        let (one, other) = (luminance(one), luminance(other));
        let (lighter, darker) = if one > other {
            (one, other)
        } else {
            (other, one)
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    /// Every foreground/background pair the interface paints, in both themes, against the floors this
    /// application holds itself to: 4.5:1 for text and for any semantic colour used as text (the
    /// design's contract, including the dim and faint ranks on each surface they sit on); 3.0:1 for
    /// the accent indicator over its soft fill; 1.3:1 for a hairline, because the design's own dark
    /// border measures 1.33:1 on its panel, and below that a border stops reading as a boundary. The
    /// design's named worst cases are the pairs below: dim and faint on `panel_high`, the accent on
    /// `accent_dim`, `accent_text` on `accent`, and each semantic colour on its soft fill.
    #[test]
    fn palettes_clear_their_contrast_floors() {
        for tokens in [Tokens::of("dark"), Tokens::of("light")] {
            let theme = if tokens.dark { "dark" } else { "light" };
            let floors: [(&str, Color32, Color32, f32); 29] = [
                ("text on panel", tokens.text, tokens.panel, 4.5),
                ("text on background", tokens.text, tokens.background, 4.5),
                ("text on panel_alt", tokens.text, tokens.panel_alt, 4.5),
                ("text on panel_high", tokens.text, tokens.panel_high, 4.5),
                ("text on selection", tokens.text, tokens.selection, 4.5),
                ("text on accent_dim", tokens.text, tokens.accent_dim, 4.5),
                (
                    "text on grid_row_alt",
                    tokens.text,
                    tokens.grid_row_alt,
                    4.5,
                ),
                ("text on grid_header", tokens.text, tokens.grid_header, 4.5),
                ("dim on panel", tokens.text_dim, tokens.panel, 4.5),
                ("dim on background", tokens.text_dim, tokens.background, 4.5),
                ("dim on panel_alt", tokens.text_dim, tokens.panel_alt, 4.5),
                ("dim on panel_high", tokens.text_dim, tokens.panel_high, 4.5),
                (
                    "dim on grid_header",
                    tokens.text_dim,
                    tokens.grid_header,
                    4.5,
                ),
                ("faint on panel", tokens.text_faint, tokens.panel, 4.5),
                (
                    "faint on panel_alt",
                    tokens.text_faint,
                    tokens.panel_alt,
                    4.5,
                ),
                (
                    "faint on panel_high",
                    tokens.text_faint,
                    tokens.panel_high,
                    4.5,
                ),
                ("accent on panel", tokens.accent, tokens.panel, 4.5),
                ("accent on panel_alt", tokens.accent, tokens.panel_alt, 4.5),
                (
                    "accent_text on accent",
                    tokens.accent_text,
                    tokens.accent,
                    4.5,
                ),
                ("ok on panel", tokens.ok, tokens.panel, 4.5),
                ("warn on panel", tokens.warn, tokens.panel, 4.5),
                ("error on panel", tokens.error, tokens.panel, 4.5),
                ("info on panel", tokens.info, tokens.panel, 4.5),
                ("ok on ok_fill", tokens.ok, tokens.ok_fill, 4.5),
                ("warn on warn_fill", tokens.warn, tokens.warn_fill, 4.5),
                ("error on error_fill", tokens.error, tokens.error_fill, 4.5),
                ("dirty_text on dirty", tokens.dirty_text, tokens.dirty, 4.5),
                (
                    "indicator on accent_dim",
                    tokens.accent,
                    tokens.accent_dim,
                    3.0,
                ),
                ("hairline on panel", tokens.stroke, tokens.panel, 1.3),
            ];
            for (pair, foreground, background, floor) in floors {
                let ratio = contrast(foreground, background);
                assert!(
                    ratio >= floor,
                    "{theme}: {pair} is {ratio:.2}:1, needs at least {floor}:1"
                );
            }
        }
    }

    /// The scrim has to dim what it covers, in both themes. `Color32` stores premultiplied bytes, so
    /// the check composites those bytes over the panel using the token's alpha instead of treating a
    /// translucent colour as opaque — the opaque comparison passes for any dark value, including one
    /// too faint to dim anything.
    #[test]
    fn scrim_dims_the_surface_below_it() {
        for tokens in [Tokens::of("dark"), Tokens::of("light")] {
            let theme = if tokens.dark { "dark" } else { "light" };
            let alpha = f32::from(tokens.scrim.a()) / 255.0;
            let over = |over: u8, under: u8| {
                (f32::from(over) + f32::from(under) * (1.0 - alpha)).round() as u8
            };
            let dimmed = Color32::from_rgb(
                over(tokens.scrim.r(), tokens.panel.r()),
                over(tokens.scrim.g(), tokens.panel.g()),
                over(tokens.scrim.b(), tokens.panel.b()),
            );
            assert!(
                luminance(dimmed) < luminance(tokens.panel) * 0.9,
                "{theme}: the scrim leaves {:.0}% of the panel's luminance, which is not dimming",
                luminance(dimmed) / luminance(tokens.panel) * 100.0
            );
        }
    }
}
