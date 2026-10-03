//! Editor window for `~/.ssh/config` (form view and raw text view).

use std::path::PathBuf;

use egui::{RichText, Ui};

use super::theme;
use super::widgets::{flat_button, list_row, primary_button};
use crate::sshconfig::{Document, Line, SshConfig, find_option, push_option, ssh_dir};

/// Options with a dedicated form field: (key, label, hint).
const FIELDS: [(&str, &str, &str); 5] = [
    ("HostName", "ホスト名", "example.com / 192.168.0.10"),
    ("User", "ユーザー", "未指定ならローカルのユーザー名"),
    ("Port", "ポート", "22"),
    ("IdentityFile", "秘密鍵", "~/.ssh/id_ed25519"),
    ("ProxyJump", "踏み台", "user@bastion"),
];

#[derive(PartialEq)]
enum Mode {
    Form,
    Text,
}

pub enum EditorResult {
    Saved,
    Closed,
}

pub struct ConfigEditor {
    path: PathBuf,
    doc: Document,
    text: String,
    mode: Mode,
    /// `None` selects the global section.
    selected: Option<usize>,
    status: Option<(String, bool)>,
}

impl ConfigEditor {
    pub fn open(focus: Option<&str>) -> Result<Self, String> {
        let path = SshConfig::default_path().ok_or("ホームディレクトリが見つかりません")?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let doc = Document::parse(&text);
        let selected = focus
            .and_then(|alias| {
                doc.blocks
                    .iter()
                    .position(|b| b.is_host() && b.patterns.split_whitespace().any(|p| p.eq_ignore_ascii_case(alias)))
            })
            .or(if doc.blocks.is_empty() { None } else { Some(0) });
        Ok(Self { path, doc, text, mode: Mode::Form, selected, status: None })
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<EditorResult> {
        let mut open = true;
        let mut result = None;
        egui::Window::new("SSH Config の編集")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([780.0, 540.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.path.display().to_string()).color(theme::TEXT_DIM).size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, "保存").clicked() {
                            result = self.save().then_some(EditorResult::Saved);
                        }
                        let text = ui.selectable_label(self.mode == Mode::Text, "テキスト");
                        let form = ui.selectable_label(self.mode == Mode::Form, "フォーム");
                        if text.clicked() && self.mode == Mode::Form {
                            self.text = self.doc.to_text();
                            self.mode = Mode::Text;
                        }
                        if form.clicked() && self.mode == Mode::Text {
                            self.doc = Document::parse(&self.text);
                            self.selected = self.selected.filter(|i| *i < self.doc.blocks.len());
                            self.mode = Mode::Form;
                        }
                    });
                });
                if let Some((msg, error)) = &self.status {
                    ui.label(RichText::new(msg).color(if *error { theme::DANGER } else { theme::SUCCESS }));
                }
                ui.separator();
                match self.mode {
                    Mode::Form => self.form(ui),
                    Mode::Text => {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.text)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(24),
                            );
                        });
                    }
                }
            });
        if !open {
            result = Some(EditorResult::Closed);
        }
        result
    }

    fn save(&mut self) -> bool {
        if self.mode == Mode::Text {
            self.doc = Document::parse(&self.text);
        }
        let text = self.doc.to_text();
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if self.path.exists() {
                std::fs::copy(&self.path, self.path.with_extension("bak"))?;
            }
            std::fs::write(&self.path, &text)
        };
        let ok = write().is_ok();
        self.status = Some(match ok {
            true => ("保存しました (以前の内容は config.bak に退避)".to_owned(), false),
            false => ("保存に失敗しました".to_owned(), true),
        });
        if ok && self.mode == Mode::Text {
            self.text = text;
        }
        ok
    }

    fn form(&mut self, ui: &mut Ui) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(210.0);
                self.block_list(ui);
            });
            ui.separator();
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().id_salt("block-form").auto_shrink([false, false]).show(ui, |ui| {
                    match self.selected {
                        None => {
                            ui.label(RichText::new("全ホスト共通の設定").strong());
                            ui.add_space(4.0);
                            options_form(ui, &mut self.doc.global);
                        }
                        Some(i) => {
                            let block = &mut self.doc.blocks[i];
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&block.keyword).strong());
                                ui.add(
                                    egui::TextEdit::singleline(&mut block.patterns)
                                        .hint_text("エイリアス (スペース区切り / * ? ! 可)")
                                        .desired_width(f32::INFINITY),
                                );
                            });
                            ui.add_space(4.0);
                            options_form(ui, &mut block.lines);
                        }
                    }
                });
            });
        });
    }

    fn block_list(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if flat_button(ui, "+ ホスト追加").clicked() {
                let idx = self.doc.add_host("new-host");
                self.selected = Some(idx);
            }
            if let Some(i) = self.selected
                && flat_button(ui, "削除").clicked()
            {
                self.doc.blocks.remove(i);
                self.selected = if self.doc.blocks.is_empty() { None } else { Some(i.saturating_sub(1)) };
            }
        });
        egui::ScrollArea::vertical().id_salt("block-list").auto_shrink([false, false]).show(ui, |ui| {
            if list_row(ui, theme::WARNING, "共通設定", "", self.selected.is_none()).clicked() {
                self.selected = None;
            }
            for (i, block) in self.doc.blocks.iter().enumerate() {
                let detail = find_option(&block.lines, "HostName")
                    .and_then(|idx| match &block.lines[idx] {
                        Line::Option { value, .. } => Some(value.as_str()),
                        Line::Raw(_) => None,
                    })
                    .unwrap_or(if block.is_host() { "" } else { "Match" });
                let marker = if block.is_host() { theme::ACCENT } else { theme::TEXT_DIM };
                let title = if block.patterns.trim().is_empty() { "(無名)" } else { block.patterns.as_str() };
                if list_row(ui, marker, title, detail, self.selected == Some(i)).clicked() {
                    self.selected = Some(i);
                }
            }
        });
    }
}

