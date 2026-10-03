//! Top bar: sidebar toggle, tabs, SFTP toggle and settings.

use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, Vec2};

use super::theme;
use super::widgets::flat_button;
use crate::terminal::Status;

pub struct TabInfo {
    pub title: String,
    pub status: Status,
}

pub enum TabAction {
    Select(usize),
    Close(usize),
    NewTab,
    ToggleSidebar,
    ToggleSftp,
    Settings,
}

const TAB_HEIGHT: f32 = 30.0;
const TAB_MAX_WIDTH: f32 = 200.0;

pub fn show(ui: &mut Ui, tabs: &[TabInfo], active: usize, sftp_available: bool, sftp_open: bool) -> Option<TabAction> {
    let mut action = None;
    ui.horizontal_centered(|ui| {
        if flat_button(ui, "\u{2261}").on_hover_text("サイドバー").clicked() {
            action = Some(TabAction::ToggleSidebar);
        }
        let right_width = 150.0;
        let tabs_width = (ui.available_width() - right_width - 36.0).max(80.0);
        let tab_width = (tabs_width / tabs.len().max(1) as f32).clamp(90.0, TAB_MAX_WIDTH);
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, tab) in tabs.iter().enumerate() {
            if let Some(a) = tab_widget(ui, i, tab, i == active, tab_width) {
                action = Some(a);
            }
        }
        if flat_button(ui, "+").on_hover_text("新しいタブ (Ctrl+Shift+T)").clicked() {
            action = Some(TabAction::NewTab);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if flat_button(ui, "設定").clicked() {
                action = Some(TabAction::Settings);
            }
            let sftp = ui.add_enabled(
                sftp_available,
                egui::Button::selectable(sftp_open, egui::RichText::new("SFTP").size(12.5)),
            );
            if sftp.on_hover_text("ファイル転送パネル").clicked() {
                action = Some(TabAction::ToggleSftp);
            }
        });
    });
    action
}

fn tab_widget(ui: &mut Ui, index: usize, tab: &TabInfo, active: bool, width: f32) -> Option<TabAction> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, TAB_HEIGHT), Sense::click());
    let close_rect = Rect::from_center_size(egui::pos2(rect.max.x - 14.0, rect.center().y), Vec2::splat(18.0));
    let close = ui.interact(close_rect, ui.id().with(("tab-close", index)), Sense::click());
    let painter = ui.painter();

    let fill = if active {
        theme::SURFACE
    } else if response.hovered() {
        theme::BG
    } else {
        theme::PANEL
    };
    painter.rect_filled(rect, 6.0, fill);
    if active {
        painter.hline(rect.shrink2(Vec2::new(8.0, 0.0)).x_range(), rect.max.y - 1.0, Stroke::new(2.0, theme::ACCENT));
    }
    let marker = match tab.status {
        Status::Connecting => theme::WARNING,
        Status::Running => theme::SUCCESS,
        Status::Exited(_) => theme::DANGER,
    };
    painter.circle_filled(egui::pos2(rect.min.x + 12.0, rect.center().y), 3.5, marker);
    let text_clip = Rect::from_min_max(rect.min, egui::pos2(close_rect.min.x - 2.0, rect.max.y));
    painter.with_clip_rect(text_clip).text(
        egui::pos2(rect.min.x + 22.0, rect.center().y),
        Align2::LEFT_CENTER,
        &tab.title,
        FontId::proportional(13.0),
        if active { theme::TEXT } else { theme::TEXT_DIM },
    );
    if close.hovered() {
        painter.rect_filled(close_rect, 4.0, theme::SURFACE_HOVER);
    }
    painter.text(close_rect.center(), Align2::CENTER_CENTER, "\u{00d7}", FontId::proportional(14.0), theme::TEXT_DIM);

    if close.clicked() || response.middle_clicked() {
        Some(TabAction::Close(index))
    } else if response.clicked() {
        Some(TabAction::Select(index))
    } else {
        response.on_hover_text(&tab.title);
        None
    }
}
