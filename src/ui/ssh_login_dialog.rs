//! Account entry for a device found by the LAN scan.

use egui::RichText;

use super::theme;
use super::widgets::primary_button;
use crate::backend::lan::Device;
use crate::sshconfig::HostDraft;

pub enum LoginResult {
    Connect { draft: HostDraft, password: String },
    Closed,
}

pub struct SshLoginDialog {
    summary: String,
    port: u16,
    alias: String,
    hostname: String,
    user: String,
    password: String,
    error: Option<String>,
    focused: bool,
}

impl SshLoginDialog {
    pub fn new(device: &Device) -> Self {
        let summary = match &device.name {
            Some(name) => format!("{name} ({}:{})", device.addr, device.port),
            None => format!("{}:{}", device.addr, device.port),
        };
        Self {
            summary,
            port: device.port,
            alias: device.alias(),
            hostname: device.addr.to_string(),
            user: String::new(),
            password: String::new(),
            error: None,
            focused: false,
        }
    }

    pub fn prefill(&mut self, user: &str, password: &str) {
        self.user = user.to_owned();
        self.password = password.to_owned();
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<LoginResult> {
        let mut result = None;
        egui::Modal::new(egui::Id::new(("ssh-login", ctx.viewport_id()))).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("SSH で接続");
            ui.label(
                RichText::new(&self.summary)
                    .color(theme::TEXT_DIM)
                    .size(12.5),
            );
            ui.add_space(8.0);
            egui::Grid::new("ssh-login-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    let width = 300.0;
                    ui.label("エイリアス");
                    ui.add(egui::TextEdit::singleline(&mut self.alias).desired_width(width));
                    ui.end_row();
                    ui.label("ホスト名");
                    ui.add(egui::TextEdit::singleline(&mut self.hostname).desired_width(width));
                    ui.end_row();
                    ui.label("ユーザー");
                    let user =
                        ui.add(egui::TextEdit::singleline(&mut self.user).desired_width(width));
                    ui.end_row();
                    ui.label("パスワード");
                    let password = ui.add(
                        egui::TextEdit::singleline(&mut self.password)
                            .password(true)
                            .hint_text("鍵で認証する場合は空")
                            .desired_width(width),
                    );
                    if !self.focused {
                        if self.user.is_empty() {
                            user.request_focus();
                        } else {
                            password.request_focus();
                        }
                        self.focused = true;
                    }
                    ui.end_row();
                });
            ui.label(
                RichText::new("接続できたら ~/.ssh/config に追加します")
                    .size(11.5)
                    .color(theme::TEXT_DIM),
            );
            if let Some(error) = &self.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(theme::DANGER));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "接続").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    result = Some(LoginResult::Connect {
                        draft: self.draft(),
                        password: self.password.clone(),
                    });
                }
                if ui.button("キャンセル").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    result = Some(LoginResult::Closed);
                }
            });
        });
        result
    }

    fn draft(&self) -> HostDraft {
        HostDraft {
            alias: self.alias.trim().to_owned(),
            hostname: self.hostname.trim().to_owned(),
            user: self.user.trim().to_owned(),
            port: self.port.to_string(),
            identity_file: String::new(),
            proxy_jump: String::new(),
        }
    }
}
