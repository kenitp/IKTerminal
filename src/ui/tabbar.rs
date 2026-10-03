//! Top bar: sidebar toggle, tabs, new-tab menu, window controls.

use egui::{
    Align2, FontId, Id, Layout, PointerButton, Rect, Response, Sense, Stroke, Ui, UiBuilder, Vec2,
};

use super::theme;
use super::widgets::flat_button;
use crate::backend::local::ShellSpec;
use crate::sshconfig::HostEntry;
use crate::terminal::Status;

pub struct TabInfo {
    pub id: u64,
    pub title: String,
    pub status: Status,
}

pub enum TabAction {
    Select(usize),
    Close(usize),
    Move {
        from: usize,
        to: usize,
    },
    /// The pointer left the window while dragging this tab.
    Detach(usize),
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
    let window_drag = ui.interact(
        bar,
        Id::new(("window-drag", ui.ctx().viewport_id())),
        Sense::click_and_drag(),
    );

    let mut action = None;
    let mut centers = Vec::new();
    let mut drag_started = None;
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
            let can_drag = tabs.len() > 1;
            for (i, tab) in tabs.iter().enumerate() {
                let painted = tab_widget(ui, i, tab, i == active, tab_width, can_drag);
                centers.push(painted.rect.center().x);
                if painted.drag_started {
                    drag_started = Some(tab.id);
                }
                if let Some(next) = painted.action {
                    action = Some(next);
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
    if let Some(id) = drag_started {
        ui.ctx().data_mut(|data| data.insert_temp(drag_key(ui), id));
        if action.is_none()
            && let Some(index) = tabs.iter().position(|tab| tab.id == id)
        {
            action = Some(TabAction::Select(index));
        }
    }
    if let Some(drag) = track_drag(ui, tabs, &centers) {
        action = Some(drag);
    }
    finish_window_drag(ui, window_drag);
    action
}

/// Neighbor to swap with while `pointer_x` is dragging the tab at `from`.
pub(crate) fn drag_neighbor(from: usize, pointer_x: f32, centers: &[f32]) -> Option<usize> {
    if let Some(next) = centers.get(from + 1)
        && pointer_x > *next
    {
        return Some(from + 1);
    }
    if from > 0
        && let Some(prev) = centers.get(from - 1)
        && pointer_x < *prev
    {
        return Some(from - 1);
    }
    None
}

fn drag_key(ui: &Ui) -> Id {
    Id::new(("tab-drag", ui.ctx().viewport_id()))
}

fn track_drag(ui: &Ui, tabs: &[TabInfo], centers: &[f32]) -> Option<TabAction> {
    let key = drag_key(ui);
    let dragging = ui.ctx().data(|data| data.get_temp::<u64>(key))?;
    let Some(from) = tabs.iter().position(|tab| tab.id == dragging) else {
        ui.ctx().data_mut(|data| {
            data.remove_temp::<u64>(key);
        });
        return None;
    };
    let pos = ui.input(|i| i.pointer.latest_pos());
    let down = ui.input(|i| i.pointer.any_down());
    if pos.is_none() {
        ui.ctx().data_mut(|data| {
            data.remove_temp::<u64>(key);
        });
        return (tabs.len() > 1).then_some(TabAction::Detach(from));
    }
    if !down {
        ui.ctx().data_mut(|data| {
            data.remove_temp::<u64>(key);
        });
        return None;
    }
    ui.ctx().data_mut(|data| data.insert_temp(key, dragging));
    let pos = pos?;
    drag_neighbor(from, pos.x, centers).map(|to| TabAction::Move { from, to })
}

fn finish_window_drag(ui: &Ui, response: Response) {
    if ui
        .ctx()
        .data(|data| data.get_temp::<u64>(drag_key(ui)))
        .is_some()
    {
        return;
    }
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

struct TabPaint {
    action: Option<TabAction>,
    rect: Rect,
    drag_started: bool,
}

fn tab_widget(
    ui: &mut Ui,
    index: usize,
    tab: &TabInfo,
    active: bool,
    width: f32,
    can_drag: bool,
) -> TabPaint {
    let sense = if can_drag {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, TAB_HEIGHT), Sense::hover());
    let response = ui.interact(
        rect,
        Id::new(("tab", ui.ctx().viewport_id(), tab.id)),
        sense,
    );
    let close_rect = Rect::from_center_size(
        egui::pos2(rect.max.x - 14.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let close = ui.interact(
        close_rect,
        Id::new(("tab-close", ui.ctx().viewport_id(), tab.id)),
        Sense::click_and_drag(),
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

    let clicked_close = close.clicked();
    let middle = response.middle_clicked();
    let clicked = response.clicked();
    let drag_started = can_drag && response.drag_started() && !close.drag_started();
    let response = response.on_hover_text(&tab.title);
    if can_drag {
        response.on_hover_cursor(egui::CursorIcon::Grab);
    }
    let action = if clicked_close || middle {
        Some(TabAction::Close(index))
    } else if clicked {
        Some(TabAction::Select(index))
    } else {
        None
    };
    TabPaint {
        action,
        rect,
        drag_started,
    }
}

#[cfg(test)]
mod tests {
    use super::drag_neighbor;

    #[test]
    fn drag_swaps_with_the_neighbor_past_its_center() {
        let centers = [50.0, 150.0, 250.0];
        assert_eq!(drag_neighbor(0, 160.0, &centers), Some(1));
        assert_eq!(drag_neighbor(0, 140.0, &centers), None);
        assert_eq!(drag_neighbor(1, 40.0, &centers), Some(0));
        assert_eq!(drag_neighbor(1, 260.0, &centers), Some(2));
        assert_eq!(drag_neighbor(2, 140.0, &centers), Some(1));
        assert_eq!(drag_neighbor(2, 160.0, &centers), None);
    }
}
