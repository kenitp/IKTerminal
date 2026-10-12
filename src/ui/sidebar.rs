//! Left panel: local shells, quick connect, SSH hosts from the config and the LAN scan.

use egui::{Color32, Ui};

use super::theme;
use super::widgets::{flat_button, list_row, section_header};
use crate::backend::lan::{Account, AuthState, Device, Scanner};
use crate::backend::local::ShellSpec;
use crate::sshconfig::HostEntry;

pub enum SidebarAction {
    OpenShell(ShellSpec),
    Connect(String),
    EditConfig(Option<String>),
    Reload,
    Scan(Option<Account>),
    PickDevice {
        device: Device,
        account: Option<Account>,
    },
}

pub struct Lan<'a> {
    pub scanner: &'a Scanner,
    pub port: u16,
}

#[derive(Default)]
pub struct Sidebar {
    filter: String,
    quick: String,
    try_auth: bool,
    auth_user: String,
    auth_password: String,
    /// Account from the latest scan that tried SSH authentication.
    auth_used: Option<Account>,
}

impl Sidebar {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        shells: &[ShellSpec],
        hosts: &[HostEntry],
        lan: Lan,
    ) -> Option<SidebarAction> {
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
                if flat_button(ui, "再読込")
                    .on_hover_text("SSH config を再読込")
                    .clicked()
                {
                    action = Some(SidebarAction::Reload);
                }
                if flat_button(ui, "編集")
                    .on_hover_text("SSH config を編集")
                    .clicked()
                {
                    action = Some(SidebarAction::EditConfig(None));
                }
            });
        });

        let quick = ui.add(
            egui::TextEdit::singleline(&mut self.quick)
                .hint_text("user@host:port で接続")
                .desired_width(f32::INFINITY),
        );
        if quick.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !self.quick.trim().is_empty()
        {
            action = Some(SidebarAction::Connect(self.quick.trim().to_owned()));
            self.quick.clear();
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("ホストを検索")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(2.0);

        let filter = self.filter.to_lowercase();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let visible = hosts.iter().filter(|h| {
                    filter.is_empty()
                        || h.alias.contains(&filter)
                        || h.detail.to_lowercase().contains(&filter)
                });
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
                if let Some(picked) = self.lan_section(ui, &lan, &filter) {
                    action = Some(picked);
                }
            });
        action
    }
}

impl Sidebar {
    fn lan_section(&mut self, ui: &mut Ui, lan: &Lan, filter: &str) -> Option<SidebarAction> {
        let mut action = None;
        let scanning = lan.scanner.scanning();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            section_header(ui, &format!("LAN (ポート {})", lan.port));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if scanning {
                    ui.spinner();
                } else if flat_button(ui, "スキャン")
                    .on_hover_text("範囲を選び、ポートが開いている機器を探す")
                    .clicked()
                {
                    let auth = self.auth_request();
                    if auth.is_some() {
                        self.auth_used = auth.clone();
                    }
                    action = Some(SidebarAction::Scan(auth));
                }
            });
        });
        ui.add_enabled_ui(!scanning, |ui| {
            ui.checkbox(&mut self.try_auth, "SSH認証を試す")
                .on_hover_text("見つかった機器に、指定したユーザーとパスワードで接続できるか試す");
        });
        if self.try_auth {
            ui.add(
                egui::TextEdit::singleline(&mut self.auth_user)
                    .hint_text("ユーザー")
                    .desired_width(f32::INFINITY)
                    .interactive(!scanning),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.auth_password)
                    .password(true)
                    .hint_text("パスワード")
                    .desired_width(f32::INFINITY)
                    .interactive(!scanning),
            );
            if self.auth_user.trim().is_empty() {
                ui.label(
                    egui::RichText::new("ユーザーが空のときはポートの確認だけします")
                        .size(10.5)
                        .color(theme::TEXT_DIM),
                );
            }
        }
        if let Some((addr, done, total)) = lan.scanner.progress() {
            ui.label(
                egui::RichText::new(format!("{addr}  {done} / {total}"))
                    .size(10.5)
                    .color(theme::TEXT_DIM.gamma_multiply(0.7)),
            );
        }
        for device in &lan.scanner.devices {
            let label = device.label();
            let detail = device_detail(device);
            if !filter.is_empty()
                && !label.to_lowercase().contains(filter)
                && !detail.to_lowercase().contains(filter)
            {
                continue;
            }
            if list_row(ui, device_marker(device), &label, &detail, false)
                .on_hover_text(device_hover(device))
                .clicked()
            {
                action = Some(SidebarAction::PickDevice {
                    account: self.account_for(device),
                    device: device.clone(),
                });
            }
        }
        if let Some(message) = &lan.scanner.message {
            ui.label(egui::RichText::new(message).color(theme::TEXT_DIM));
        }
        action
    }

    fn auth_request(&self) -> Option<Account> {
        let user = self.auth_user.trim();
        if !self.try_auth || user.is_empty() {
            return None;
        }
        Some(Account {
            user: user.to_owned(),
            password: self.auth_password.clone(),
        })
    }

    fn account_for(&self, device: &Device) -> Option<Account> {
        let used = self.auth_used.as_ref()?;
        let auth = device.auth.as_ref()?;
        (auth.user == used.user).then(|| used.clone())
    }
}

fn device_detail(device: &Device) -> String {
    let place = if device.name.is_some() {
        format!("{}:{}", device.addr, device.port)
    } else {
        format!("ポート {}", device.port)
    };
    match &device.auth {
        Some(auth) => match &auth.state {
            AuthState::Checking => format!("{place}  認証を確認中"),
            AuthState::Connected => format!("{place}  {} で接続できました", auth.user),
            AuthState::Rejected(_) => format!("{place}  認証できません"),
        },
        None => place,
    }
}

fn device_marker(device: &Device) -> Color32 {
    match device.auth.as_ref().map(|auth| &auth.state) {
        Some(AuthState::Connected) => theme::SUCCESS,
        Some(AuthState::Rejected(_)) => theme::DANGER,
        _ => theme::WARNING,
    }
}

fn device_hover(device: &Device) -> String {
    match &device.auth {
        Some(auth) => match &auth.state {
            AuthState::Connected => format!("{} で接続できました。クリックで接続", auth.user),
            AuthState::Rejected(reason) => reason.clone(),
            AuthState::Checking => "認証を確認しています".to_owned(),
        },
        None => "ユーザーとパスワードを入力して接続".to_owned(),
    }
}
