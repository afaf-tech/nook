//! Design tokens and the small widget helpers the rest of the UI builds from.
//! No screen in `crates/ui` invents a colour, a radius, a spacing value or a font size: it asks here.

use egui::{
    Color32, CornerRadius, FontId, Frame, Margin, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2,
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
/// The 12px glyph the design's `.sm` icons use inside a 26px control.
pub const GLYPH_SM: f32 = 12.0;
/// The design's `.menu`: a 28px label button with 6px of padding and no chrome of its own.
pub const MENU_H: f32 = 28.0;
/// The design's `.keycap`: an 11px line, 4px of padding and a 1px border.
pub const KEYCAP_H: f32 = 21.0;
/// The design's `.engine` tag: the same 11px line with 4px of padding and a 1px border.
pub const TAG_H: f32 = 21.0;
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

/// The query editor's own geometry: `.ed` is the design's 320 — sixteen 20px rows in a 52px gutter —
/// its `.tb` buttons are 26, its statement strip's chips and the completion popup's items are 20.
pub const EDITOR_H: f32 = 320.0;
pub const GUTTER_W: f32 = 52.0;
pub const EDITOR_ROW_H: f32 = 20.0;
pub const STMT_H: f32 = 20.0;
pub const STMT_BAND_H: f32 = 56.0;
pub const TOOLBAR_BTN_H: f32 = 26.0;
pub const POP_W: f32 = 456.0;
pub const POP_HD_H: f32 = 24.0;
pub const POP_FT_H: f32 = 24.0;
pub const POP_ITEM_H: f32 = 20.0;

/// The results register's own geometry: `.rtabs` is the tab strip's `34 + 1`, its `.note` pill and the
/// empty block are 20 and 196, its JSON gutter is 48, and `.eband` is padded with `--s5` on all four
/// sides.
pub const REG_TABS_H: f32 = TAB_H + 1.0;
pub const NOTE_H: f32 = 20.0;
pub const EMPTY_BLOCK_H: f32 = 196.0;
pub const JSON_GUTTER_W: f32 = 48.0;
pub const REG_ERR_PAD: i8 = S5;

/// Panel widths and the small geometry the shell and the lists share.
pub const TREE_W: f32 = 300.0;
/// The object inspector, from the design's `--w-inspector`.
pub const INSPECTOR_W: f32 = 320.0;
/// Where the tree's, the grid's and the inspector's pane heads all end, from the design's
/// `--h-panehead` (40 + 24 + 40 + 28). The three panes start their bodies on the same line only if
/// each one's bands add up to this, so it is one number rather than three sums.
pub const PANE_HEAD_H: f32 = 132.0;
pub const INDENT_STEP: f32 = 14.0;
pub const INDICATOR_W: f32 = 2.0;

/// The grid's own band heights. `.fchips` is 32 — the design's 18px chip with 7px of air above and
/// below — and `.stalestrip` is the status bar's 26. `.pkbadge` and `.taglite` are 16px boxes.
pub const FCHIP_H: f32 = 18.0;
const FCHIP_AIR: f32 = 7.0;
pub const FCHIPS_H: f32 = FCHIP_H + 2.0 * FCHIP_AIR;
pub const STALE_STRIP_H: f32 = STATUS_H;
pub const MICRO_TAG_H: f32 = 16.0;
/// `.errband` and `.robanner` keep 10px of air above and below their two lines.
pub const NOTICE_PAD_V: f32 = 10.0;
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
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 16.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
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

/// The design's `.btn.ba`: a 24px button outlined in `accent_line` with `accent` ink, which fills
/// with `accent_dim` under the pointer. The quiet half of the pair whose other half is
/// [`primary_button`], and the one the panes use for "open the thing this panel is describing".
pub fn accent_button(ui: &mut Ui, tokens: &Tokens, label: &str, enabled: bool) -> Response {
    let text = RichText::new(label).size(T_SM).color(if enabled {
        tokens.accent
    } else {
        tokens.text_dim
    });
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(
                1.0,
                if enabled {
                    tokens.accent_line
                } else {
                    tokens.stroke
                },
            ))
            .corner_radius(CornerRadius::same(R_CTL))
            .min_size(Vec2::new(0.0, CHIP_H)),
    );
    if response.hovered() && enabled {
        let rect = response.rect;
        ui.painter()
            .rect_filled(rect, CornerRadius::same(R_CTL), tokens.accent_dim);
    }
    response
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
        .map(|(label, _)| segment_item_width(ui, tokens, label))
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

/// The item rounding inside a pane's segmented control: the design's 3px, one less than `R_CTL`.
const SEG_ITEM_RADIUS: u8 = 3;

/// The room the items of a pane segmented control share: the container less its 1px border and its
/// inset, less the gaps between the items, split equally. `None` when that leaves under a pixel for
/// an item — the caller asked for a strip its labels cannot fit, and an inverted child rect panics.
pub fn segment_item(outer: Vec2, inset: f32, count: usize) -> Option<(f32, f32)> {
    if count == 0 {
        return None;
    }
    let inner = Vec2::new(
        (outer.x - 2.0 * (1.0 + inset)).max(0.0),
        (outer.y - 2.0 * (1.0 + inset)).max(0.0),
    );
    let count = count as f32;
    let item_width = ((inner.x - inset * (count - 1.0)) / count).max(0.0);
    if item_width < 1.0 || inner.y < 1.0 {
        return None;
    }
    Some((item_width, inner.y))
}

