//! Modal for questions raised by backends (passwords, host keys).

use egui::RichText;

use super::theme;
use super::widgets::primary_button;
use crate::session::Session;
use crate::terminal::{Prompt, PromptKind};

#[derive(Default)]
pub struct PromptDialog {
    current: Option<Prompt>,
    input: String,
}

impl PromptDialog {
    pub fn show(&mut self, ctx: &egui::Context, sessions: &[Session]) {
        if self.current.as_ref().is_some_and(|p| p.reply.is_closed()) {
            self.current = None;
        }
        if self.current.is_none() {
            self.current = sessions.iter().find_map(Session::take_prompt);
            self.input.clear();
        }
        let Some(prompt) = &self.current else { return };

        let mut answer: Option<Option<String>> = None;
        egui::Modal::new(egui::Id::new(("prompt", ctx.viewport_id()))).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(&prompt.title);
            ui.add_space(4.0);
            ui.label(RichText::new(&prompt.message).color(theme::TEXT));
            ui.add_space(8.0);
            let enter = if prompt.kind == PromptKind::Confirm {
                false
            } else {
                let edit = egui::TextEdit::singleline(&mut self.input)
                    .password(prompt.kind == PromptKind::Secret)
                    .desired_width(f32::INFINITY);
                ui.add(edit).request_focus();
                ui.input(|i| i.key_pressed(egui::Key::Enter))
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok_label = if prompt.kind == PromptKind::Confirm {
                    "信頼して接続"
                } else {
                    "OK"
                };
                if primary_button(ui, ok_label).clicked() || enter {
                    answer = Some(Some(std::mem::take(&mut self.input)));
                }
                if ui.button("キャンセル").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    answer = Some(None);
                }
            });
        });

        if let Some(answer) = answer
            && let Some(prompt) = self.current.take()
        {
            let _ = prompt.reply.send(answer);
            self.input.clear();
        }
    }
}
