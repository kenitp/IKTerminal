//! Right panel: remote file browser and transfer list for an SSH session.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{RichText, Ui};

use super::theme;
use super::widgets::{flat_button, format_size, list_row, primary_button, section_header};
use crate::backend::sftp::{self, Entry, SftpClient, TransferState};

enum Dialog {
    NewFolder(String),
    Rename { from: String, to: String },
    Delete(Entry),
}

/// Per-session UI state of the SFTP panel.
#[derive(Default)]
pub struct SftpView {
    path_input: String,
    selected: Option<String>,
    dialog: Option<Dialog>,
    local_dir: Option<PathBuf>,
    opened: bool,
}

impl SftpView {
    pub fn show(&mut self, ui: &mut Ui, client: &SftpClient) {
        if !client.is_ready() {
            ui.add_space(12.0);
            ui.label(RichText::new("SSH 接続の確立を待っています...").color(theme::TEXT_DIM));
            return;
        }
        if !self.opened {
            self.opened = true;
            client.open_dir(".".to_owned());
        }

        let (cwd, entries, loading, error) = {
            let l = client.listing.lock().unwrap();
            (l.cwd.clone(), l.entries.clone(), l.loading, l.error.clone())
        };
        if !ui.memory(|m| m.has_focus(ui.id().with("path"))) && self.path_input != cwd {
            self.path_input = cwd.clone();
        }

        self.toolbar(ui, client, &cwd);
        let path = ui.add(
            egui::TextEdit::singleline(&mut self.path_input).id(ui.id().with("path")).desired_width(f32::INFINITY),
        );
        if path.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            client.open_dir(self.path_input.trim().to_owned());
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).color(theme::DANGER).size(12.0));
        }
        if loading {
            ui.add(egui::Spinner::new().size(14.0));
        }

        let transfers_height = self.transfer_height(client);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - transfers_height)
            .show(ui, |ui| {
                for entry in &entries {
                    self.entry_row(ui, client, &cwd, entry);
                }
                if entries.is_empty() && !loading {
                    ui.label(RichText::new("(空のフォルダ)").color(theme::TEXT_DIM));
                }
            });
        self.transfers(ui, client);
        self.dialog(ui, client);
    }

    fn toolbar(&mut self, ui: &mut Ui, client: &SftpClient, cwd: &str) {
        ui.horizontal(|ui| {
            if flat_button(ui, "上へ").clicked() {
                client.open_dir(sftp::parent(cwd));
            }
            if flat_button(ui, "更新").clicked() {
                client.refresh();
            }
            if flat_button(ui, "新規フォルダ").clicked() {
                self.dialog = Some(Dialog::NewFolder(String::new()));
            }
            if flat_button(ui, "アップロード").on_hover_text("ファイルをドロップしてもアップロードできます").clicked()
                && let Some(files) = rfd::FileDialog::new().pick_files()
            {
                client.upload(files);
            }
        });
    }

    fn entry_row(&mut self, ui: &mut Ui, client: &SftpClient, cwd: &str, entry: &Entry) {
        let (marker, detail) = if entry.is_dir {
            (theme::ACCENT, "フォルダ".to_owned())
        } else {
            (theme::TEXT_DIM, format_size(entry.size))
        };
        let selected = self.selected.as_deref() == Some(entry.name.as_str());
        let row = list_row(ui, marker, &entry.name, &detail, selected);
        if row.clicked() {
            self.selected = Some(entry.name.clone());
        }
        if row.double_clicked() {
            if entry.is_dir {
                client.open_dir(sftp::join(cwd, &entry.name));
            } else {
                self.download(client, entry);
            }
        }
        row.context_menu(|ui| {
            if entry.is_dir && ui.button("開く").clicked() {
                client.open_dir(sftp::join(cwd, &entry.name));
            }
            if ui.button("ダウンロード...").clicked() {
                self.download(client, entry);
            }
            if ui.button("名前を変更").clicked() {
                self.dialog = Some(Dialog::Rename { from: entry.name.clone(), to: entry.name.clone() });
            }
            if ui.button(RichText::new("削除").color(theme::DANGER)).clicked() {
                self.dialog = Some(Dialog::Delete(entry.clone()));
            }
        });
    }

    fn download(&mut self, client: &SftpClient, entry: &Entry) {
        let mut dialog = rfd::FileDialog::new().set_title("保存先フォルダ");
        if let Some(dir) = &self.local_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dir) = dialog.pick_folder() {
            client.download(vec![entry.clone()], dir.clone());
            self.local_dir = Some(dir);
        }
    }

    fn transfer_height(&self, client: &SftpClient) -> f32 {
        let n = client.transfers.lock().unwrap().len().min(5);
        if n == 0 { 0.0 } else { 40.0 + n as f32 * 46.0 }
    }

    fn transfers(&mut self, ui: &mut Ui, client: &SftpClient) {
        let transfers = client.transfers.lock().unwrap().clone();
        if transfers.is_empty() {
            return;
        }
        ui.separator();
        ui.horizontal(|ui| {
            section_header(ui, "転送");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if flat_button(ui, "完了を消去").clicked() {
                    client.clear_finished();
                }
            });
        });
        egui::ScrollArea::vertical().id_salt("transfers").max_height(5.0 * 46.0).show(ui, |ui| {
            for t in transfers.iter().rev() {
                let total = t.total.load(Ordering::Relaxed);
                let done = t.done.load(Ordering::Relaxed);
                let state = t.state();
                ui.horizontal(|ui| {
                    let arrow = if t.upload { "\u{2191}" } else { "\u{2193}" };
                    ui.label(RichText::new(format!("{arrow} {}", t.label)).size(12.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &state {
                        TransferState::Running => {
                            if flat_button(ui, "中止").clicked() {
                                t.cancel();
                            }
                        }
                        TransferState::Done => {
                            ui.label(RichText::new("完了").color(theme::SUCCESS).size(12.0));
                        }
                        TransferState::Cancelled => {
                            ui.label(RichText::new("中止").color(theme::WARNING).size(12.0));
                        }
                        TransferState::Failed(e) => {
                            ui.label(RichText::new("失敗").color(theme::DANGER).size(12.0)).on_hover_text(e);
                        }
                    });
                });
                let fraction = if total == 0 { 1.0 } else { done as f32 / total as f32 };
                let fill = match state {
                    TransferState::Failed(_) => theme::DANGER,
                    TransferState::Cancelled => theme::WARNING,
                    _ => theme::ACCENT,
                };
                ui.add(
                    egui::ProgressBar::new(fraction.min(1.0))
                        .desired_height(6.0)
                        .fill(fill)
                        .text(RichText::new(format!("{} / {}", format_size(done), format_size(total))).size(10.0)),
                );
            }
        });
    }

    fn dialog(&mut self, ui: &mut Ui, client: &SftpClient) {
        let Some(dialog) = &mut self.dialog else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::new("sftp-dialog")).show(ui.ctx(), |ui| {
            ui.set_width(320.0);
            let confirm = match dialog {
                Dialog::NewFolder(name) => {
                    ui.heading("新規フォルダ");
                    ui.text_edit_singleline(name).request_focus();
                    !name.trim().is_empty()
                }
                Dialog::Rename { to, .. } => {
                    ui.heading("名前を変更");
                    ui.text_edit_singleline(to).request_focus();
                    !to.trim().is_empty()
                }
                Dialog::Delete(entry) => {
                    ui.heading("削除の確認");
                    let what = if entry.is_dir { "フォルダとその中身" } else { "ファイル" };
                    ui.label(format!("{what}「{}」を削除しますか?", entry.name));
                    true
                }
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = ui.add_enabled_ui(confirm, |ui| primary_button(ui, "OK")).inner.clicked()
                    || (confirm && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if ok {
                    match dialog {
                        Dialog::NewFolder(name) => client.mkdir(name.trim()),
                        Dialog::Rename { from, to } => client.rename(from, to.trim()),
                        Dialog::Delete(entry) => client.remove(entry),
                    }
                    close = true;
                }
                if ui.button("キャンセル").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        });
        if close {
            self.dialog = None;
        }
    }
}