/// The design's other segmented control: the one its panes use (`.tabs .seg` in the inspector,
/// `.filterblk .seg` in the tree). Same container as [`segmented`], but the items are equal widths
/// across `width`, `inset` is both the inner padding and the gap between items, and the selection is
/// marked with an `accent_line` ring rather than the top bar's bottom indicator. `height` and `inset`
/// are the caller's literals because the design states them per pane: 28 with a 1px inset in the
/// tree, 26 with a 2px inset in the inspector. The items take what is left over, so the selected
/// item's ink stays the caller's choice — the inspector names it in `accent`, the tree in `text`.
pub fn pane_segmented(
    ui: &mut Ui,
    tokens: &Tokens,
    options: &[(&str, bool)],
    width: f32,
    height: f32,
    inset: f32,
    selected_ink: Color32,
) -> Option<usize> {
    if options.is_empty() {
        return None;
    }
    let font = tokens.font(T_XS);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let radius = CornerRadius::same(R_CTL);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tokens.panel_alt);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke),
        StrokeKind::Inside,
    );

    // The border row and the padding are spent before the items get their share.
    let inner = rect.shrink(1.0 + inset);
    let (item_width, item_height) = segment_item(rect.size(), inset, options.len())?;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = inset;
    let mut clicked = None;
    for (index, (label, selected)) in options.iter().enumerate() {
        let (item, response) =
            child.allocate_exact_size(Vec2::new(item_width, item_height), Sense::click());
        let painter = child.painter();
        if *selected {
            painter.rect_filled(item, CornerRadius::same(SEG_ITEM_RADIUS), tokens.accent_dim);
            painter.rect_stroke(
                item,
                CornerRadius::same(SEG_ITEM_RADIUS),
                Stroke::new(1.0, tokens.accent_line),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(item, CornerRadius::same(SEG_ITEM_RADIUS), tokens.panel_high);
        }
        painter.text(
            item.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if *selected {
                selected_ink
            } else if response.hovered() {
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
pub fn keycap(ui: &mut Ui, tokens: &Tokens, text: &str) -> Response {
    let font = tokens.mono(T_XS);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(keycap_width(ui, tokens, text), KEYCAP_H),
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
    // The design's `box-shadow: 0 1px 0 --border`: a hairline under the key, not a blur.
    painter.hline(
        rect.left()..=rect.right(),
        rect.bottom() + 0.5,
        Stroke::new(1.0, tokens.stroke),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        tokens.text,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    response
}

/// The row height a density preference asks for: the design's `--h-row` or `--h-row-comfort`.
pub fn row_h(density: datacraft_engine::Density) -> f32 {
    match density {
        datacraft_engine::Density::Compact => ROW_H,
        datacraft_engine::Density::Comfortable => ROW_COMFORT_H,
    }
}

/// Width of an `.iconbtn` for `label` and an optional glyph, so a caller can reserve the space
/// before it paints — that is how the top bar's palette field knows how wide it may flex.
pub fn icon_button_width(ui: &Ui, tokens: &Tokens, label: &str, glyph: bool) -> f32 {
    let label_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), tokens.font(T_XS), Color32::PLACEHOLDER)
        .size()
        .x;
    label_width + if glyph { GLYPH_SM + f32::from(S3) } else { 0.0 } + 2.0 * f32::from(S4) + 2.0
}

/// Width of a `.keycap` for `text`.
pub fn keycap_width(ui: &Ui, tokens: &Tokens, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), tokens.mono(T_XS), Color32::PLACEHOLDER)
        .size()
        .x
        + 2.0 * f32::from(S3)
        + 2.0
}

/// Width of a `.chip` or `.agent` for `text`: the 6px dot, the 6px gap and 8px of padding each side.
pub fn chip_width(ui: &Ui, tokens: &Tokens, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), tokens.font(T_XS), Color32::PLACEHOLDER)
        .size()
        .x
        + 6.0
        + f32::from(S3)
        + 2.0 * f32::from(S4)
        + 2.0
}

/// Width of an item inside a `.seg`: its label plus 8px of padding.
fn segment_item_width(ui: &Ui, tokens: &Tokens, label: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(label.to_owned(), tokens.font(T_XS), Color32::PLACEHOLDER)
        .size()
        .x
        + 2.0 * f32::from(S4)
}

/// Width of the whole `.seg` for `labels`, borders included.
pub fn segmented_width(ui: &Ui, tokens: &Tokens, labels: &[&str]) -> f32 {
    labels
        .iter()
        .map(|label| segment_item_width(ui, tokens, label))
        .sum::<f32>()
        + 2.0
}

/// Width of an `.engine` tag: uppercase mono with 6px of padding each side and a 1px border.
///
/// The text is upper-cased here because that is what [`paint_tag`] draws: measuring the string as
/// passed under-reserves for a lowercase caller.
pub fn tag_width(ui: &Ui, tokens: &Tokens, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_uppercase(), tokens.mono(T_XS), Color32::PLACEHOLDER)
        .size()
        .x
        + 2.0 * f32::from(S3)
        + 2.0
}

/// Paint an `.engine` tag inside `rect`: an uppercase mono pill on `surface-3`. The caller owns the
/// box, because the tag is a control's child and not a widget of its own.
pub fn paint_tag(ui: &Ui, tokens: &Tokens, rect: egui::Rect, text: &str) {
    let radius = CornerRadius::same(R_CTL);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tokens.hover);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text.to_uppercase(),
        tokens.mono(T_XS),
        tokens.text_dim,
    );
}

/// The design's `.iconbtn` and `.ws`: a 26px bordered control on `surface-2`, its ink `text_dim`.
pub fn icon_button(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    glyph: Option<crate::icons::Glyph>,
    enabled: bool,
) -> Response {
    let font = tokens.font(T_XS);
    let width = icon_button_width(ui, tokens, label, glyph.is_some());
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, ICON_BUTTON_H),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let radius = CornerRadius::same(R_CTL);
    let ink = if enabled {
        tokens.text_dim
    } else {
        tokens.text_faint
    };
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tokens.panel_alt);
    // The hover fill goes under the stroke: egui paints in call order, so a fill after the border
    // would swallow it, and the design keeps the 1px border in every state.
    if response.hovered() && enabled {
        painter.rect_filled(rect, radius, tokens.hover);
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tokens.stroke),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + f32::from(S4) + 1.0;
    if let Some(glyph) = glyph {
        crate::icons::glyph(
            ui,
            egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - GLYPH_SM / 2.0),
                Vec2::splat(GLYPH_SM),
            ),
            glyph,
            ink,
        );
        x += GLYPH_SM + f32::from(S3);
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

