//! Top-level application state and window layout.

use std::collections::HashMap;

use egui::{Frame, Key, KeyboardShortcut, Margin, Modifiers, RichText, Ui};

use crate::backend::local::{self, ShellSpec};
use crate::session::{Kind, Session};
use crate::settings::Settings;
use crate::sshconfig::{HostEntry, SshConfig};
use crate::terminal::Status;
use crate::ui::config_editor::{ConfigEditor, EditorResult};
use crate::ui::fonts::{self, TermFont};
use crate::ui::prompt_dialog::PromptDialog;
use crate::ui::settings_dialog::{SettingsDialog, SettingsResult};
use crate::ui::sftp_panel::SftpView;
use crate::ui::sidebar::{Sidebar, SidebarAction};
use crate::ui::tabbar::{self, TabAction, TabInfo};
use crate::ui::{terminal, theme};

const NEW_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::T);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::W);
const PREV_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);

pub struct App {
    settings: Settings,
    ssh_config: SshConfig,
    hosts: Vec<HostEntry>,
    shells: Vec<ShellSpec>,
    sessions: Vec<Session>,
    active: usize,
    next_id: u64,
    sftp_views: HashMap<u64, SftpView>,
    sidebar: Sidebar,
    show_sidebar: bool,
    show_sftp: bool,
    editor: Option<ConfigEditor>,
    settings_dialog: Option<SettingsDialog>,
    prompt: PromptDialog,
    notice: Option<String>,
    focus_terminal: bool,
    window_title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        theme::apply(&cc.egui_ctx);
        fonts::install(&cc.egui_ctx, &settings);
        let ssh_config = SshConfig::load_default();
        let mut app = Self {
            hosts: ssh_config.hosts(),
            ssh_config,
            settings,
            shells: local::detect_shells(),
            sessions: Vec::new(),
            active: 0,
            next_id: 0,
            sftp_views: HashMap::new(),
            sidebar: Sidebar::default(),
            show_sidebar: true,
            show_sftp: false,
            editor: None,
            settings_dialog: None,
            prompt: PromptDialog::default(),
            notice: None,
            focus_terminal: true,
            window_title: String::new(),
        };
        if let Some(shell) = app.default_shell() {
            app.open_shell(&cc.egui_ctx, &shell);
        }
        app
    }

    fn default_shell(&self) -> Option<ShellSpec> {
        ShellSpec::from_command_line(&self.settings.shell).or_else(|| self.shells.first().cloned())
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn push_session(&mut self, session: Session) {
        self.sessions.push(session);
        self.active = self.sessions.len() - 1;
        self.focus_terminal = true;
    }

    fn open_shell(&mut self, ctx: &egui::Context, shell: &ShellSpec) {
        let id = self.next_id();
        match Session::local(ctx, &self.settings, id, shell) {
            Ok(s) => self.push_session(s),
            Err(e) => self.notice = Some(format!("{} を起動できません: {e}", shell.program)),
        }
    }

    fn connect(&mut self, ctx: &egui::Context, target: &str) {
        let id = self.next_id();
        self.push_session(Session::ssh(ctx, &self.settings, id, &self.ssh_config, target));
    }

    fn reconnect(&mut self, ctx: &egui::Context, index: usize) {
        let Kind::Ssh { target, .. } = &self.sessions[index].kind else { return };
        let target = target.clone();
        let id = self.next_id();
        let old = std::mem::replace(&mut self.sessions[index], Session::ssh(ctx, &self.settings, id, &self.ssh_config, &target));
        self.sftp_views.remove(&old.id);
        self.focus_terminal = true;
    }

    fn close(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        let session = self.sessions.remove(index);
        self.sftp_views.remove(&session.id);
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len().saturating_sub(1);
        }
        self.focus_terminal = true;
    }

    fn reload_config(&mut self) {
        self.ssh_config = SshConfig::load_default();
        self.hosts = self.ssh_config.hosts();
    }

    fn open_editor(&mut self, focus: Option<&str>) {
        match ConfigEditor::open(focus) {
            Ok(e) => self.editor = Some(e),
            Err(e) => self.notice = Some(e),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (new_tab, close_tab, prev, next) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&NEW_TAB),
                i.consume_shortcut(&CLOSE_TAB),
                i.consume_shortcut(&PREV_TAB),
                i.consume_shortcut(&NEXT_TAB),
            )
        });
        if new_tab && let Some(shell) = self.default_shell() {
            self.open_shell(ctx, &shell);
        }
        if close_tab {
            self.close(self.active);
        }
        let n = self.sessions.len();
        if n > 0 && (prev || next) {
            self.active = if next { (self.active + 1) % n } else { (self.active + n - 1) % n };
            self.focus_terminal = true;
        }
    }

    /// Local tabs close when their shell exits; SSH tabs stay to offer reconnecting.
    fn close_exited_local(&mut self) {
        let exited: Vec<usize> = (0..self.sessions.len())
            .filter(|&i| matches!(self.sessions[i].kind, Kind::Local) && matches!(self.sessions[i].status(), Status::Exited(_)))
            .collect();
        for i in exited.into_iter().rev() {
            self.close(i);
        }
    }

    /// Dropped files are uploaded via SFTP when the panel is open, otherwise their paths are typed.
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let paths: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        let Some(session) = self.sessions.get(self.active).filter(|_| !paths.is_empty()) else { return };
        match session.sftp().filter(|s| self.show_sftp && s.is_ready()) {
            Some(sftp) => sftp.upload(paths),
            None => {
                let text: Vec<String> = paths
                    .iter()
                    .map(|p| {
                        let s = p.to_string_lossy();
                        if s.contains(' ') { format!("\"{s}\"") } else { s.into_owned() }
                    })
                    .collect();
                session.write(text.join(" ").into_bytes());
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.sessions.get(self.active) {
            Some(s) => format!("{} - IkTerminal", s.title()),
            None => "IkTerminal".to_owned(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn tab_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let tabs: Vec<TabInfo> = self.sessions.iter().map(|s| TabInfo { title: s.title(), status: s.status() }).collect();
        let sftp_available = self.sessions.get(self.active).is_some_and(|s| s.sftp().is_some());
        let frame = Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(8, 5));
        egui::Panel::top("tabbar").frame(frame).show(ui, |ui| {
            match tabbar::show(ui, &tabs, self.active, sftp_available, self.show_sftp && sftp_available) {
                Some(TabAction::Select(i)) => {
                    self.active = i;
                    self.focus_terminal = true;
                }
                Some(TabAction::Close(i)) => self.close(i),
                Some(TabAction::NewTab) => {
                    if let Some(shell) = self.default_shell() {
                        self.open_shell(&ctx, &shell);
                    }
                }
                Some(TabAction::ToggleSidebar) => self.show_sidebar = !self.show_sidebar,
                Some(TabAction::ToggleSftp) => self.show_sftp = !self.show_sftp,
                Some(TabAction::Settings) => self.settings_dialog = Some(SettingsDialog::new(&self.settings)),
                None => {}
            }
        });
    }

    fn side_panel(&mut self, ui: &mut Ui) {
        if !self.show_sidebar {
            return;
        }
        let ctx = ui.ctx().clone();
        let frame = Frame::new().fill(theme::PANEL).inner_margin(Margin::same(10));
        egui::Panel::left("sidebar").resizable(true).default_size(240.0).size_range(180.0..=420.0).frame(frame).show(
            ui,
            |ui| match self.sidebar.show(ui, &self.shells, &self.hosts) {
                Some(SidebarAction::OpenShell(shell)) => self.open_shell(&ctx, &shell),
                Some(SidebarAction::Connect(target)) => self.connect(&ctx, &target),
                Some(SidebarAction::EditConfig(focus)) => self.open_editor(focus.as_deref()),
                Some(SidebarAction::Reload) => self.reload_config(),
                None => {}
            },
        );
    }

    fn sftp_panel(&mut self, ui: &mut Ui) {
        if !self.show_sftp {
            return;
        }
        let Some(session) = self.sessions.get(self.active) else { return };
        let Some(client) = session.sftp() else { return };
        let view = self.sftp_views.entry(session.id).or_default();
        let frame = Frame::new().fill(theme::PANEL).inner_margin(Margin::same(10));
        egui::Panel::right("sftp").resizable(true).default_size(330.0).size_range(240.0..=640.0).frame(frame).show(
            ui,
            |ui| view.show(ui, client),
        );
    }

    fn central(&mut self, ui: &mut Ui, dialogs_open: bool) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if let Some(msg) = self.notice.clone()
                && banner(ui, &msg, theme::DANGER, &["閉じる"])
            {
                self.notice = None;
            }
            if self.sessions.is_empty() {
                self.welcome(ui);
                return;
            }
            let index = self.active;
            match self.sessions[index].status() {
                Status::Exited(reason) if matches!(self.sessions[index].kind, Kind::Ssh { .. }) => {
                    match banner_choice(ui, &reason, theme::WARNING, &["再接続", "閉じる"]) {
                        Some(0) => self.reconnect(&ctx, index),
                        Some(_) => self.close(index),
                        None => {}
                    }
                }
                Status::Connecting => {
                    banner(ui, &format!("{} に接続中...", self.sessions[index].title()), theme::ACCENT, &[]);
                }
                _ => {}
            }
            let Some(session) = self.sessions.get_mut(self.active) else { return };
            let want_focus = !dialogs_open && (self.focus_terminal || ctx.memory(|m| m.focused().is_none()));
            let font = TermFont::new(&ctx, self.settings.font_size);
            let out = terminal::show(ui, session, &font, want_focus);
            if want_focus {
                self.focus_terminal = false;
            }
            if out.zoom != 0.0 {
                let range = Settings::FONT_SIZE_RANGE;
                self.settings.font_size = (self.settings.font_size + out.zoom).clamp(*range.start(), *range.end());
                let _ = self.settings.save();
            }
        });
    }

    fn welcome(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.label(RichText::new("IkTerminal").size(28.0).color(theme::ACCENT).strong());
            ui.add_space(12.0);
            if ui.button("ローカルシェルを開く").clicked()
                && let Some(shell) = self.default_shell()
            {
                self.open_shell(&ctx, &shell);
            }
            if ui.button("SSH config を編集").clicked() {
                self.open_editor(None);
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.prompt.show(ctx, &self.sessions);
        if let Some(editor) = &mut self.editor {
            match editor.show(ctx) {
                Some(EditorResult::Saved) => self.reload_config(),
                Some(EditorResult::Closed) => {
                    self.editor = None;
                    self.focus_terminal = true;
                }
                None => {}
            }
        }
        if let Some(dialog) = &mut self.settings_dialog {
            match dialog.show(ctx, &self.shells) {
                Some(SettingsResult::Saved(new)) => {
                    if new.font_path != self.settings.font_path {
                        fonts::install(ctx, &new);
                    }
                    self.settings = new;
                    self.settings_dialog = None;
                    self.focus_terminal = true;
                }
                Some(SettingsResult::Closed) => {
                    self.settings_dialog = None;
                    self.focus_terminal = true;
                }
                None => {}
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.close_exited_local();
        self.handle_dropped_files(&ctx);
        let dialogs_open = self.editor.is_some() || self.settings_dialog.is_some();

        self.tab_bar(ui);
        self.side_panel(ui);
        self.sftp_panel(ui);
        self.central(ui, dialogs_open);
        self.dialogs(&ctx);
        self.update_title(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::term_bg().to_normalized_gamma_f32()
    }
}

/// Message strip above the terminal. Returns the index of the clicked button.
fn banner_choice(ui: &mut Ui, text: &str, color: egui::Color32, buttons: &[&str]) -> Option<usize> {
    let mut clicked = None;
    Frame::new().fill(theme::SURFACE).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(text).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (i, label) in buttons.iter().enumerate().rev() {
                    if ui.button(*label).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    });
    clicked
}

fn banner(ui: &mut Ui, text: &str, color: egui::Color32, buttons: &[&str]) -> bool {
    banner_choice(ui, text, color, buttons).is_some()
}