fn field_edit<'a>(value: &'a mut String, key: &str, hint: &str, width: f32) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(value).id_salt(key).hint_text(hint).desired_width(width)
}

/// Known fields plus a generic key/value list for every other option.
fn options_form(ui: &mut Ui, lines: &mut Vec<Line>) {
    egui::Grid::new("known-fields").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        for (key, label, hint) in FIELDS {
            ui.label(label).on_hover_text(key);
            ui.horizontal(|ui| {
                let width = if key == "IdentityFile" { ui.available_width() - 60.0 } else { ui.available_width() };
                match find_option(lines, key) {
                    Some(idx) => {
                        if let Line::Option { value, .. } = &mut lines[idx] {
                            ui.add(field_edit(value, key, hint, width));
                        }
                    }
                    None => {
                        let mut value = String::new();
                        if ui.add(field_edit(&mut value, key, hint, width)).changed() && !value.is_empty() {
                            push_option(lines, key, &value);
                        }
                    }
                }
                if key == "IdentityFile" && ui.button("参照").clicked() {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(dir) = ssh_dir() {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(path) = dialog.pick_file() {
                        let value = path.to_string_lossy().into_owned();
                        match find_option(lines, key) {
                            Some(idx) => lines[idx] = Line::option(key, &value),
                            None => push_option(lines, key, &value),
                        }
                    }
                }
            });
            ui.end_row();
        }
    });

    ui.add_space(10.0);
    ui.label(RichText::new("その他のオプション").strong());
    let first_known: Vec<usize> = FIELDS.iter().filter_map(|(k, _, _)| find_option(lines, k)).collect();
    let mut remove = None;
    egui::Grid::new("other-options").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
        for (i, line) in lines.iter_mut().enumerate() {
            if first_known.contains(&i) {
                continue;
            }
            let Line::Option { key, value, .. } = line else { continue };
            ui.add(egui::TextEdit::singleline(key).id_salt(("k", i)).hint_text("キー").desired_width(150.0));
            ui.add(egui::TextEdit::singleline(value).id_salt(("v", i)).hint_text("値").desired_width(260.0));
            if flat_button(ui, "\u{00d7}").on_hover_text("削除").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        lines.remove(i);
    }
    if flat_button(ui, "+ オプション追加").clicked() {
        push_option(lines, "", "");
    }
    ui.label(
        RichText::new("値が空のオプションは保存時に削除されます。コメント行はそのまま保持されます。")
            .size(11.5)
            .color(theme::TEXT_DIM),
    );
}