/// The design's `.chip` and `.agent`: a 24px pill with a leading 6px dot, filled or outlined.
pub fn state_chip(
    ui: &mut Ui,
    tokens: &Tokens,
    text: &str,
    fill: Option<Color32>,
    border: Option<Color32>,
    ink: Color32,
    dot: Color32,
) -> Response {
    let font = tokens.font(T_XS);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(chip_width(ui, tokens, text), CHIP_H),
        Sense::hover(),
    );
    let radius = CornerRadius::same(R_CHIP);
    let painter = ui.painter();
    // The design has three of these and they differ exactly in these two arguments: `.chip` (a dirty
    // count) is a filled pill with no border, `.chip.zero` is surface-2 inside a border, and `.agent`
    // is only a border — the bar's own fill shows through.
    if let Some(fill) = fill {
        painter.rect_filled(rect, radius, fill);
    }
    if let Some(border) = border {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    }
    painter.circle_filled(
        egui::pos2(rect.left() + f32::from(S4) + 1.0 + 3.0, rect.center().y),
        3.0,
        dot,
    );
    painter.text(
        egui::pos2(
            rect.left() + f32::from(S4) + 1.0 + 6.0 + f32::from(S3),
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        ink,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    response
}

/// Measure one line of `text`, so a band can reserve room before it paints.
fn measured(ui: &Ui, font: FontId, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
        .size()
        .x
}

/// `.fchips` height: an 18px chip with 7px of air above and below, the design's 32.
pub fn filter_row_h(chip_h: f32) -> f32 {
    chip_h + 2.0 * FCHIP_AIR
}

/// One micro tag's shape: `.pkbadge` and `.taglite` are the same box in different tokens — a 16px
/// box, a pill or 3px radius, a 1px border and one line of text, sized from its own measurement.
#[derive(Clone)]
struct MicroTag {
    font: FontId,
    ink: Color32,
    fill: Color32,
    border: Color32,
    radius: u8,
}

/// The one definition of `.pkbadge`: 11px mono ink, `accent_soft` over an `accent_line` border.
fn pk_style(tokens: &Tokens) -> MicroTag {
    MicroTag {
        font: tokens.mono(T_XS),
        ink: tokens.text,
        fill: tokens.accent_dim,
        border: tokens.accent_line,
        radius: 3,
    }
}

fn micro_tag(ui: &mut Ui, text: &str, tag: &MicroTag) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(micro_tag_width(ui, tag, text), MICRO_TAG_H),
        Sense::hover(),
    );
    paint_micro_tag(ui, rect, text, tag);
    response
}

/// The width a micro tag measures for `text`: the design's padding either side, plus its 1px border.
fn micro_tag_width(ui: &Ui, tag: &MicroTag, text: &str) -> f32 {
    measured(ui, tag.font.clone(), text) + 2.0 * f32::from(S3) + 2.0
}

/// A micro tag painted into a rect the caller has already laid out. The widgets above measure and
/// allocate one; a surface that lays out its own row — the grid's head does — paints the same shape
/// through this, so the two can never drift apart.
fn paint_micro_tag(ui: &Ui, rect: Rect, text: &str, tag: &MicroTag) {
    let painter = ui.painter();
    if tag.fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, CornerRadius::same(tag.radius), tag.fill);
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(tag.radius),
        Stroke::new(1.0, tag.border),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        tag.font.clone(),
        tag.ink,
    );
}

/// `.pkbadge`: the primary-key mark the design puts beside a table's name and in the grid head.
pub fn pk_badge(ui: &mut Ui, tokens: &Tokens, text: &str) -> Response {
    micro_tag(ui, text, &pk_style(tokens))
}

/// The width `.pkbadge` measures, so a surface that reserves room for one asks here rather than
/// guessing — the grid head does exactly that before it paints it.
pub fn pk_badge_width(ui: &Ui, tokens: &Tokens, text: &str) -> f32 {
    micro_tag_width(ui, &pk_style(tokens), text)
}

/// `.pkbadge` painted into `rect`: `pk_badge`'s geometry, for a caller that lays the badge out
/// itself. `rect` is the design's 16px-tall box, so a caller gets it from `pk_badge_width` and
/// `MICRO_TAG_H`.
pub fn paint_pk_badge(ui: &Ui, tokens: &Tokens, rect: Rect) {
    paint_micro_tag(ui, rect, "PK", &pk_style(tokens));
}

/// `.taglite`: the design's micro tag for one quiet fact about the grid, e.g. `read-only`.
pub fn tag_lite(ui: &mut Ui, tokens: &Tokens, text: &str) -> Response {
    micro_tag(
        ui,
        text,
        &MicroTag {
            font: tokens.font(T_XS),
            ink: tokens.text_dim,
            fill: Color32::TRANSPARENT,
            border: tokens.stroke_strong,
            radius: R_CHIP,
        },
    )
}

/// Height of `.errband` and `.robanner`: 10px of padding above and below a 16px title line and a
/// 16px message line, 2px apart.
pub fn notice_band_h() -> f32 {
    2.0 * NOTICE_PAD_V + 16.0 + f32::from(S1) + 16.0
}

/// `.errband`: the page that failed to load — an `error_soft` fill, a 1px `--error` bottom border,
/// the warning triangle (`.ei`), the h4, the engine's message in mono, and the caller's actions
/// pinned to the right.
pub fn error_band(
    ui: &mut Ui,
    tokens: &Tokens,
    title: &str,
    message: &str,
    add: impl FnOnce(&mut Ui),
) -> egui::Rect {
    notice_band(
        ui,
        tokens,
        tokens.error_fill,
        tokens.error,
        (crate::icons::Glyph::Warning, tokens.error),
        tokens.mono(T_SM),
        tokens.error,
        title,
        message,
        add,
    )
}

/// `.robanner`: the same box in `warn_soft`, for the grid that cannot be edited. Its paragraph is
/// the design's proportional `--t-xs`, unlike the error band's mono message.
pub fn ro_banner(
    ui: &mut Ui,
    tokens: &Tokens,
    title: &str,
    text: &str,
    add: impl FnOnce(&mut Ui),
) -> egui::Rect {
    notice_band(
        ui,
        tokens,
        tokens.warn_fill,
        tokens.warn,
        (crate::icons::Glyph::Lock, tokens.warn),
        tokens.font(T_XS),
        tokens.text_dim,
        title,
        text,
        add,
    )
}

#[allow(clippy::too_many_arguments)]
fn notice_band(
    ui: &mut Ui,
    tokens: &Tokens,
    fill: Color32,
    edge: Color32,
    mark: (crate::icons::Glyph, Color32),
    message_font: FontId,
    message_ink: Color32,
    title: &str,
    message: &str,
    add: impl FnOnce(&mut Ui),
) -> egui::Rect {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, notice_band_h()), Sense::hover());
    {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
        painter.hline(
            rect.left()..=rect.right(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, edge),
        );
    }
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(S5), rect.top() + NOTICE_PAD_V),
        egui::pos2(rect.right() - f32::from(S5), rect.bottom() - NOTICE_PAD_V),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = f32::from(S1);
    child.horizontal(|ui| {
        // The band's leading mark: the design's `.ei`/`.ri` beside the h4, 1px down from its cap.
        let (mark_rect, _) = ui.allocate_exact_size(Vec2::splat(GLYPH_BOX), Sense::hover());
        crate::icons::glyph(ui, mark_rect.translate(Vec2::Y), mark.0, mark.1);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(T_MD).color(tokens.text));
            ui.label(RichText::new(message).font(message_font).color(message_ink));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| add(ui));
    });
    rect
}

