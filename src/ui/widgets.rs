//! Small reusable widgets.

use egui::{Align2, Color32, FontId, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::theme;

pub fn section_header(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).size(11.0).color(theme::TEXT_DIM).strong());
}

/// Full-width clickable row with a colored marker, a title and an optional subtitle.
pub fn list_row(ui: &mut Ui, marker: Color32, title: &str, subtitle: &str, selected: bool) -> Response {
    let height = if subtitle.is_empty() { 28.0 } else { 42.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    if selected || response.hovered() {
        let fill = if selected { theme::SURFACE_HOVER } else { theme::SURFACE };
        painter.rect_filled(rect, 6.0, fill);
    }
    let dot = Rect::from_center_size(egui::pos2(rect.min.x + 12.0, rect.center().y), Vec2::splat(7.0));
    painter.rect_filled(dot, 2.0, marker);
    let x = rect.min.x + 24.0;
    let clip = rect.shrink2(Vec2::new(4.0, 0.0));
    let painter = painter.with_clip_rect(clip);
    if subtitle.is_empty() {
        painter.text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, title, FontId::proportional(13.5), theme::TEXT);
    } else {
        painter.text(egui::pos2(x, rect.min.y + 4.0), Align2::LEFT_TOP, title, FontId::proportional(13.5), theme::TEXT);
        painter.text(
            egui::pos2(x, rect.max.y - 4.0),
            Align2::LEFT_BOTTOM,
            subtitle,
            FontId::proportional(11.5),
            theme::TEXT_DIM,
        );
    }
    response
}

/// Text button without background until hovered.
pub fn flat_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(egui::RichText::new(text).size(12.5)).frame_when_inactive(false))
}

/// Accent-colored primary button.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(theme::BG).strong())
            .fill(theme::ACCENT)
            .stroke(Stroke::NONE),
    )
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}
