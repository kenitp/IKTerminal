//! Top bar: sidebar toggle, tabs, new-tab menu, window controls.

use egui::{Align2, FontId, Id, Layout, PointerButton, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};

use super::theme;
use super::widgets::flat_button;
use crate::backend::local::ShellSpec;
use crate::sshconfig::HostEntry;
use crate::terminal::Status;

pub struct TabInfo {
    pub title: String,
    pub status: Status,
}

pub enum TabAction {
    Select(usize),
    Close(usize),
    OpenLocal(usize),
    OpenSsh(usize),
    OpenSerial,
    ToggleSidebar,
    ToggleSftp,
    Settings,
}

const BAR_HEIGHT: f32 = 36.0;
const TAB_HEIGHT: f32 = 30.0;
const TAB_MAX_WIDTH: f32 = 200.0;

pub struct Bar<'a> {
    pub tabs: &'a [TabInfo],
    pub active: usize,
    pub sftp_available: bool,
    pub sftp_open: bool,
    pub shells: &'a [ShellSpec],
    pub hosts: &'a [HostEntry],
    pub menu_open: &'a mut bool,
}

pub fn show(ui: &mut Ui, bar: Bar<'_>) -> Option<TabAction> {
    let Bar {
        tabs,
        active,
        sftp_available,
        sftp_open,
        shells,
        hosts,
        menu_open,
    } = bar;
    let available = ui.available_rect_before_wrap();
    let bar = Rect::from_min_size(available.min, Vec2::new(available.width(), BAR_HEIGHT));
    ui.allocate_rect(bar, Sense::hover());
    window_drag(ui, bar);

    let mut action = None;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(bar)
            .layout(Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add_space(8.0);
            if flat_button(ui, "\u{2261}")
                .on_hover_text("サイドバー")
                .clicked()
            {
                action = Some(TabAction::ToggleSidebar);
            }
            let right_width = 250.0;
            let tabs_width = (ui.available_width() - right_width - 36.0).max(80.0);
            let tab_width = (tabs_width / tabs.len().max(1) as f32).clamp(90.0, TAB_MAX_WIDTH);
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, tab) in tabs.iter().enumerate() {
                if let Some(a) = tab_widget(ui, i, tab, i == active, tab_width) {
                    action = Some(a);
                }
            }
            let plus = flat_button(ui, "+").on_hover_text("新しいタブ (Ctrl+Shift+T)");
            if plus.clicked() {
                *menu_open = !*menu_open;
            }
            new_tab_menu(&plus, shells, hosts, menu_open, &mut action);

            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                if caption_button(ui, "\u{00d7}", true)
                    .on_hover_text("閉じる")
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let zoom = if maximized { "\u{29c9}" } else { "\u{25a1}" };
                if caption_button(ui, zoom, false)
                    .on_hover_text(if maximized {
                        "元に戻す"
                    } else {
                        "最大化"
                    })
                    .clicked()
                {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if caption_button(ui, "\u{2013}", false)
                    .on_hover_text("最小化")
                    .clicked()
                {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.add_space(8.0);
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
        },
    );
    action
}

fn window_drag(ui: &mut Ui, bar: Rect) {
    let response = ui.interact(bar, Id::new("window-drag"), Sense::click_and_drag());
    if response.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if response.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

fn new_tab_menu(
    plus: &egui::Response,
    shells: &[ShellSpec],
    hosts: &[HostEntry],
    menu_open: &mut bool,
    action: &mut Option<TabAction>,
) {
    egui::Popup::menu(plus)
        .open_bool(menu_open)
        .width(240.0)
        .show(|ui| {
            ui.label(
                egui::RichText::new("ローカル")
                    .size(11.0)
                    .color(theme::TEXT_DIM)
                    .strong(),
            );
            if shells.is_empty() {
                ui.label(egui::RichText::new("シェルがありません").color(theme::TEXT_DIM));
            }
            for (i, shell) in shells.iter().enumerate() {
                if ui.button(&shell.name).clicked() {
                    *action = Some(TabAction::OpenLocal(i));
                }
            }
            ui.separator();
            if ui.button("シリアル (COM)").clicked() {
                *action = Some(TabAction::OpenSerial);
            }
            ui.separator();
            ui.label(
                egui::RichText::new("SSH")
                    .size(11.0)
                    .color(theme::TEXT_DIM)
                    .strong(),
            );
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    if hosts.is_empty() {
                        ui.label(egui::RichText::new("ホストがありません").color(theme::TEXT_DIM));
                    }
                    for (i, host) in hosts.iter().enumerate() {
                        if ui.button(&host.alias).on_hover_text(&host.detail).clicked() {
                            *action = Some(TabAction::OpenSsh(i));
                        }
                    }
                });
        });
}

fn caption_button(ui: &mut Ui, glyph: &str, close: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(46.0, BAR_HEIGHT), Sense::click());
    if response.hovered() {
        let fill = if close {
            theme::DANGER
        } else {
            theme::SURFACE_HOVER
        };
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(14.0),
        theme::TEXT,
    );
    response
}

fn tab_widget(
    ui: &mut Ui,
    index: usize,
    tab: &TabInfo,
    active: bool,
    width: f32,
) -> Option<TabAction> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, TAB_HEIGHT), Sense::click());
    let close_rect = Rect::from_center_size(
        egui::pos2(rect.max.x - 14.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let close = ui.interact(
        close_rect,
        ui.id().with(("tab-close", index)),
        Sense::click(),
    );
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
        painter.hline(
            rect.shrink2(Vec2::new(8.0, 0.0)).x_range(),
            rect.max.y - 1.0,
            Stroke::new(2.0, theme::ACCENT),
        );
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
    painter.text(
        close_rect.center(),
        Align2::CENTER_CENTER,
        "\u{00d7}",
        FontId::proportional(14.0),
        theme::TEXT_DIM,
    );

    if close.clicked() || response.middle_clicked() {
        Some(TabAction::Close(index))
    } else if response.clicked() {
        Some(TabAction::Select(index))
    } else {
        response.on_hover_text(&tab.title);
        None
    }
}