/// The design's `.btn`: a 26px control with 8px of padding and 6px between its mark, its label and
/// whatever the caller appends. `fill` is the sheet's resting fill, because the editor's `.tb` and the
/// register's `.rbar` state different ones — `panel_high` and `panel_alt`.
///
/// `add` runs inside the button, after the label, and the button grows to fit it. The closure paints
/// before the button's own chrome is laid down — egui has no second pass — so the fill, the pointer's
/// fill and the border go into shape slots reserved before the caller's content.
#[allow(clippy::too_many_arguments)]
pub fn toolbar_button(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    mark: Option<Mark>,
    fill: Color32,
    emphasis: Emphasis,
    enabled: bool,
    add: impl FnOnce(&mut Ui),
) -> Response {
    let face = button_face(tokens, fill, emphasis, enabled);
    let font = tokens.font(T_SM);
    let (width, lead, label_width) = button_width(ui, tokens, label, mark);
    let radius = CornerRadius::same(R_CTL);
    let slots = {
        let painter = ui.painter();
        [
            painter.add(egui::Shape::Noop),
            painter.add(egui::Shape::Noop),
            painter.add(egui::Shape::Noop),
        ]
    };
    let (id, placed) = ui.allocate_space(Vec2::new(width, TOOLBAR_BTN_H));
    // The caller's content starts a gap after the label: `lead` already carries the padding, the
    // border and the mark's own gap.
    let trailing = placed.left() + lead + label_width + f32::from(S3);
    let tail = {
        let room = Rect::from_min_max(
            egui::pos2(trailing, placed.top()),
            egui::pos2(
                ui.available_rect_before_wrap().right().max(trailing),
                placed.bottom(),
            ),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(room)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        add(&mut child);
        child.min_rect().width()
    };
    // The button grows to fit what the caller appended, and answers for the whole of itself: a
    // response that stopped at the label would leave the caret it drew outside its own hit box.
    let rect = if tail > 0.0 {
        let grown = Rect::from_min_max(
            placed.min,
            egui::pos2(placed.right() + f32::from(S3) + tail, placed.bottom()),
        );
        ui.advance_cursor_after_rect(grown);
        grown
    } else {
        placed
    };
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );

    let painter = ui.painter();
    if face.fill != Color32::TRANSPARENT {
        painter.set(slots[0], egui::Shape::rect_filled(rect, radius, face.fill));
    }
    if let Some(hover) = face.hover {
        if response.hovered() {
            painter.set(slots[1], egui::Shape::rect_filled(rect, radius, hover));
        }
    }
    painter.set(
        slots[2],
        egui::Shape::rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, face.border),
            StrokeKind::Inside,
        ),
    );
    let mut x = rect.left() + f32::from(S4) + 1.0;
    if let Some(mark) = mark {
        let size = mark.size();
        let box_rect = Rect::from_min_size(egui::pos2(x, rect.center().y - size.y / 2.0), size);
        match mark {
            Mark::Glyph(glyph) => {
                crate::icons::glyph(ui, box_rect, glyph, face.ink);
            }
            Mark::Play => {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        box_rect.left_top(),
                        egui::pos2(box_rect.right(), box_rect.center().y),
                        box_rect.left_bottom(),
                    ],
                    face.ink,
                    Stroke::NONE,
                ));
            }
            Mark::Stop => {
                painter.rect_filled(box_rect, CornerRadius::same(1), face.ink);
            }
        }
        x += size.x + f32::from(S3);
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        face.ink,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

/// The width [`toolbar_button`] takes for `label` and `mark`, so a caller can prove its toolbar fits
/// inside the pane before the band is laid out.
pub fn toolbar_button_width(ui: &Ui, tokens: &Tokens, label: &str, mark: Option<Mark>) -> f32 {
    button_width(ui, tokens, label, mark).0
}

/// A `.btn`'s total width, the width it spends before its label (a padding, the border, the mark and
/// its gap) and the label's own width — the caller's trailing content starts after that last one.
fn button_width(ui: &Ui, tokens: &Tokens, label: &str, mark: Option<Mark>) -> (f32, f32, f32) {
    let label_width = measured(ui, tokens.font(T_SM), label);
    let lead = f32::from(S4) + 1.0 + mark.map_or(0.0, |mark| mark.size().x + f32::from(S3));
    (lead + label_width + f32::from(S4) + 1.0, lead, label_width)
}

/// The mark a `.btn` carries in front of its label: a stroked [`crate::icons::Glyph`], or one of the
/// editor's filled marks — the run triangle and the stop square, which are fills and so are not glyphs
/// (`icons` is a stroked-geometry module, and a fill would change every existing glyph's contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Glyph(crate::icons::Glyph),
    Play,
    Stop,
}

impl Mark {
    /// The box the mark occupies, which is what a button reserves for it.
    pub fn size(self) -> Vec2 {
        match self {
            Mark::Glyph(_) => Vec2::splat(GLYPH_BOX),
            Mark::Play => Vec2::new(7.0, 10.0),
            Mark::Stop => Vec2::splat(8.0),
        }
    }
}

/// How much a `.btn` insists on being noticed: the design's plain button on its sheet's fill, or its
/// `.primary` one on `accent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    Plain,
    Primary,
}

/// What one `.btn` paints: its fill at rest, its 1px border, its ink, and the fill the pointer gets —
/// `None` when the button does not react to the pointer at all, which is what disabled means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonFace {
    pub fill: Color32,
    pub border: Color32,
    pub ink: Color32,
    pub hover: Option<Color32>,
}

