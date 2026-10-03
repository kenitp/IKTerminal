//! A terminal tab: emulator state plus the backend feeding it.

use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::Term;
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config;

use crate::backend::local::{self, ShellSpec};
use crate::backend::sftp::SftpClient;
use crate::backend::ssh;
use crate::settings::Settings;
use crate::sshconfig::SshConfig;
use crate::terminal::{GridSize, Listener, Prompt, Shared, Status, TermHandle};

const INITIAL: GridSize = GridSize { cols: 80, rows: 24 };

pub enum Kind {
    Local,
    Ssh { target: String, sftp: SftpClient },
}

pub struct Session {
    pub id: u64,
    pub kind: Kind,
    pub term: TermHandle,
    shared: Arc<Shared>,
    grid: GridSize,
}

impl Session {
    fn create(ctx: &egui::Context, settings: &Settings, title: String) -> (Arc<Shared>, TermHandle, Listener) {
        let size = WindowSize { num_lines: INITIAL.rows as u16, num_cols: INITIAL.cols as u16, cell_width: 8, cell_height: 16 };
        let shared = Shared::new(ctx.clone(), title, size);
        let listener = Listener(shared.clone());
        let config = Config { scrolling_history: settings.scrollback, ..Config::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &INITIAL, listener.clone())));
        (shared, term, listener)
    }

    pub fn local(ctx: &egui::Context, settings: &Settings, id: u64, shell: &ShellSpec) -> std::io::Result<Self> {
        let (shared, term, listener) = Self::create(ctx, settings, shell.name.clone());
        local::spawn(term.clone(), listener, shell)?;
        Ok(Self { id, kind: Kind::Local, term, shared, grid: INITIAL })
    }

    pub fn ssh(ctx: &egui::Context, settings: &Settings, id: u64, config: &SshConfig, target: &str) -> Self {
        let host = config.resolve(target);
        let jumps = host.proxy_jump.iter().map(|j| config.resolve(j)).collect();
        let (shared, term, _) = Self::create(ctx, settings, host.alias.clone());
        let link = ssh::spawn(shared.clone(), term.clone(), host, jumps);
        let sftp = SftpClient::new(link, ctx.clone());
        Self { id, kind: Kind::Ssh { target: target.to_owned(), sftp }, term, shared, grid: INITIAL }
    }

    /// Window title set by the program; executable paths are shortened to their name.
    pub fn title(&self) -> String {
        let title = self.shared.title();
        if title.to_ascii_lowercase().ends_with(".exe") {
            let path = std::path::Path::new(&title);
            if let Some(stem) = path.file_stem() {
                return stem.to_string_lossy().into_owned();
            }
        }
        title
    }

    pub fn status(&self) -> Status {
        self.shared.status()
    }

    pub fn sftp(&self) -> Option<&SftpClient> {
        match &self.kind {
            Kind::Ssh { sftp, .. } => Some(sftp),
            Kind::Local => None,
        }
    }

    pub fn take_prompt(&self) -> Option<Prompt> {
        self.shared.take_prompt()
    }

    pub fn write(&self, data: impl Into<Cow<'static, [u8]>>) {
        self.shared.write(data.into());
    }

    /// Applies a new grid size to both the emulator and the backend.
    pub fn resize(&mut self, cols: usize, rows: usize, cell_width: f32, cell_height: f32) {
        if (cols, rows) == (self.grid.cols, self.grid.rows) || cols == 0 || rows == 0 {
            return;
        }
        self.grid = GridSize { cols, rows };
        self.term.lock().resize(self.grid);
        self.shared.resize(WindowSize {
            num_lines: rows as u16,
            num_cols: cols as u16,
            cell_width: cell_width as u16,
            cell_height: cell_height as u16,
        });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.shutdown();
    }
}
