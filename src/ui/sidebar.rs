//! Left panel: local shells, quick connect and SSH hosts from the config.

use egui::Ui;

use super::theme;
use super::widgets::{flat_button, list_row, section_header};
use crate::backend::local::ShellSpec;
use crate::sshconfig::HostEntry;

pub enum SidebarAction {
    OpenShell(ShellSpec),
    Connect(String),
    EditConfig(Option<String>),
    Reload,
}

#[derive(Default)]
pub struct Sidebar {
    filter: String,
    quick: String,
}

impl Sidebar {
    pub fn show(&mut self, ui: &mut Ui, shells: &[ShellSpec], hosts: &[HostEntry]) -> Option<SidebarAction> {
        let mut action = None;

        section_header(ui, "ローカル");
        for shell in shells {
            if list_row(ui, theme::SUCCESS, &shell.name, "", false).clicked() {
                action = Some(SidebarAction::OpenShell(shell.clone()));
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            section_header(ui, "SSH");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if flat_button(ui, "再読込").on_hover_text("SSH config を再読込").clicked() {
                    action = Some(SidebarAction::Reload);
                }
                if flat_button(ui, "編集").on_hover_text("SSH config を編集").clicked() {
                    action = Some(SidebarAction::EditConfig(None));
                }
            });
        });

        let quick = ui.add(
            egui::TextEdit::singleline(&mut self.quick)
                .hint_text("user@host:port で接続")
                .desired_width(f32::INFINITY),
        );
        if quick.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.quick.trim().is_empty() {
            action = Some(SidebarAction::Connect(self.quick.trim().to_owned()));
            self.quick.clear();
        }
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("ホストを検索").desired_width(f32::INFINITY));
        ui.add_space(2.0);

        let filter = self.filter.to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let visible = hosts
                .iter()
                .filter(|h| filter.is_empty() || h.alias.contains(&filter) || h.detail.to_lowercase().contains(&filter));
            let mut empty = true;
            for host in visible {
                empty = false;
                let row = list_row(ui, theme::ACCENT, &host.alias, &host.detail, false);
                if row.clicked() {
                    action = Some(SidebarAction::Connect(host.alias.clone()));
                }
                row.context_menu(|ui| {
                    if ui.button("接続").clicked() {
                        action = Some(SidebarAction::Connect(host.alias.clone()));
                    }
                    if ui.button("設定を編集").clicked() {
                        action = Some(SidebarAction::EditConfig(Some(host.alias.clone())));
                    }
                });
            }
            if empty {
                ui.label(egui::RichText::new("ホストがありません").color(theme::TEXT_DIM));
            }
        });
        action
    }
}
