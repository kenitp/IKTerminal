//! Asks whether a typed `ssh` command should be stored in `~/.ssh/config`.

use egui::{RichText, Ui};

use super::theme;
use super::widgets::primary_button;
use crate::sshconfig::{HostDraft, SshCommand};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Alias,
    Hostname,
    User,
    Port,
    Identity,
    Jump,
}

pub enum SshSaveResult {
    Submit(HostDraft),
    Dismissed,
}

pub struct SshSaveDialog {
    summary: String,
    dismiss_key: String,
    alias: String,
    hostname: String,
    user: String,
    port: String,
    identity_file: String,
    proxy_jump: String,
    error: Option<String>,
    focused: bool,
    /// Enter from the shell line that opened the dialog must not confirm it.
    armed: bool,
}

impl SshSaveDialog {
    pub fn new(cmd: &SshCommand) -> Self {
        let draft = HostDraft::from_command(cmd);
        let dismiss_key = format!(
            "{}|{}|{}",
            cmd.destination.to_lowercase(),
            cmd.user.as_deref().unwrap_or("").to_lowercase(),
            cmd.port.unwrap_or(22)
        );
        Self {
            summary: cmd.summary(),
            dismiss_key,
            alias: draft.alias,
            hostname: draft.hostname,
            user: draft.user,
            port: draft.port,
            identity_file: draft.identity_file,
            proxy_jump: draft.proxy_jump,
            error: None,
            focused: false,
            armed: false,
        }
    }

    pub fn dismiss_key(&self) -> &str {
        &self.dismiss_key
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<SshSaveResult> {
        let mut result = None;
        egui::Modal::new(egui::Id::new("ssh-save")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("SSH config に追加");
            ui.add_space(4.0);
            ui.label("この接続を ~/.ssh/config に追加しますか?");
            ui.label(
                RichText::new(&self.summary)
                    .color(theme::TEXT_DIM)
                    .size(12.5),
            );
            ui.add_space(8.0);
            self.fields(ui);
            if let Some(error) = &self.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(theme::DANGER));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let submit = primary_button(ui, "追加").clicked()
                    || (self.armed && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit {
                    result = Some(SshSaveResult::Submit(self.draft()));
                }
                let cancel = ui.button("追加しない").clicked()
                    || (self.armed && ui.input(|i| i.key_pressed(egui::Key::Escape)));
                if cancel {
                    result = Some(SshSaveResult::Dismissed);
                }
            });
            self.armed = true;
        });
        result
    }

    fn fields(&mut self, ui: &mut Ui) {
        self.field(ui, "エイリアス", Field::Alias);
        self.field(ui, "ホスト名", Field::Hostname);
        self.field(ui, "ユーザー", Field::User);
        self.field(ui, "ポート", Field::Port);
        self.field(ui, "秘密鍵", Field::Identity);
        self.field(ui, "踏み台", Field::Jump);
    }

    fn field(&mut self, ui: &mut Ui, label: &str, which: Field) {
        let want_focus = which == Field::Alias && !self.focused;
        ui.horizontal(|ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(label).color(theme::TEXT_DIM));
            let value = match which {
                Field::Alias => &mut self.alias,
                Field::Hostname => &mut self.hostname,
                Field::User => &mut self.user,
                Field::Port => &mut self.port,
                Field::Identity => &mut self.identity_file,
                Field::Jump => &mut self.proxy_jump,
            };
            let response = ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
            if want_focus {
                response.request_focus();
            }
        });
        if want_focus {
            self.focused = true;
        }
    }

    fn draft(&self) -> HostDraft {
        HostDraft {
            alias: self.alias.clone(),
            hostname: self.hostname.clone(),
            user: self.user.clone(),
            port: self.port.clone(),
            identity_file: self.identity_file.clone(),
            proxy_jump: self.proxy_jump.clone(),
        }
    }
}
