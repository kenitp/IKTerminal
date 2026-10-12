//! Top-level application state and window layout.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use egui::{
    Frame, Key, KeyboardShortcut, Margin, Modifiers, RichText, Ui, ViewportBuilder,
    ViewportCommand, ViewportId,
};

use crate::backend::lan::{self as lan, Account, Device, Range, Scanner};
use crate::backend::local::{self, ShellSpec};
use crate::backend::netif;
use crate::cursor;
use crate::instance::{self, Request};
use crate::session::{Kind, Session};
use crate::settings::Settings;
use crate::sshconfig::{self, HostDraft, HostEntry, SshCommand, SshConfig};
use crate::terminal::Status;
use crate::tray::{self, Resident};
use crate::ui::alias_dialog::{AliasDialog, AliasResult};
use crate::ui::config_editor::{ConfigEditor, EditorResult};
use crate::ui::fonts::{self, TermFont};
use crate::ui::prompt_dialog::PromptDialog;
use crate::ui::scan_dialog::{ScanDialog, ScanResult};
use crate::ui::serial_dialog::{SerialDialog, SerialResult};
use crate::ui::settings_dialog::{SettingsDialog, SettingsResult};
use crate::ui::sftp_panel::SftpView;
use crate::ui::sidebar::{Lan, Sidebar, SidebarAction};
use crate::ui::ssh_login_dialog::{LoginResult, SshLoginDialog};
use crate::ui::ssh_save_dialog::{SshSaveDialog, SshSaveResult};
use crate::ui::tabbar::{self, TabAction, TabInfo};
use crate::ui::{chrome, terminal, theme};
use crate::update::{self, Check, StagedUpdate};

#[derive(Clone)]
struct PendingCursor {
    session: u64,
    desk: u64,
    spec: String,
    host: String,
    hint: Option<String>,
    until: Instant,
}

/// A host from the LAN scan, written to the SSH config once its session is running.
struct PendingSave {
    session: u64,
    desk: u64,
    draft: HostDraft,
}

const NEW_TAB: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::T);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::W);
const PREV_TAB: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);
const SSH_SAVE_WAIT: Duration = Duration::from_secs(180);

struct Spawn {
    pos: Option<egui::Pos2>,
    size: egui::Vec2,
}

#[derive(Clone)]
struct PendingSsh {
    session: u64,
    line: String,
    cmd: SshCommand,
    at: Instant,
}

struct Desk {
    id: u64,
    sessions: Vec<Session>,
    active: usize,
    sftp_views: HashMap<u64, SftpView>,
    sidebar: Sidebar,
    show_sidebar: bool,
    show_sftp: bool,
    serial_dialog: Option<SerialDialog>,
    prompt: PromptDialog,
    ssh_save: Option<SshSaveDialog>,
    login: Option<SshLoginDialog>,
    scan: Option<ScanDialog>,
    alias: Option<AliasDialog>,
    pending_ssh: Option<PendingSsh>,
    notice: Option<String>,
    focus_terminal: bool,
    new_tab_menu: bool,
    /// Working directory for local shells opened in this window. `None` is the home directory.
    directory: Option<PathBuf>,
    window_title: String,
    spawn: Option<Spawn>,
    close: bool,
}

impl Desk {
    fn new(id: u64, directory: Option<PathBuf>, notice: Option<String>) -> Self {
        Self {
            id,
            sessions: Vec::new(),
            active: 0,
            sftp_views: HashMap::new(),
            sidebar: Sidebar::default(),
            show_sidebar: false,
            show_sftp: false,
            serial_dialog: None,
            prompt: PromptDialog::default(),
            ssh_save: None,
            login: None,
            scan: None,
            alias: None,
            pending_ssh: None,
            notice,
            focus_terminal: true,
            new_tab_menu: false,
            directory,
            window_title: String::new(),
            spawn: None,
            close: false,
        }
    }
}

