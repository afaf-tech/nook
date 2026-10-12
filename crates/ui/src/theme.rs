//! Visual tokens. Colours live here so the UI code never invents one.

use egui::Color32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tokens {
    pub dark: bool,
    pub background: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub grid_header: Color32,
    pub grid_row_alt: Color32,
    pub selection: Color32,
    pub dirty: Color32,
}

impl Tokens {
    pub fn of(theme: &str) -> Tokens {
        match theme {
            "light" => Tokens {
                dark: false,
                background: Color32::from_rgb(0xf4, 0xf6, 0xfa),
                panel: Color32::from_rgb(0xff, 0xff, 0xff),
                panel_alt: Color32::from_rgb(0xec, 0xf0, 0xf6),
                stroke: Color32::from_rgb(0xcd, 0xd5, 0xdf),
                text: Color32::from_rgb(0x1b, 0x22, 0x2e),
                text_dim: Color32::from_rgb(0x5c, 0x67, 0x76),
                accent: Color32::from_rgb(0x1f, 0x5f, 0xd4),
                ok: Color32::from_rgb(0x18, 0x82, 0x4a),
                warn: Color32::from_rgb(0xb2, 0x6a, 0x00),
                error: Color32::from_rgb(0xc0, 0x2c, 0x2c),
                grid_header: Color32::from_rgb(0xe4, 0xe9, 0xf1),
                grid_row_alt: Color32::from_rgb(0xf7, 0xf9, 0xfc),
                selection: Color32::from_rgb(0xd6, 0xe4, 0xff),
                dirty: Color32::from_rgb(0xff, 0xf3, 0xcd),
            },
            _ => Tokens {
                dark: true,
                background: Color32::from_rgb(0x0f, 0x13, 0x1a),
                panel: Color32::from_rgb(0x16, 0x1c, 0x26),
                panel_alt: Color32::from_rgb(0x1d, 0x25, 0x32),
                stroke: Color32::from_rgb(0x2b, 0x36, 0x47),
                text: Color32::from_rgb(0xe6, 0xed, 0xf7),
                text_dim: Color32::from_rgb(0x91, 0xa2, 0xba),
                accent: Color32::from_rgb(0x5b, 0x8d, 0xff),
                ok: Color32::from_rgb(0x46, 0xc9, 0x7e),
                warn: Color32::from_rgb(0xe3, 0xb3, 0x41),
                error: Color32::from_rgb(0xef, 0x6f, 0x6f),
                grid_header: Color32::from_rgb(0x20, 0x2a, 0x3a),
                grid_row_alt: Color32::from_rgb(0x18, 0x1f, 0x2b),
                selection: Color32::from_rgb(0x25, 0x3c, 0x6b),
                dirty: Color32::from_rgb(0x3d, 0x33, 0x12),
            },
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.extreme_bg_color = self.background;
        visuals.faint_bg_color = self.panel_alt;
        visuals.widgets.noninteractive.bg_stroke.color = self.stroke;
        visuals.widgets.inactive.bg_fill = self.panel_alt;
        visuals.selection.bg_fill = self.selection;
        visuals.override_text_color = Some(self.text);
        visuals.hyperlink_color = self.accent;
        ctx.set_visuals(visuals);
    }

    pub fn level_color(&self, level: datacraft_engine::Level) -> Color32 {
        match level {
            datacraft_engine::Level::Info => self.text_dim,
            datacraft_engine::Level::Warn => self.warn,
            datacraft_engine::Level::Error => self.error,
            datacraft_engine::Level::Success => self.ok,
        }
    }
}
