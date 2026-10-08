//! Preferences dialog.

use egui::RichText;

use super::theme;
use super::widgets::primary_button;
use crate::backend::local::ShellSpec;
use crate::settings::Settings;

pub enum SettingsResult {
    Saved(Settings),
    Closed,
    CheckUpdate,
}

pub struct SettingsDialog {
    draft: Settings,
    error: Option<String>,
    checking_update: bool,
    update_message: Option<String>,
}

impl SettingsDialog {
    pub fn new(settings: &Settings) -> Self {
        Self {
            draft: settings.clone(),
            error: None,
            checking_update: false,
            update_message: None,
        }
    }

    pub fn finish_update_check(&mut self, message: String) {
        self.checking_update = false;
        self.update_message = Some(message);
    }

    pub fn show(&mut self, ctx: &egui::Context, shells: &[ShellSpec]) -> Option<SettingsResult> {
        let mut result = None;
        egui::Modal::new(egui::Id::new(("settings", ctx.viewport_id()))).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("設定");
            ui.add_space(8.0);
            egui::Grid::new("settings-grid")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("フォントサイズ");
                    ui.add(
                        egui::Slider::new(&mut self.draft.font_size, Settings::FONT_SIZE_RANGE)
                            .step_by(0.5),
                    );
                    ui.end_row();

                    ui.label("フォントファイル");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.draft.font_path)
                                .hint_text("自動 (システムの等幅フォント)")
                                .desired_width(250.0),
                        );
                        if ui.button("参照").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("Font", &["ttf", "otf", "ttc"])
                                .pick_file()
                        {
                            self.draft.font_path = path.to_string_lossy().into_owned();
                        }
                    });
                    ui.end_row();

                    ui.label("既定のシェル");
                    ui.vertical(|ui| {
                        let selected = shells
                            .iter()
                            .find(|s| s.program == self.draft.shell)
                            .map_or_else(
                                || {
                                    if self.draft.shell.is_empty() {
                                        "自動"
                                    } else {
                                        "カスタム"
                                    }
                                    .to_owned()
                                },
                                |s| s.name.clone(),
                            );
                        egui::ComboBox::from_id_salt("shell")
                            .selected_text(selected)
                            .width(250.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.draft.shell, String::new(), "自動");
                                for s in shells {
                                    ui.selectable_value(
                                        &mut self.draft.shell,
                                        s.program.clone(),
                                        &s.name,
                                    );
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut self.draft.shell)
                                .hint_text(if cfg!(windows) {
                                    "コマンドライン (例: pwsh -NoLogo)"
                                } else {
                                    "コマンドライン (例: /bin/bash -l)"
                                })
                                .desired_width(250.0),
                        );
                    });
                    ui.end_row();

                    ui.label("スクロールバック行数");
                    ui.add(
                        egui::DragValue::new(&mut self.draft.scrollback)
                            .range(0..=100_000)
                            .speed(100),
                    );
                    ui.end_row();

                    ui.label("Bitwarden");
                    ui.checkbox(
                        &mut self.draft.launch_bitwarden,
                        "SSH 開始時に、起動していなければ起動する",
                    );
                    ui.end_row();

                    ui.label("タスクトレイ");
                    ui.checkbox(&mut self.draft.tray, "閉じても終了せず、常駐する");
                    ui.end_row();
                });
            ui.label(
                RichText::new("スクロールバック行数は新しいタブから適用されます")
                    .size(11.5)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(format!("バージョン {}", env!("CARGO_PKG_VERSION")));
                if ui
                    .add_enabled(!self.checking_update, egui::Button::new("更新を確認"))
                    .clicked()
                {
                    self.checking_update = true;
                    self.update_message = Some("確認しています".to_owned());
                    result = Some(SettingsResult::CheckUpdate);
                }
            });
            if let Some(message) = &self.update_message {
                ui.label(RichText::new(message).size(11.5).color(theme::TEXT_DIM));
            }
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).color(theme::DANGER));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "保存").clicked() {
                    match self.draft.save() {
                        Ok(()) => result = Some(SettingsResult::Saved(self.draft.clone())),
                        Err(e) => self.error = Some(format!("保存に失敗しました: {e}")),
                    }
                }
                if ui.button("キャンセル").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    result = Some(SettingsResult::Closed);
                }
            });
        });
        result
    }
}