pub struct App {
    settings: Settings,
    ssh_config: SshConfig,
    hosts: Vec<HostEntry>,
    shells: Vec<ShellSpec>,
    desks: Vec<Desk>,
    next_session: u64,
    next_desk: u64,
    focused: u64,
    editor: Option<ConfigEditor>,
    editor_desk: u64,
    settings_dialog: Option<SettingsDialog>,
    settings_desk: u64,
    ssh_dismissed: HashSet<String>,
    /// SSH sessions already checked for an IP-address alias.
    alias_checked: HashSet<u64>,
    /// IP-address aliases the user chose to keep, in lower case.
    alias_kept: HashSet<String>,
    incoming: Receiver<Request>,
    update_rx: Receiver<StagedUpdate>,
    update: Option<StagedUpdate>,
    update_launched: bool,
    update_check: Option<Receiver<Check>>,
    icon: Arc<egui::IconData>,
    tray: Option<Resident>,
    /// The window is hidden and the process stays in the notification area.
    hidden: bool,
    quitting: bool,
    pending_cursor: Option<PendingCursor>,
    lan: Scanner,
    scan_range: Option<Range>,
    scan_auth: Option<Account>,
    pending_saves: Vec<PendingSave>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        icon: Arc<egui::IconData>,
        directory: Option<PathBuf>,
        execute: Option<ShellSpec>,
        notice: Option<String>,
        incoming: Receiver<Request>,
    ) -> Self {
        let settings = Settings::load();
        crate::frame::install(cc);
        let ctx = cc.egui_ctx.clone();
        instance::bind_wake(move || ctx.request_repaint());
        let update_ctx = cc.egui_ctx.clone();
        let update_rx = update::start(move || update_ctx.request_repaint());
        theme::apply(&cc.egui_ctx);
        fonts::install(&cc.egui_ctx, &settings);
        tray::bind(&cc.egui_ctx);
        let ssh_config = SshConfig::load_default();
        let mut app = Self {
            hosts: ssh_config.hosts(),
            ssh_config,
            settings,
            shells: local::detect_shells(),
            desks: vec![Desk::new(0, directory, notice)],
            next_session: 0,
            next_desk: 1,
            focused: 0,
            editor: None,
            editor_desk: 0,
            settings_dialog: None,
            settings_desk: 0,
            ssh_dismissed: HashSet::new(),
            alias_checked: HashSet::new(),
            alias_kept: HashSet::new(),
            incoming,
            update_rx,
            update: None,
            update_launched: false,
            update_check: None,
            icon,
            tray: None,
            hidden: false,
            quitting: false,
            pending_cursor: None,
            lan: Scanner::default(),
            scan_range: None,
            scan_auth: None,
            pending_saves: Vec::new(),
        };
        let shell = execute.or_else(|| app.default_shell());
        if let Some(shell) = shell {
            app.open_shell(&cc.egui_ctx, 0, &shell);
        }
        app.sync_tray(&cc.egui_ctx);
        app
    }

    fn default_shell(&self) -> Option<ShellSpec> {
        ShellSpec::from_command_line(&self.settings.shell).or_else(|| self.shells.first().cloned())
    }

    fn next_session(&mut self) -> u64 {
        self.next_session += 1;
        self.next_session
    }

    fn desk_index(&self, id: u64) -> usize {
        self.desks
            .iter()
            .position(|desk| desk.id == id && !desk.close)
            .unwrap_or(0)
    }

    fn push_session(&mut self, index: usize, session: Session) {
        let desk = &mut self.desks[index];
        desk.sessions.push(session);
        desk.active = desk.sessions.len() - 1;
        desk.focus_terminal = true;
    }

    fn open_shell(&mut self, ctx: &egui::Context, index: usize, shell: &ShellSpec) {
        let directory = self.desks[index].directory.clone();
        let id = self.next_session();
        match Session::local(ctx, &self.settings, id, shell, directory.as_deref()) {
            Ok(session) => self.push_session(index, session),
            Err(e) => {
                self.desks[index].notice = Some(format!("{} を起動できません: {e}", shell.program));
            }
        }
    }

    fn open_default(&mut self, ctx: &egui::Context, index: usize) {
        if let Some(shell) = self.default_shell() {
            self.open_shell(ctx, index, &shell);
        }
    }

    fn connect(&mut self, ctx: &egui::Context, index: usize, target: &str) {
        let id = self.next_session();
        let session = Session::ssh(ctx, &self.settings, id, &self.ssh_config, target, None);
        self.push_session(index, session);
    }

    fn open_scan(&mut self, index: usize, auth: Option<Account>) {
        if self.lan.scanning() {
            return;
        }
        self.scan_auth = auth;
        self.desks[index].scan = Some(ScanDialog::new(
            &netif::interfaces(),
            lan::local_ipv4().ok(),
            self.scan_range,
        ));
    }

    fn scan_dialog(&mut self, ctx: &egui::Context, index: usize) {
        let Some(dialog) = &mut self.desks[index].scan else {
            return;
        };
        match dialog.show(ctx) {
            Some(ScanResult::Start(range)) => {
                self.desks[index].scan = None;
                self.scan_range = Some(range);
                if !self.lan.scanning() {
                    let wake = ctx.clone();
                    let auth = self.scan_auth.clone();
                    self.lan.start(
                        range,
                        self.settings.scan_port,
                        self.settings.scan_names,
                        auth,
                        move || wake.request_repaint(),
                    );
                }
            }
            Some(ScanResult::Closed) => {
                self.desks[index].scan = None;
                self.desks[index].focus_terminal = true;
            }
            None => {}
        }
    }

    fn poll_lan(&mut self) {
        let config = &self.ssh_config;
        self.lan.poll(|device| !is_configured(config, device));
    }

    /// Opens a session to a scanned device; the host is saved after it connects.
    fn connect_device(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        draft: HostDraft,
        password: String,
    ) -> Result<(), String> {
        draft.validate()?;
        if draft.user.is_empty() {
            return Err("ユーザーを入力してください".to_owned());
        }
        if self
            .hosts
            .iter()
            .any(|host| host.alias.eq_ignore_ascii_case(&draft.alias))
        {
            return Err(format!("{} は既に config にあります", draft.alias));
        }
        let target = format!("{}@{}:{}", draft.user, draft.hostname, draft.port);
        let password = (!password.is_empty()).then_some(password);
        let id = self.next_session();
        let session = Session::ssh(ctx, &self.settings, id, &self.ssh_config, &target, password);
        self.push_session(index, session);
        self.pending_saves.push(PendingSave {
            session: id,
            desk: self.desks[index].id,
            draft,
        });
        Ok(())
    }

    fn poll_saves(&mut self) {
        if self.pending_saves.is_empty() {
            return;
        }
        let desks = &self.desks;
        self.pending_saves.retain(|pending| {
            matches!(
                session_status(desks, pending.session),
                Some(Status::Connecting | Status::Running)
            )
        });
        let ready: Vec<PendingSave> = self
            .pending_saves
            .extract_if(.., |pending| {
                session_status(desks, pending.session) == Some(Status::Running)
            })
            .collect();
        if ready.is_empty() {
            return;
        }
        for pending in &ready {
            let message = match sshconfig::save_host(&pending.draft) {
                Ok(()) => format!("{} を SSH config に追加しました", pending.draft.alias),
                Err(error) => format!("SSH config に追加できません: {error}"),
            };
            self.notice(pending.desk, &message);
        }
        self.reload_config();
        for pending in ready {
            self.offer_alias(pending.session, pending.draft.alias.trim());
        }
    }

    /// Asks to rename the config alias of a running SSH session that connected by an IP alias.
    fn poll_aliases(&mut self) {
        let mut found = Vec::new();
        for desk in &self.desks {
            for session in &desk.sessions {
                if !session.is_running_ssh() || !self.alias_checked.insert(session.id) {
                    continue;
                }
                if let Some(target) = session.ssh_target()
                    && sshconfig::is_ip_host(target)
                    && self
                        .hosts
                        .iter()
                        .any(|host| host.alias.eq_ignore_ascii_case(target))
                {
                    found.push((session.id, target.to_owned()));
                }
            }
        }
        for (session, alias) in found {
            self.offer_alias(session, &alias);
        }
    }

    fn offer_alias(&mut self, session: u64, old: &str) {
        self.alias_checked.insert(session);
        if !sshconfig::is_ip_host(old) || self.alias_kept.contains(&old.to_ascii_lowercase()) {
            return;
        }
        let Some((index, tab)) = self.desks.iter().enumerate().find_map(|(index, desk)| {
            let tab = desk.sessions.iter().position(|s| s.id == session)?;
            Some((index, tab))
        }) else {
            return;
        };
        let desk = &mut self.desks[index];
        let Some(suggested) = desk.sessions[tab]
            .login_host()
            .and_then(|host| sshconfig::local_alias(&host))
        else {
            return;
        };
        if desk.alias.is_none() {
            desk.alias = Some(AliasDialog::new(old.to_owned(), suggested));
        }
    }

    fn alias_dialog(&mut self, ctx: &egui::Context, index: usize) {
        let Some(dialog) = &mut self.desks[index].alias else {
            return;
        };
        match dialog.show(ctx) {
            Some(AliasResult::Rename(new)) => {
                let old = dialog.old().to_owned();
                match sshconfig::rename_host(&old, &new) {
                    Ok(()) => {
                        self.desks[index].alias = None;
                        self.desks[index].focus_terminal = true;
                        self.desks[index].notice =
                            Some(format!("エイリアスを {old} から {new} に変更しました"));
                        self.reload_config();
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut self.desks[index].alias {
                            dialog.set_error(error);
                        }
                    }
                }
            }
            Some(AliasResult::Dismissed) => {
                self.alias_kept.insert(dialog.old().to_ascii_lowercase());
                self.desks[index].alias = None;
                self.desks[index].focus_terminal = true;
            }
            None => {}
        }
    }

    fn reconnect(&mut self, ctx: &egui::Context, index: usize, tab: usize) {
        let Kind::Ssh { target, .. } = &self.desks[index].sessions[tab].kind else {
            return;
        };
        let target = target.clone();
        let id = self.next_session();
        let old = std::mem::replace(
            &mut self.desks[index].sessions[tab],
            Session::ssh(ctx, &self.settings, id, &self.ssh_config, &target, None),
        );
        self.desks[index].sftp_views.remove(&old.id);
        self.desks[index].focus_terminal = true;
    }

    fn reopen_serial(&mut self, ctx: &egui::Context, index: usize, tab: usize) {
        let Kind::Serial { port, baud } = &self.desks[index].sessions[tab].kind else {
            return;
        };
        let port = port.clone();
        let baud = *baud;
        let id = self.next_session();
        match Session::serial(ctx, &self.settings, id, &port, baud) {
            Ok(session) => {
                self.desks[index].sessions[tab] = session;
                self.desks[index].focus_terminal = true;
            }
            Err(e) => {
                self.desks[index].notice = Some(format!("{port} を開けません: {e}"));
            }
        }
    }

    fn close_tab(&mut self, index: usize, tab: usize) {
        let Some(session) = take_session(&mut self.desks[index], tab) else {
            return;
        };
        self.desks[index].sftp_views.remove(&session.id);
        if self.desks[index].id != 0 && self.desks[index].sessions.is_empty() {
            self.desks[index].close = true;
        }
    }

    fn move_tab(&mut self, index: usize, from: usize, to: usize) {
        let desk = &mut self.desks[index];
        if from == to || from >= desk.sessions.len() || to >= desk.sessions.len() {
            return;
        }
        let session = desk.sessions.remove(from);
        desk.sessions.insert(to, session);
        desk.active = to;
        desk.focus_terminal = true;
    }

    fn detach(&mut self, index: usize, tab: usize, spawn: Spawn) {
        if self.desks[index].sessions.len() < 2 {
            return;
        }
        let Some(session) = take_session(&mut self.desks[index], tab) else {
            return;
        };
        let view = self.desks[index].sftp_views.remove(&session.id);
        let directory = self.desks[index].directory.clone();
        let id = self.next_desk;
        self.next_desk += 1;
        let mut desk = Desk::new(id, directory, None);
        desk.spawn = Some(spawn);
        let session_id = session.id;
        desk.sessions.push(session);
        if let Some(view) = view {
            desk.sftp_views.insert(session_id, view);
        }
        self.desks.push(desk);
        self.focused = id;
    }

    fn reload_config(&mut self) {
        self.ssh_config = SshConfig::load_default();
        self.hosts = self.ssh_config.hosts();
        let config = &self.ssh_config;
        self.lan.retain(|device| !is_configured(config, device));
    }

    fn open_editor(&mut self, index: usize, focus: Option<&str>) {
        match ConfigEditor::open(focus) {
            Ok(editor) => {
                self.editor_desk = self.desks[index].id;
                self.editor = Some(editor);
            }
            Err(error) => self.desks[index].notice = Some(error),
        }
    }

    fn poll_launches(&mut self, ctx: &egui::Context) {
        let mut focus_id = None;
        while let Ok(request) = self.incoming.try_recv() {
            let index = self.desk_index(self.focused);
            match request {
                Request::Home => {
                    self.desks[index].directory = None;
                    self.open_default(ctx, index);
                }
                Request::Dir(path) => {
                    if path.is_dir() {
                        self.desks[index].directory = Some(path);
                        self.open_default(ctx, index);
                    } else {
                        self.desks[index].notice =
                            Some(format!("{} はフォルダではありません", path.display()));
                    }
                }
                Request::Notice(message) => self.desks[index].notice = Some(message),
            }
            focus_id = Some(self.desks[index].id);
        }
        if let Some(id) = focus_id {
            self.reveal(ctx);
            let viewport = viewport_id(id);
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::Focus);
        }
    }

    fn poll_update(&mut self) {
        if self.update.is_none()
            && let Ok(staged) = self.update_rx.try_recv()
        {
            self.arm_update(staged);
        }
        let Some(rx) = &self.update_check else {
            return;
        };
        let Ok(check) = rx.try_recv() else {
            return;
        };
        self.update_check = None;
        #[cfg(debug_assertions)]
        let message = match check {
            Check::Skipped => "デバッグビルドでは確認しません".to_owned(),
        };
        #[cfg(not(debug_assertions))]
        let message = match check {
            Check::UpToDate => match &self.update {
                Some(update) => {
                    format!(
                        "IkTerminal {} があります。Ver.Up で更新します",
                        update.version
                    )
                }
                None => "最新です".to_owned(),
            },
            Check::Ready(staged) => {
                let version = staged.version.clone();
                self.arm_update(staged);
                format!("IkTerminal {version} があります。Ver.Up で更新します")
            }
            Check::Failed => "確認できませんでした".to_owned(),
        };
        if let Some(dialog) = &mut self.settings_dialog {
            dialog.finish_update_check(message);
        }
    }

    fn arm_update(&mut self, staged: StagedUpdate) {
        if self.update.is_none() {
            self.update = Some(staged);
        }
        self.note_hidden_update();
    }

    fn request_update_check(&mut self, ctx: &egui::Context) {
        if self.update_check.is_some() {
            return;
        }
        let wake = ctx.clone();
        self.update_check = Some(update::check_now(move || wake.request_repaint()));
    }

    fn show_update_banner(&mut self, ui: &mut Ui) {
        if self.update_launched {
            return;
        }
        let text = {
            let Some(update) = &self.update else {
                return;
            };
            format!("IkTerminal {} があります", update.version)
        };
        let ctx = ui.ctx().clone();
        match banner_choice(ui, &text, theme::ACCENT, &["今はしない", "Ver.Up"]) {
            Some(0) => self.discard_update(),
            Some(1) => self.start_update(&ctx),
            _ => {}
        }
    }

    fn start_update(&mut self, ctx: &egui::Context) {
        if self.update_launched {
            return;
        }
        let Some(staged) = self.update.clone() else {
            return;
        };
        match update::apply(&staged) {
            Ok(()) => {
                self.update_launched = true;
                self.quitting = true;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
            Err(error) => {
                self.discard_update();
                self.desks[0].notice = Some(format!("更新できません: {error}"));
            }
        }
    }

    fn sync_tray(&mut self, ctx: &egui::Context) {
        if self.settings.tray {
            if self.tray.is_none() {
                match tray::install(&self.icon.rgba, self.icon.width, self.icon.height) {
                    Ok(resident) => self.tray = Some(resident),
                    Err(error) => self.desks[0].notice = Some(error),
                }
            }
        } else if self.tray.take().is_some() && self.hidden {
            self.reveal(ctx);
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            return;
        }
        while let Some(event) = tray::poll() {
            match event {
                tray::Event::Show => self.reveal(ctx),
                tray::Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    fn reveal(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        for desk in &self.desks {
            if desk.close {
                continue;
            }
            let viewport = viewport_id(desk.id);
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::Minimized(false));
        }
        ctx.send_viewport_cmd_to(viewport_id(self.focused), ViewportCommand::Focus);
    }

    fn apply_visibility(&self, ctx: &egui::Context) {
        if !self.hidden {
            return;
        }
        for desk in &self.desks {
            if desk.close {
                continue;
            }
            ctx.send_viewport_cmd_to(viewport_id(desk.id), ViewportCommand::Visible(false));
        }
    }

    fn note_remote_cd(&mut self, index: usize, line: &str) {
        let Some(session) = self.desks[index]
            .sessions
            .get(self.desks[index].active)
            .filter(|session| session.is_running_ssh())
        else {
            return;
        };
        session.update_remote(|dir| cursor::note_cd(line, dir));
    }

    fn open_cursor(&mut self, index: usize, line: &str, above: Option<&str>) {
        let desk_id = self.desks[index].id;
        let active = self.desks[index].active;
        let Some(session) = self.desks[index].sessions.get(active) else {
            return;
        };
        let Some(target) = session.ssh_target().map(str::to_owned) else {
            return;
        };
        let session_id = session.id;
        let title = session.title();
        let dir = session.remote_dir();
        let Some(spec) = cursor::command(line) else {
            return;
        };
        let hint = cursor::directory_hint(line, above, &title);
        let host = cursor::remote_authority(&self.ssh_config, &target);
        let resolved = cursor::resolve(&spec, hint.as_deref(), &dir);
        {
            let session = &self.desks[index].sessions[active];
            session.write(&b"\x15"[..]);
            if resolved.is_err() {
                session.write(cursor::REPORT_DIRS);
            }
        }
        match resolved {
            Ok(path) => {
                if let Err(error) = cursor::launch(&host, &path) {
                    self.desks[index].notice = Some(error);
                }
            }
            Err(()) => {
                self.pending_cursor = Some(PendingCursor {
                    session: session_id,
                    desk: desk_id,
                    spec,
                    host,
                    hint,
                    until: Instant::now() + Duration::from_secs(2),
                });
            }
        }
    }

    fn poll_cursor(&mut self) {
        let Some(pending) = self.pending_cursor.clone() else {
            return;
        };
        if Instant::now() >= pending.until {
            self.pending_cursor = None;
            self.notice(pending.desk, "カレントフォルダを特定できません");
            return;
        }
        let dir = {
            let Some(session) = self.desks.iter().find_map(|desk| {
                desk.sessions
                    .iter()
                    .find(|session| session.id == pending.session)
            }) else {
                self.pending_cursor = None;
                return;
            };
            session.remote_dir()
        };
        let Ok(path) = cursor::resolve(&pending.spec, pending.hint.as_deref(), &dir) else {
            return;
        };
        self.pending_cursor = None;
        if let Err(error) = cursor::launch(&pending.host, &path) {
            self.notice(pending.desk, &error);
        }
    }

    fn notice(&mut self, desk: u64, message: &str) {
        if let Some(desk) = self.desks.iter_mut().find(|item| item.id == desk) {
            desk.notice = Some(message.to_owned());
        }
    }

    fn discard_update(&mut self) {
        if let Some(staged) = self.update.take() {
            let _ = std::fs::remove_file(staged.path);
        }
        self.note_hidden_update();
    }

    fn note_hidden_update(&self) {
        let Some(tray) = &self.tray else {
            return;
        };
        let text = match &self.update {
            Some(update) if self.hidden => format!("IkTerminal {} があります", update.version),
            _ => "IkTerminal".to_owned(),
        };
        tray.set_tooltip(&text);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, index: usize) {
        let (new_tab, close_tab, prev, next) = ctx.input_mut(|i| {
            let close_tab = i.consume_shortcut(&CLOSE_TAB);
            if close_tab {
                i.events
                    .retain(|event| !matches!(event, egui::Event::Text(text) if text == "\u{17}"));
            }
            (
                i.consume_shortcut(&NEW_TAB),
                close_tab,
                i.consume_shortcut(&PREV_TAB),
                i.consume_shortcut(&NEXT_TAB),
            )
        });
        if new_tab {
            self.desks[index].new_tab_menu = !self.desks[index].new_tab_menu;
        }
        if close_tab {
            self.close_tab(index, self.desks[index].active);
        }
        let n = self.desks[index].sessions.len();
        if n > 0 && (prev || next) {
            let active = &mut self.desks[index].active;
            *active = if next {
                (*active + 1) % n
            } else {
                (*active + n - 1) % n
            };
            self.desks[index].focus_terminal = true;
        }
    }

    /// Local tabs close when their shell exits; SSH tabs stay to offer reconnecting.
    fn close_exited_local(&mut self, index: usize) {
        let exited: Vec<usize> = (0..self.desks[index].sessions.len())
            .filter(|&tab| {
                matches!(self.desks[index].sessions[tab].kind, Kind::Local)
                    && matches!(self.desks[index].sessions[tab].status(), Status::Exited(_))
            })
            .collect();
        for tab in exited.into_iter().rev() {
            self.close_tab(index, tab);
        }
    }

    /// Dropped files are uploaded via SFTP when the panel is open, otherwise their paths are typed.
    fn handle_dropped_files(&mut self, ctx: &egui::Context, index: usize) {
        let paths: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        let Some(session) = self.desks[index]
            .sessions
            .get(self.desks[index].active)
            .filter(|_| !paths.is_empty())
        else {
            return;
        };
        let show_sftp = self.desks[index].show_sftp;
        match session.sftp().filter(|s| show_sftp && s.is_ready()) {
            Some(sftp) => sftp.upload(paths),
            None => {
                let text: Vec<String> = paths
                    .iter()
                    .map(|p| {
                        let s = p.to_string_lossy();
                        if s.contains(' ') {
                            format!("\"{s}\"")
                        } else {
                            s.into_owned()
                        }
                    })
                    .collect();
                session.write(text.join(" ").into_bytes());
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context, index: usize) {
        let title = match self.desks[index].sessions.get(self.desks[index].active) {
            Some(session) => format!("{} - IkTerminal", session.tab_text().0),
            None => "IkTerminal".to_owned(),
        };
        if title != self.desks[index].window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.desks[index].window_title = title;
        }
    }

    fn tab_bar(&mut self, ui: &mut Ui, index: usize) {
        let ctx = ui.ctx().clone();
        let tabs: Vec<TabInfo> = self.desks[index]
            .sessions
            .iter()
            .map(|session| {
                let (title, tooltip) = session.tab_text();
                TabInfo {
                    id: session.id,
                    title,
                    tooltip,
                    status: session.status(),
                }
            })
            .collect();
        let sftp_available = self.desks[index]
            .sessions
            .get(self.desks[index].active)
            .is_some_and(|session| session.sftp().is_some());
        let frame = Frame::new().fill(theme::BG);
        egui::Panel::top(panel_id(ui, "tabbar"))
            .frame(frame)
            .show(ui, |ui| {
                match tabbar::show(
                    ui,
                    tabbar::Bar {
                        tabs: &tabs,
                        active: self.desks[index].active,
                        sftp_available,
                        sftp_open: self.desks[index].show_sftp && sftp_available,
                        shells: &self.shells,
                        hosts: &self.hosts,
                        menu_open: &mut self.desks[index].new_tab_menu,
                    },
                ) {
                    Some(TabAction::Select(tab)) => {
                        self.desks[index].active = tab;
                        self.desks[index].focus_terminal = true;
                    }
                    Some(TabAction::Close(tab)) => self.close_tab(index, tab),
                    Some(TabAction::Move { from, to }) => self.move_tab(index, from, to),
                    Some(TabAction::Detach(tab)) => {
                        let spawn = detach_spawn(ui);
                        self.detach(index, tab, spawn);
                        ui.ctx().request_repaint();
                    }
                    Some(TabAction::OpenLocal(i)) => {
                        if let Some(shell) = self.shells.get(i).cloned() {
                            self.open_shell(&ctx, index, &shell);
                        }
                    }
                    Some(TabAction::OpenSsh(i)) => {
                        if let Some(host) = self.hosts.get(i) {
                            let alias = host.alias.clone();
                            self.connect(&ctx, index, &alias);
                        }
                    }
                    Some(TabAction::OpenSerial) => {
                        self.desks[index].serial_dialog = Some(SerialDialog::new());
                    }
                    Some(TabAction::ToggleSidebar) => {
                        self.desks[index].show_sidebar = !self.desks[index].show_sidebar;
                    }
                    Some(TabAction::ToggleSftp) => {
                        self.desks[index].show_sftp = !self.desks[index].show_sftp;
                    }
                    Some(TabAction::Settings) => {
                        self.settings_desk = self.desks[index].id;
                        self.settings_dialog = Some(SettingsDialog::new(&self.settings));
                    }
                    None => {}
                }
            });
    }

    fn side_panel(&mut self, ui: &mut Ui, index: usize) {
        if !self.desks[index].show_sidebar {
            return;
        }
        let ctx = ui.ctx().clone();
        let frame = Frame::new()
            .fill(theme::PANEL)
            .inner_margin(Margin::same(10));
        egui::Panel::left(panel_id(ui, "sidebar"))
            .resizable(true)
            .default_size(240.0)
            .size_range(180.0..=420.0)
            .frame(frame)
            .show(ui, |ui| {
                let lan = Lan {
                    scanner: &self.lan,
                    port: self.settings.scan_port,
                };
                match self.desks[index]
                    .sidebar
                    .show(ui, &self.shells, &self.hosts, lan)
                {
                    Some(SidebarAction::OpenShell(shell)) => self.open_shell(&ctx, index, &shell),
                    Some(SidebarAction::Connect(target)) => self.connect(&ctx, index, &target),
                    Some(SidebarAction::EditConfig(focus)) => {
                        self.open_editor(index, focus.as_deref())
                    }
                    Some(SidebarAction::Reload) => self.reload_config(),
                    Some(SidebarAction::Scan(auth)) => self.open_scan(index, auth),
                    Some(SidebarAction::PickDevice { device, account }) => {
                        let mut dialog = SshLoginDialog::new(&device);
                        if let Some(account) = account {
                            dialog.prefill(&account.user, &account.password);
                        } else if let Some(auth) = &device.auth {
                            dialog.prefill(&auth.user, "");
                        }
                        self.desks[index].login = Some(dialog);
                    }
                    None => {}
                }
            });
    }

    fn sftp_panel(&mut self, ui: &mut Ui, index: usize) {
        if !self.desks[index].show_sftp {
            return;
        }
        let desk = &mut self.desks[index];
        let active = desk.active;
        let Desk {
            sessions,
            sftp_views,
            ..
        } = desk;
        let Some(session) = sessions.get(active) else {
            return;
        };
        let Some(client) = session.sftp() else { return };
        let view = sftp_views.entry(session.id).or_default();
        let frame = Frame::new()
            .fill(theme::PANEL)
            .inner_margin(Margin::same(10));
        egui::Panel::right(panel_id(ui, "sftp"))
            .resizable(true)
            .default_size(330.0)
            .size_range(240.0..=640.0)
            .frame(frame)
            .show(ui, |ui| view.show(ui, client));
    }

    fn central(&mut self, ui: &mut Ui, index: usize, dialogs_open: bool) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.show_update_banner(ui);
            if let Some(msg) = self.desks[index].notice.clone()
                && banner(ui, &msg, theme::DANGER, &["閉じる"])
            {
                self.desks[index].notice = None;
            }
            if self.desks[index].sessions.is_empty() {
                self.welcome(ui, index);
                return;
            }
            let tab = self.desks[index].active;
            match self.desks[index].sessions[tab].status() {
                Status::Exited(reason)
                    if matches!(self.desks[index].sessions[tab].kind, Kind::Ssh { .. }) =>
                {
                    match banner_choice(ui, &reason, theme::WARNING, &["再接続", "閉じる"]) {
                        Some(0) => self.reconnect(&ctx, index, tab),
                        Some(_) => self.close_tab(index, tab),
                        None => {}
                    }
                }
                Status::Exited(reason)
                    if matches!(self.desks[index].sessions[tab].kind, Kind::Serial { .. }) =>
                {
                    match banner_choice(ui, &reason, theme::WARNING, &["再接続", "閉じる"]) {
                        Some(0) => self.reopen_serial(&ctx, index, tab),
                        Some(_) => self.close_tab(index, tab),
                        None => {}
                    }
                }
                Status::Connecting => {
                    banner(
                        ui,
                        &format!("{} に接続中...", self.desks[index].sessions[tab].title()),
                        theme::ACCENT,
                        &[],
                    );
                }
                _ => {}
            }
            let active = self.desks[index].active;
            let focus_terminal = self.desks[index].focus_terminal;
            let font_size = self.settings.font_size;
            let Some(session) = self.desks[index].sessions.get_mut(active) else {
                return;
            };
            let want_focus =
                !dialogs_open && (focus_terminal || ctx.memory(|m| m.focused().is_none()));
            let font = TermFont::new(&ctx, font_size);
            let out = terminal::show(ui, session, &font, want_focus);
            if want_focus {
                self.desks[index].focus_terminal = false;
            }
            if let Some(line) = out.command_line {
                if out.swallow_enter {
                    let above = out.line_above.clone();
                    self.open_cursor(index, &line, above.as_deref());
                } else {
                    self.note_remote_cd(index, &line);
                    self.note_ssh_command(index, &line);
                }
            }
            if let Some(url) = out.open_url
                && let Err(error) = crate::browser::open(&url)
            {
                self.desks[index].notice = Some(error);
            }
            if out.zoom != 0.0 {
                let range = Settings::FONT_SIZE_RANGE;
                self.settings.font_size =
                    (self.settings.font_size + out.zoom).clamp(*range.start(), *range.end());
                let _ = self.settings.save();
            }
        });
    }

    fn welcome(&mut self, ui: &mut Ui, index: usize) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.label(
                RichText::new("IkTerminal")
                    .size(28.0)
                    .color(theme::ACCENT)
                    .strong(),
            );
            ui.add_space(12.0);
            if ui.button("ローカルシェルを開く").clicked()
                && let Some(shell) = self.default_shell()
            {
                self.open_shell(&ctx, index, &shell);
            }
            if ui.button("SSH config を編集").clicked() {
                self.open_editor(index, None);
            }
        });
    }

    fn dialogs_open(&self, index: usize) -> bool {
        let id = self.desks[index].id;
        self.desks[index].ssh_save.is_some()
            || self.desks[index].login.is_some()
            || self.desks[index].scan.is_some()
            || self.desks[index].alias.is_some()
            || self.desks[index].serial_dialog.is_some()
            || (self.editor.is_some() && self.editor_desk == id)
            || (self.settings_dialog.is_some() && self.settings_desk == id)
    }

    fn dialogs(&mut self, ctx: &egui::Context, index: usize) {
        let desk = &mut self.desks[index];
        desk.prompt.show(ctx, &desk.sessions);
        self.ssh_save_dialog(ctx, index);
        self.login_dialog(ctx, index);
        self.scan_dialog(ctx, index);
        self.alias_dialog(ctx, index);
        if self.editor.is_some()
            && self.editor_desk == self.desks[index].id
            && let Some(editor) = &mut self.editor
        {
            match editor.show(ctx) {
                Some(EditorResult::Saved) => self.reload_config(),
                Some(EditorResult::Closed) => {
                    self.editor = None;
                    self.desks[index].focus_terminal = true;
                }
                None => {}
            }
        }
        let serial_result = self.desks[index]
            .serial_dialog
            .as_mut()
            .and_then(|dialog| dialog.show(ctx));
        match serial_result {
            Some(SerialResult::Open(open)) => {
                let id = self.next_session();
                match Session::serial(ctx, &self.settings, id, &open.port, open.baud) {
                    Ok(session) => {
                        self.push_session(index, session);
                        self.desks[index].serial_dialog = None;
                    }
                    Err(e) => {
                        if let Some(dialog) = &mut self.desks[index].serial_dialog {
                            dialog.set_error(format!("{} を開けません: {e}", open.port));
                        }
                    }
                }
            }
            Some(SerialResult::Closed) => {
                self.desks[index].serial_dialog = None;
                self.desks[index].focus_terminal = true;
            }
            None => {}
        }
        if self.settings_dialog.is_some()
            && self.settings_desk == self.desks[index].id
            && let Some(dialog) = &mut self.settings_dialog
        {
            match dialog.show(ctx, &self.shells) {
                Some(SettingsResult::Saved(new)) => {
                    if new.font_path != self.settings.font_path {
                        fonts::install(ctx, &new);
                    }
                    self.settings = new;
                    self.sync_tray(ctx);
                    self.settings_dialog = None;
                    self.desks[index].focus_terminal = true;
                }
                Some(SettingsResult::Closed) => {
                    self.settings_dialog = None;
                    self.desks[index].focus_terminal = true;
                }
                Some(SettingsResult::CheckUpdate) => self.request_update_check(ctx),
                None => {}
            }
        }
    }

    fn note_ssh_command(&mut self, index: usize, line: &str) {
        if self.desks[index].ssh_save.is_some() {
            return;
        }
        let Some(session) = self.desks[index]
            .sessions
            .get(self.desks[index].active)
            .map(|session| session.id)
        else {
            return;
        };
        let Some(cmd) = sshconfig::find_ssh_command(line) else {
            return;
        };
        let port = cmd.port.unwrap_or(22);
        if self
            .ssh_config
            .contains_target(&cmd.destination, cmd.user.as_deref(), port)
        {
            return;
        }
        if self.ssh_dismissed.contains(&dismiss_key(&cmd)) {
            return;
        }
        self.desks[index].pending_ssh = Some(PendingSsh {
            session,
            line: line.to_owned(),
            cmd,
            at: Instant::now(),
        });
    }

    fn poll_ssh_save(&mut self, index: usize) {
        if self.desks[index].ssh_save.is_some() {
            return;
        }
        let Some(pending) = self.desks[index].pending_ssh.clone() else {
            return;
        };
        if pending.at.elapsed() > SSH_SAVE_WAIT {
            self.desks[index].pending_ssh = None;
            return;
        }
        let screen = {
            let Some(session) = self.desks[index]
                .sessions
                .iter()
                .find(|session| session.id == pending.session)
            else {
                self.desks[index].pending_ssh = None;
                return;
            };
            session.screen_tail(200)
        };
        match sshconfig::ssh_outcome(&pending.line, &screen) {
            sshconfig::SshOutcome::Pending => {}
            sshconfig::SshOutcome::Failed => self.desks[index].pending_ssh = None,
            sshconfig::SshOutcome::Connected { host } => {
                self.desks[index].pending_ssh = None;
                let mut cmd = pending.cmd;
                sshconfig::apply_connected_host(&mut cmd, &host);
                let port = cmd.port.unwrap_or(22);
                let known =
                    self.ssh_config
                        .contains_target(&cmd.destination, cmd.user.as_deref(), port)
                        || self.ssh_config.contains_target(
                            &cmd.hostname,
                            cmd.user.as_deref(),
                            port,
                        );
                if known || self.ssh_dismissed.contains(&dismiss_key(&cmd)) {
                    return;
                }
                self.desks[index].ssh_save = Some(SshSaveDialog::new(&cmd));
            }
        }
    }

    fn ssh_save_dialog(&mut self, ctx: &egui::Context, index: usize) {
        let Some(dialog) = &mut self.desks[index].ssh_save else {
            return;
        };
        let result = dialog.show(ctx);
        match result {
            Some(SshSaveResult::Submit(draft)) => match sshconfig::save_host(&draft) {
                Ok(()) => {
                    if let Some(dialog) = self.desks[index].ssh_save.take() {
                        self.ssh_dismissed.insert(dialog.dismiss_key().to_owned());
                    }
                    self.reload_config();
                    self.desks[index].notice = Some("SSH config に追加しました".to_owned());
                    self.desks[index].focus_terminal = true;
                }
                Err(error) => {
                    if let Some(dialog) = &mut self.desks[index].ssh_save {
                        dialog.set_error(error);
                    }
                }
            },
            Some(SshSaveResult::Dismissed) => {
                if let Some(dialog) = self.desks[index].ssh_save.take() {
                    self.ssh_dismissed.insert(dialog.dismiss_key().to_owned());
                }
                self.desks[index].focus_terminal = true;
            }
            None => {}
        }
    }

    fn login_dialog(&mut self, ctx: &egui::Context, index: usize) {
        let Some(dialog) = &mut self.desks[index].login else {
            return;
        };
        match dialog.show(ctx) {
            Some(LoginResult::Connect { draft, password }) => {
                match self.connect_device(ctx, index, draft, password) {
                    Ok(()) => self.desks[index].login = None,
                    Err(error) => {
                        if let Some(dialog) = &mut self.desks[index].login {
                            dialog.set_error(error);
                        }
                    }
                }
            }
            Some(LoginResult::Closed) => {
                self.desks[index].login = None;
                self.desks[index].focus_terminal = true;
            }
            None => {}
        }
    }

    fn desk_ui(&mut self, ui: &mut Ui, index: usize) {
        if index >= self.desks.len() {
            return;
        }
        let ctx = ui.ctx().clone();
        if ui.input(|i| i.viewport().focused.unwrap_or(false)) {
            self.focused = self.desks[index].id;
        }
        if self.desks[index].id != 0 && ui.input(|i| i.viewport().close_requested()) {
            self.desks[index].close = true;
            return;
        }
        if index == 0 {
            self.poll_tray(&ctx);
            self.poll_cursor();
            self.poll_launches(&ctx);
            self.poll_update();
            self.poll_lan();
            self.poll_saves();
            self.poll_aliases();
            if ui.input(|i| i.viewport().close_requested())
                && self.settings.tray
                && self.tray.is_some()
                && !self.quitting
                && !self.update_launched
            {
                self.hidden = true;
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.note_hidden_update();
            }
        }
        if self.desks.get(index).is_some_and(|desk| desk.close) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.handle_shortcuts(&ctx, index);
        self.close_exited_local(index);
        if self.desks[index].close {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.handle_dropped_files(&ctx, index);
        let dialogs_open = self.dialogs_open(index);
        self.tab_bar(ui, index);
        if self.desks[index].close {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.side_panel(ui, index);
        self.sftp_panel(ui, index);
        self.central(ui, index, dialogs_open);
        self.poll_ssh_save(index);
        self.dialogs(&ctx, index);
        chrome::resize_borders(ui);
        self.update_title(&ctx, index);
    }

    fn present_children(&mut self, ctx: &egui::Context) {
        let pending: Vec<(ViewportId, Option<Spawn>)> = self
            .desks
            .iter_mut()
            .skip(1)
            .map(|desk| (viewport_id(desk.id), desk.spawn.take()))
            .collect();
        for (offset, (id, spawn)) in pending.into_iter().enumerate() {
            let builder = self.child_builder(spawn);
            let index = offset + 1;
            ctx.show_viewport_immediate(id, builder, |ui, _| {
                self.desk_ui(ui, index);
            });
        }
        self.desks.retain(|desk| desk.id == 0 || !desk.close);
    }

    fn child_builder(&self, spawn: Option<Spawn>) -> ViewportBuilder {
        let mut builder = ViewportBuilder::default()
            .with_title("IkTerminal")
            .with_app_id("IkTerminal")
            .with_min_inner_size([480.0, 300.0])
            .with_decorations(false)
            .with_drag_and_drop(true)
            .with_icon(Arc::clone(&self.icon))
            .with_visible(!self.hidden);
        if let Some(spawn) = spawn {
            builder = builder.with_inner_size(spawn.size).with_active(true);
            if let Some(pos) = spawn.pos {
                builder = builder.with_position(pos);
            }
        }
        builder
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        crate::frame::enforce();
        let ctx = ui.ctx().clone();
        self.desk_ui(ui, 0);
        self.present_children(&ctx);
        self.apply_visibility(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::term_bg().to_normalized_gamma_f32()
    }
}

fn viewport_id(desk_id: u64) -> ViewportId {
    if desk_id == 0 {
        ViewportId::ROOT
    } else {
        ViewportId::from_hash_of(("ikterminal-window", desk_id))
    }
}

fn panel_id(ui: &Ui, name: &str) -> egui::Id {
    egui::Id::new((name, ui.ctx().viewport_id()))
}

fn session_status(desks: &[Desk], id: u64) -> Option<Status> {
    desks
        .iter()
        .flat_map(|desk| &desk.sessions)
        .find(|session| session.id == id)
        .map(Session::status)
}

fn is_configured(config: &SshConfig, device: &Device) -> bool {
    config.contains_target(&device.addr.to_string(), None, device.port)
        || device
            .name
            .as_deref()
            .is_some_and(|name| config.contains_target(name, None, device.port))
}

fn take_session(desk: &mut Desk, tab: usize) -> Option<Session> {
    if tab >= desk.sessions.len() {
        return None;
    }
    let session = desk.sessions.remove(tab);
    if desk.active >= desk.sessions.len() {
        desk.active = desk.sessions.len().saturating_sub(1);
    }
    desk.focus_terminal = true;
    Some(session)
}

fn detach_spawn(ui: &Ui) -> Spawn {
    let outer = ui.input(|i| i.viewport().outer_rect);
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let size = ui
        .input(|i| i.viewport().inner_rect.map(|rect| rect.size()))
        .unwrap_or(egui::vec2(1100.0, 700.0));
    let pos = match (outer, pointer) {
        (Some(outer), Some(pointer)) => {
            Some(outer.min + pointer.to_vec2() - egui::vec2(48.0, 18.0))
        }
        (Some(outer), None) => Some(outer.min + egui::vec2(32.0, 32.0)),
        _ => None,
    };
    Spawn { pos, size }
}

fn dismiss_key(cmd: &SshCommand) -> String {
    format!(
        "{}|{}|{}",
        cmd.destination.to_lowercase(),
        cmd.user.as_deref().unwrap_or("").to_lowercase(),
        cmd.port.unwrap_or(22)
    )
}

/// Message strip above the terminal. Returns the index of the clicked button.
fn banner_choice(ui: &mut Ui, text: &str, color: egui::Color32, buttons: &[&str]) -> Option<usize> {
    let mut clicked = None;
    Frame::new()
        .fill(theme::SURFACE)
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
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
