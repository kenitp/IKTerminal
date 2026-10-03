//! Application color scheme and egui style.

use egui::{Color32, CornerRadius, Margin, Stroke, Theme, Vec2};

use crate::terminal::palette;

pub const BG: Color32 = Color32::from_rgb(0x13, 0x14, 0x1c);
pub const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x26);
pub const SURFACE: Color32 = Color32::from_rgb(0x24, 0x26, 0x36);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x2f, 0x33, 0x4d);
pub const BORDER: Color32 = Color32::from_rgb(0x29, 0x2e, 0x42);
pub const TEXT: Color32 = Color32::from_rgb(0xc0, 0xca, 0xf5);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x73, 0x7a, 0xa2);
pub const ACCENT: Color32 = Color32::from_rgb(0x7a, 0xa2, 0xf7);
pub const SUCCESS: Color32 = Color32::from_rgb(0x9e, 0xce, 0x6a);
pub const WARNING: Color32 = Color32::from_rgb(0xe0, 0xaf, 0x68);
pub const DANGER: Color32 = Color32::from_rgb(0xf7, 0x76, 0x8e);

pub fn rgb(c: alacritty_terminal::vte::ansi::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

pub fn term_bg() -> Color32 {
    rgb(palette::BACKGROUND)
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;

    let radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    for (state, fill) in [
        (&mut w.inactive, SURFACE),
        (&mut w.hovered, SURFACE_HOVER),
        (&mut w.active, SURFACE_HOVER),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, TEXT);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, BORDER);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.open = w.active;

    ctx.set_theme(Theme::Dark);
    ctx.set_visuals_of(Theme::Dark, v);
    ctx.style_mut_of(Theme::Dark, |s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 4.0);
        s.spacing.window_margin = Margin::same(14);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.interact_size.y = 24.0;
    });
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}
