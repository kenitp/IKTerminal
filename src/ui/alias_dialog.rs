//! Offer to replace an IP-address alias in the SSH config with a host name.

use egui::RichText;

use super::theme;
use super::widgets::primary_button;

pub enum AliasResult {
    Rename(String),
    Dismissed,
}

pub struct AliasDialog {
    old: String,
    alias: String,
    error: Option<String>,
    focused: bool,
}

impl AliasDialog {
    pub fn new(old: String, suggested: String) -> Self {
        Self {
            old,
            alias: suggested,
            error: None,
            focused: false,
        }
    }

    pub fn old(&self) -> &str {
        &self.old
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<AliasResult> {
        let mut result = None;
        egui::Modal::new(egui::Id::new(("ssh-alias", ctx.viewport_id()))).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("エイリアスを変更");
            ui.label(
                RichText::new(format!(
                    "config のエイリアス {} は IP アドレスです。HostName は {} のままにします",
                    self.old, self.old
                ))
                .color(theme::TEXT_DIM)
                .size(12.5),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("新しいエイリアス");
                let edit = ui.add(egui::TextEdit::singleline(&mut self.alias).desired_width(260.0));
                if !self.focused {
                    edit.request_focus();
                    self.focused = true;
                }
            });
            if let Some(error) = &self.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(theme::DANGER));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "変更").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    result = Some(AliasResult::Rename(self.alias.trim().to_owned()));
                }
                if ui.button("今はしない").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    result = Some(AliasResult::Dismissed);
                }
            });
        });
        result
    }
}