/// The design's three `.btn` faces, from the sheet's resting fill: the plain button (`stroke` border,
/// `text` ink, `hover` under the pointer), `.primary` (`accent` fill, `accent_line` border,
/// `accent_text` ink) and `.off` (transparent, `text_faint` ink, no pointer reaction). Disabled beats
/// emphasis, because the only disabled button in these frames is the editor's Stop.
pub fn button_face(
    tokens: &Tokens,
    sheet: Color32,
    emphasis: Emphasis,
    enabled: bool,
) -> ButtonFace {
    if !enabled {
        return ButtonFace {
            fill: Color32::TRANSPARENT,
            border: tokens.stroke,
            ink: tokens.text_faint,
            hover: None,
        };
    }
    match emphasis {
        Emphasis::Primary => ButtonFace {
            fill: tokens.accent,
            border: tokens.accent_line,
            ink: tokens.accent_text,
            hover: None,
        },
        Emphasis::Plain => ButtonFace {
            fill: sheet,
            border: tokens.stroke,
            ink: tokens.text,
            hover: Some(tokens.hover),
        },
    }
}

/// The design's `.note`: a 20px `panel_alt` pill inside a `stroke` border, with an optional 5px dot,
/// carrying one mono fact in `text_dim`.
pub fn note_pill(ui: &mut Ui, tokens: &Tokens, text: &str, dot: Option<Color32>) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(note_pill_width(ui, tokens, text, dot.is_some()), NOTE_H),
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
    let x = if let Some(dot) = dot {
        let centre = note_dot_centre(rect);
        painter.circle_filled(centre, NOTE_DOT / 2.0, dot);
        centre.x + NOTE_DOT / 2.0 + f32::from(S3)
    } else {
        rect.left() + f32::from(S4) + 1.0
    };
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        tokens.mono(T_XS),
        tokens.text_dim,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    response
}

/// The width a `.note` takes for `text`, with or without its dot.
pub fn note_pill_width(ui: &Ui, tokens: &Tokens, text: &str, dot: bool) -> f32 {
    measured(ui, tokens.mono(T_XS), text)
        + 2.0 * f32::from(S4)
        + 2.0
        + if dot { NOTE_DOT + f32::from(S3) } else { 0.0 }
}

/// Where the `.note`'s 5px dot sits: one padding and the border in from the pill's left edge, on its
/// vertical centre.
pub fn note_dot_centre(pill: Rect) -> egui::Pos2 {
    egui::pos2(
        pill.left() + f32::from(S4) + 1.0 + NOTE_DOT / 2.0,
        pill.center().y,
    )
}

/// `.band`: the statement error band under the strip — `error_fill` with a 1px `error` top edge, the
/// design's `.ico` box beside a 13px title, an 11px mono sub-line, and the caller's actions pinned to
/// the right. The design's own `.band` carries 8px of padding and its content at the top; the 56 it
/// measures to is what the register's layout is built on, so the text column is centred in it.
pub fn statement_band(
    ui: &mut Ui,
    tokens: &Tokens,
    title: &str,
    message: &str,
    add: impl FnOnce(&mut Ui),
) -> egui::Rect {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, STMT_BAND_H), Sense::hover());
    {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, tokens.error_fill);
        painter.hline(
            rect.left()..=rect.right(),
            rect.top() + 0.5,
            Stroke::new(1.0, tokens.error),
        );
    }
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(S5), rect.top() + STMT_BAND_PAD_V),
        egui::pos2(
            rect.right() - f32::from(S5),
            rect.bottom() - STMT_BAND_PAD_V,
        ),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.x = f32::from(S4);
    child.horizontal(|ui| {
        let (mark, _) = ui.allocate_exact_size(Vec2::splat(STMT_MARK_BOX), Sense::hover());
        let mark = mark.translate(Vec2::Y * f32::from(S1));
        {
            let painter = ui.painter();
            painter.rect_filled(mark, CornerRadius::same(3), tokens.error);
            painter.text(
                mark.center(),
                egui::Align2::CENTER_CENTER,
                "!",
                tokens.mono(T_XS),
                tokens.background,
            );
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = f32::from(S1);
            ui.label(RichText::new(title).size(T_MD).color(tokens.text));
            ui.label(
                RichText::new(message)
                    .font(tokens.mono(T_XS))
                    .color(tokens.text_dim),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            add(ui);
        });
    });
    rect
}

/// `.eband`: the register's own error band, between its `.rbar` and its void. `panel_alt` rather than
/// the editor band's `error_fill` — the design states both and they differ — with a 1px `error`
/// hairline under it, `--s5` of padding, the design's 14px `error` mark, the driver's code in an 18px
/// `error`-ringed tag, the message in `text` and the statement in mono `text_dim`.
pub fn register_error_band(
    ui: &mut Ui,
    tokens: &Tokens,
    tag: &str,
    message: &str,
    statement: &str,
    add: impl FnOnce(&mut Ui),
) -> egui::Rect {
    let width = ui.available_width();
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(width, register_error_band_h()), Sense::hover());
    {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, tokens.panel_alt);
        painter.hline(
            rect.left()..=rect.right(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, tokens.error),
        );
    }
    let pad = f32::from(REG_ERR_PAD);
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + pad),
        egui::pos2(rect.right() - pad, rect.bottom() - pad),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.x = f32::from(S5);
    child.horizontal(|ui| {
        // The design's `.eband > svg`: a 14px mark 2px down from the body's cap.
        let (mark, _) = ui.allocate_exact_size(Vec2::splat(REG_ERR_MARK), Sense::hover());
        crate::icons::glyph(
            ui,
            mark.translate(Vec2::Y * f32::from(S1)),
            crate::icons::Glyph::Warning,
            tokens.error,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = f32::from(S3);
            let (row, _) = ui.allocate_exact_size(
                Vec2::new(
                    measured(ui, tokens.mono(T_XS), tag) + 2.0 * f32::from(S3) + 2.0,
                    REG_ERR_TAG_H,
                ),
                Sense::hover(),
            );
            let radius = CornerRadius::same(R_CTL);
            let painter = ui.painter();
            painter.rect_stroke(
                row,
                radius,
                Stroke::new(1.0, tokens.error),
                StrokeKind::Inside,
            );
            painter.text(
                row.center(),
                egui::Align2::CENTER_CENTER,
                tag,
                tokens.mono(T_XS),
                tokens.error,
            );
            ui.label(RichText::new(message).size(T_SM).color(tokens.text));
            ui.label(
                RichText::new(statement)
                    .font(tokens.mono(T_XS))
                    .color(tokens.text_dim),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            add(ui);
        });
    });
    rect
}

/// `.band`'s text column: a 16px title line, 2px of air and a 16px mono sub-line. The design measures
/// the whole band at 56, which is what the strip and the register below it are laid out against, so
/// the padding this leaves — 11px — is derived rather than the design's own 8.
const STMT_TITLE_H: f32 = 16.0;
const STMT_SUB_H: f32 = 16.0;
/// The design's `.ico`: a 16px filled `error` box carrying the bang.
const STMT_MARK_BOX: f32 = 16.0;
const STMT_BAND_PAD_V: f32 = (STMT_BAND_H - STMT_TITLE_H - S1 as f32 - STMT_SUB_H) / 2.0;

/// `.eband`'s own parts: the design's 14px mark, its 18px tag row and the message and mono statement
/// lines, `--s3` apart.
const REG_ERR_MARK: f32 = 14.0;
const REG_ERR_TAG_H: f32 = 18.0;
const REG_ERR_LINE_H: f32 = 16.0;

/// `.eband`'s height: `--s5` of padding, then its three lines.
pub fn register_error_band_h() -> f32 {
    2.0 * f32::from(REG_ERR_PAD)
        + REG_ERR_TAG_H
        + f32::from(S3)
        + REG_ERR_LINE_H
        + f32::from(S3)
        + REG_ERR_LINE_H
}

/// `.note`'s dot: 5px, on the pill's vertical centre.
const NOTE_DOT: f32 = 5.0;

/// `.stalestrip`: the 26px `surface-2` row between the error band and the grid, its note on the
/// left and one mono fact on the right.
pub fn stale_strip(ui: &mut Ui, tokens: &Tokens, note: &str, fact: &str) -> egui::Rect {
    let rect = band_fill(
        ui,
        STALE_STRIP_H,
        tokens.panel_alt,
        S4,
        egui::Align::Center,
        |ui| {
            ui.label(RichText::new(note).size(T_XS).color(tokens.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(fact)
                        .size(T_XS)
                        .monospace()
                        .color(tokens.text_dim),
                );
            });
        },
    );
    band_hairline(ui, tokens, rect);
    rect
}

/// What the `.pend` cluster's buttons asked for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    Revert,
    Apply,
}

/// `.pend`'s sentence: `1 pending`, `3 pending`.
pub fn pending_sentence(counts: (usize, usize, usize)) -> String {
    format!("{} pending", pending_total(counts))
}

/// The staged buffer's size: inserts + updates + deletes.
pub fn pending_total(counts: (usize, usize, usize)) -> usize {
    counts.0 + counts.1 + counts.2
}

/// `.pend`'s counts by kind in the design's order — `+2 ●1 −3` — each kind left out when it is zero.
pub fn pending_counts_text(counts: (usize, usize, usize)) -> String {
    let mut parts: Vec<String> = Vec::new();
    if counts.0 > 0 {
        parts.push(format!("+{}", counts.0));
    }
    if counts.1 > 0 {
        parts.push(format!("●{}", counts.1));
    }
    if counts.2 > 0 {
        parts.push(format!("−{}", counts.2));
    }
    parts.join(" ")
}

/// The cluster's Apply label: `Apply 3`.
pub fn apply_label(n: usize) -> String {
    format!("Apply {n}")
}

/// The drawer footer's Apply label: `Apply 1 change`, `Apply 3 changes`.
pub fn apply_changes_label(n: usize) -> String {
    let noun = crate::table::plural(n as u64, "change", "changes");
    format!("Apply {n} {noun}")
}

/// `.fchips`' right-hand fact: `1,204 matches`.
pub fn matches_label(n: u64) -> String {
    let noun = crate::table::plural(n, "match", "matches");
    format!("{} {noun}", crate::table::grouped(n))
}

/// `.pend`: the design's warn-soft cluster pinned to the toolbar's right edge and as tall as the
/// band it sits in — the counts by kind, Revert and `Apply n`. Returns `None` for an empty buffer:
/// with nothing staged there is no cluster, which is also what the design draws for its own
/// `0 pending` states.
pub fn pending_cluster(
    ui: &mut Ui,
    tokens: &Tokens,
    counts: (usize, usize, usize),
) -> Option<PendingAction> {
    let total = pending_total(counts);
    if total == 0 {
        return None;
    }
    let sentence = pending_sentence(counts);
    let apply = apply_label(total);
    let counts_text = pending_counts_text(counts);
    // The two buttons are 24px boxes with the style's 10px of padding each side and a hairline; the
    // counts and the sentence are one line each, 8px apart, inside S5 of padding and a 1px edge.
    let button = |label: &str| measured(ui, tokens.font(T_SM), label) + 2.0 * 10.0 + 2.0;
    let pieces: [f32; 4] = [
        measured(ui, tokens.font(T_SM), &sentence),
        measured(ui, tokens.mono(T_XS), &counts_text),
        button("Revert"),
        button(&apply),
    ];
    let width = pieces.iter().sum::<f32>()
        + f32::from(S4) * pieces.len() as f32
        + 2.0 * f32::from(S5)
        + f32::from(S2);

    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ui.available_height()), Sense::hover());
    {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, tokens.warn_fill);
        painter.vline(
            rect.left() + 0.5,
            rect.top()..=rect.bottom(),
            Stroke::new(1.0, tokens.stroke),
        );
    }
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(S5), rect.top()),
        egui::pos2(rect.right() - f32::from(S5), rect.bottom()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = f32::from(S4);
    let mut action = None;
    if primary_button(&mut child, tokens, &apply, true)
        .on_hover_text("Run the pending statements in one transaction")
        .clicked()
    {
        action = Some(PendingAction::Apply);
    }
    if ghost_button(&mut child, tokens, "Revert", true)
        .on_hover_text("Throw the pending statements away")
        .clicked()
    {
        action = Some(PendingAction::Revert);
    }
    if counts.2 > 0 {
        child.label(
            RichText::new(format!("−{}", counts.2))
                .size(T_XS)
                .monospace()
                .color(tokens.error),
        );
    }
    if counts.1 > 0 {
        child.label(
            RichText::new(format!("●{}", counts.1))
                .size(T_XS)
                .monospace()
                .color(tokens.warn),
        );
    }
    if counts.0 > 0 {
        child.label(
            RichText::new(format!("+{}", counts.0))
                .size(T_XS)
                .monospace()
                .color(tokens.ok),
        );
    }
    child.label(RichText::new(&sentence).size(T_SM).color(tokens.warn));
    action
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

    #[test]
    fn every_pane_head_adds_up_to_the_same_line() {
        // The tree, the grid and the inspector start their bodies on one line only while each head
        // adds up to the design's `--h-panehead`. The tree is 34 + its filter block; the inspector is
        // its object header, its tab strip and the list head.
        assert_eq!(
            crate::tree::HEADER_H + crate::tree::FILTER_BLOCK_H,
            PANE_HEAD_H
        );
        assert_eq!(
            crate::inspector::HEADER_H + TOOLBAR_H + HEAD_H,
            PANE_HEAD_H,
            "the inspector's bands no longer add up to the pane head"
        );
    }

    #[test]
    fn the_filter_row_centres_its_chip_in_the_designs_thirty_two() {
        assert_eq!(FCHIPS_H, 32.0, "the design's `.fchips` is 32px tall");
        assert_eq!(filter_row_h(FCHIP_H), FCHIPS_H);
        assert_eq!(FCHIP_AIR, (FCHIPS_H - FCHIP_H) / 2.0);
        assert_eq!(STALE_STRIP_H, 26.0);
        assert_eq!(notice_band_h(), 54.0, "10 + title + 2 + message + 10");
        assert_eq!(MICRO_TAG_H, 16.0);
    }

    #[test]
    fn the_pending_cluster_counts_each_kind_and_drops_the_empty_ones() {
        assert_eq!(pending_total((3, 0, 4)), 7);
        assert_eq!(pending_sentence((1, 0, 0)), "1 pending");
        assert_eq!(pending_sentence((0, 2, 1)), "3 pending");
        assert_eq!(pending_counts_text((1, 1, 1)), "+1 ●1 −1");
        assert_eq!(pending_counts_text((0, 2, 0)), "●2");
        assert_eq!(pending_counts_text((0, 0, 0)), "");
    }

    #[test]
    fn an_apply_or_match_label_pluralises_or_groups_its_count() {
        assert_eq!(apply_label(1), "Apply 1");
        assert_eq!(apply_changes_label(1), "Apply 1 change");
        assert_eq!(apply_changes_label(3), "Apply 3 changes");
        assert_eq!(matches_label(1), "1 match");
        assert_eq!(matches_label(0), "0 matches");
        assert_eq!(matches_label(12_481), "12,481 matches");
    }

    #[test]
    fn a_pane_segmented_control_shares_its_inner_width_and_refuses_the_impossible() {
        let (width, height) =
            segment_item(Vec2::new(124.0, 28.0), 1.0, 4).expect("four items fit in a 124px strip");
        assert_eq!(width, (124.0 - 4.0 - 3.0) / 4.0);
        assert_eq!(height, 24.0);
        // 28 minus the 1px border and the 1px inset on each side.
        assert!(segment_item(Vec2::new(6.0, 28.0), 2.0, 4).is_none());
        assert!(segment_item(Vec2::new(124.0, 2.0), 1.0, 4).is_none());
        assert!(segment_item(Vec2::new(124.0, 28.0), 1.0, 0).is_none());
    }

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

    /// A headless frame the way `table.rs` drives one: no texture uploads, the query pane's own rect.
    fn query_frame() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(344.0, 75.0),
                egui::vec2(776.0, 900.0),
            )),
            ..Default::default()
        }
    }

    /// The toolbar's `.btn` wears one of the design's three faces, and `off` beats emphasis: a
    /// disabled `.btn.primary` is the plain disabled button.
    #[test]
    fn a_toolbar_button_wears_one_of_the_designs_three_faces() {
        for tokens in [Tokens::of("dark"), Tokens::of("light")] {
            let theme = if tokens.dark { "dark" } else { "light" };
            let sheet = tokens.panel_high;

            let plain = button_face(&tokens, sheet, Emphasis::Plain, true);
            assert_eq!(plain.fill, sheet, "a plain button sits on its sheet's fill");
            assert_eq!(plain.border, tokens.stroke);
            assert_eq!(plain.ink, tokens.text);
            assert_eq!(plain.hover, Some(tokens.hover));

            let primary = button_face(&tokens, sheet, Emphasis::Primary, true);
            assert_eq!(primary.fill, tokens.accent);
            assert_eq!(primary.border, tokens.accent_line);
            assert_eq!(primary.ink, tokens.accent_text);
            assert_eq!(
                primary.hover, None,
                "the design's primary has no hover fill"
            );

            for emphasis in [Emphasis::Plain, Emphasis::Primary] {
                let off = button_face(&tokens, sheet, emphasis, false);
                assert_eq!(
                    off.fill,
                    Color32::TRANSPARENT,
                    "`.btn.off` paints no fill of its own"
                );
                assert_eq!(off.ink, tokens.text_faint);
                assert_eq!(off.hover, None, "a disabled button ignores the pointer");
                // A transparent fill means the band's own fill shows through, and those differ: the
                // editor's `.tb` is `panel_alt` inside the pane, the register's `.rbar` is `panel`.
                for under in [tokens.panel_alt, tokens.panel, sheet] {
                    let ratio = contrast(off.ink, under);
                    assert!(
                        ratio >= 2.3,
                        "{theme}: the disabled ink is {ratio:.2}:1 on its band"
                    );
                }
            }
        }
    }

    /// The `.btn` reserves room for its mark before it measures itself, and the editor's two marks are
    /// the design's fills rather than glyphs.
    #[test]
    fn a_toolbar_button_reserves_room_for_the_mark_it_carries() {
        assert_eq!(Mark::Play.size(), Vec2::new(7.0, 10.0));
        assert_eq!(Mark::Stop.size(), Vec2::splat(8.0));
        assert_eq!(
            Mark::Glyph(crate::icons::Glyph::Warning).size(),
            Vec2::splat(GLYPH_BOX)
        );

        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        let mut out = ctx.run_ui(query_frame(), |ui| {
            let bare = toolbar_button_width(ui, &tokens, "Run", None);
            let played = toolbar_button_width(ui, &tokens, "Run", Some(Mark::Play));
            assert_eq!(played - bare, Mark::Play.size().x + f32::from(S3));
            // The label, a padding and a 1px border each side, and nothing else.
            assert_eq!(
                bare,
                measured(ui, tokens.font(T_SM), "Run") + 2.0 * f32::from(S4) + 2.0
            );

            // Whatever the caller appends starts a gap after the label and the button grows to fit
            // it: the design's `Export` carries its caret inside the button.
            let width = toolbar_button_width(ui, &tokens, "Export", None);
            let mut placed = Rect::NOTHING;
            let button = toolbar_button(
                ui,
                &tokens,
                "Export",
                None,
                tokens.panel_high,
                Emphasis::Plain,
                true,
                |ui| {
                    placed = ui
                        .allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover())
                        .0;
                },
            );
            assert_eq!(
                placed.left(),
                button.rect.left() + width - f32::from(S4) - 1.0 + f32::from(S3),
                "the trailing content starts one padding and border short of the right edge"
            );
            assert_eq!(button.rect.width(), width + f32::from(S3) + 10.0);
            assert_eq!(button.rect.height(), TOOLBAR_BTN_H);
        });
        out.textures_delta.clear();
    }

    /// The chrome reaches the frame even though the caller's content is painted first: the fill, the
    /// pointer's fill and the border go into slots reserved before it.
    #[test]
    fn a_toolbar_button_paints_the_face_it_chose() {
        let tokens = Tokens::of("dark");
        let painted = |emphasis: Emphasis, enabled: bool| -> Vec<egui::Shape> {
            let ctx = egui::Context::default();
            tokens.apply(&ctx);
            let mut out = ctx.run_ui(query_frame(), |ui| {
                let _ = toolbar_button(
                    ui,
                    &tokens,
                    "Run",
                    Some(Mark::Play),
                    tokens.panel_high,
                    emphasis,
                    enabled,
                    |ui| {
                        ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                    },
                );
            });
            out.textures_delta.clear();
            out.shapes
                .into_iter()
                .map(|clipped| clipped.shape)
                .collect()
        };
        let filled = |shapes: &[egui::Shape], colour: Color32| {
            shapes
                .iter()
                .any(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill == colour))
        };
        let outlined = |shapes: &[egui::Shape], colour: Color32| {
            shapes.iter().any(
                |shape| matches!(shape, egui::Shape::Rect(rect) if rect.stroke.color == colour),
            )
        };

        let primary = painted(Emphasis::Primary, true);
        assert!(
            filled(&primary, tokens.accent),
            "the primary's fill has to reach the frame"
        );
        assert!(outlined(&primary, tokens.accent_line));

        let off = painted(Emphasis::Plain, false);
        assert!(
            !filled(&off, tokens.panel_high),
            "a disabled button paints no sheet fill"
        );
        assert!(!filled(&off, tokens.accent));
        assert!(outlined(&off, tokens.stroke));
    }

    /// `.note` is 20 tall and its 5px dot sits inside it, one padding in from the left edge.
    #[test]
    fn a_note_pill_centres_its_dot_inside_its_twenty() {
        assert_eq!(NOTE_H, 20.0);
        let pill = Rect::from_min_size(egui::pos2(344.0, 507.0), Vec2::new(214.0, NOTE_H));
        let centre = note_dot_centre(pill);
        assert_eq!(centre.y, pill.center().y);
        assert_eq!(centre.x, 344.0 + f32::from(S4) + 1.0 + NOTE_DOT / 2.0);
        assert!(
            centre.x - NOTE_DOT / 2.0 >= pill.left() && centre.x + NOTE_DOT / 2.0 <= pill.right(),
            "the dot's box has to sit inside the pill"
        );
        const {
            assert!(
                NOTE_DOT < NOTE_H,
                "the dot has to sit inside the pill's height"
            )
        };
    }

    /// Both error bands: their parts add up to the height the register's layout spends on them, and
    /// every inner slot fits inside that height.
    #[test]
    fn both_error_bands_add_up_to_their_constants() {
        assert_eq!(STMT_BAND_H, 56.0, "the design's `.band` is 56");
        // 56 = a 16px title, 2px of air, a 16px mono sub-line, and what that leaves above and below.
        assert_eq!(
            2.0 * STMT_BAND_PAD_V + STMT_TITLE_H + f32::from(S1) + STMT_SUB_H,
            STMT_BAND_H
        );
        const { assert!(STMT_BAND_PAD_V > 0.0) };
        // The `.ico` box and its 2px drop have to fit beside the text column.
        assert!(STMT_MARK_BOX + f32::from(S1) <= STMT_TITLE_H + f32::from(S1) + STMT_SUB_H);

        assert_eq!(REG_ERR_PAD, S5, "the design pads `.eband` with `--s5`");
        assert_eq!(
            register_error_band_h(),
            86.0,
            "12 + 18 + 6 + 16 + 6 + 16 + 12"
        );
        assert_eq!(
            register_error_band_h(),
            2.0 * f32::from(REG_ERR_PAD)
                + REG_ERR_TAG_H
                + f32::from(S3)
                + REG_ERR_LINE_H
                + f32::from(S3)
                + REG_ERR_LINE_H
        );
        // The body's own height: the tag row, the message and the statement, `--s3` apart.
        let body = register_error_band_h() - 2.0 * f32::from(REG_ERR_PAD);
        assert!(REG_ERR_TAG_H + f32::from(S3) + 2.0 * REG_ERR_LINE_H <= body);
        // The design's 14px mark, dropped 2px, has to fit in the same body.
        assert!(REG_ERR_MARK + f32::from(S1) <= body);
    }

    /// The four widgets allocate exactly the heights their constants claim.
    #[test]
    fn the_query_workspaces_bands_are_the_heights_their_constants_claim() {
        let ctx = egui::Context::default();
        let tokens = Tokens::of("dark");
        tokens.apply(&ctx);
        let mut heights = [0.0_f32; 4];
        let mut out = ctx.run_ui(query_frame(), |ui| {
            heights[0] = statement_band(
                ui,
                &tokens,
                "near \"FROMM\": syntax error",
                "SQLITE_ERROR · Ln 3, Col 1 · statement 1 · nothing was executed",
                |_| {},
            )
            .height();
            heights[1] = register_error_band(
                ui,
                &tokens,
                "SQLITE_ERROR",
                "no such column: statuss",
                "SELECT id, statuss FROM main.customers;",
                |_| {},
            )
            .height();
            heights[2] = note_pill(
                ui,
                &tokens,
                "statement 2 of 2 · no result set",
                Some(tokens.info),
            )
            .rect
            .height();
            heights[3] = toolbar_button(
                ui,
                &tokens,
                "Run",
                Some(Mark::Play),
                tokens.panel_high,
                Emphasis::Primary,
                true,
                |_| {},
            )
            .rect
            .height();
        });
        out.textures_delta.clear();
        assert_eq!(heights[0], STMT_BAND_H);
        assert_eq!(heights[1], register_error_band_h());
        assert_eq!(heights[2], NOTE_H);
        assert_eq!(heights[3], TOOLBAR_BTN_H);
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
