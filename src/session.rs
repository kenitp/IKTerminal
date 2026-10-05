//! A terminal tab: emulator state plus the backend feeding it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::Term;
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config;

use crate::backend::local::{self, ShellSpec};
use crate::backend::sftp::SftpClient;
use crate::backend::{serial, ssh};
use crate::settings::Settings;
use crate::sshconfig::SshConfig;
use crate::terminal::{GridSize, Listener, Prompt, Shared, Status, TermHandle};

const INITIAL: GridSize = GridSize { cols: 80, rows: 24 };
const DIRECTORY_REFRESH: Duration = Duration::from_millis(500);

struct DirectoryState {
    pid: Option<u32>,
    path: Option<PathBuf>,
    refreshed: Option<Instant>,
}

impl DirectoryState {
    fn idle() -> Mutex<Self> {
        Mutex::new(Self {
            pid: None,
            path: None,
            refreshed: None,
        })
    }
}

pub enum Kind {
    Local,
    Ssh { target: String, sftp: SftpClient },
    Serial { port: String, baud: u32 },
}

pub struct Session {
    pub id: u64,
    pub kind: Kind,
    pub term: TermHandle,
    shared: Arc<Shared>,
    directory: Mutex<DirectoryState>,
    grid: GridSize,
}

impl Session {
    fn create(
        ctx: &egui::Context,
        settings: &Settings,
        title: String,
    ) -> (Arc<Shared>, TermHandle, Listener) {
        let size = WindowSize {
            num_lines: INITIAL.rows as u16,
            num_cols: INITIAL.cols as u16,
            cell_width: 8,
            cell_height: 16,
        };
        let shared = Shared::new(ctx.clone(), title, size);
        let listener = Listener(shared.clone());
        let config = Config {
            scrolling_history: settings.scrollback,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &INITIAL,
            listener.clone(),
        )));
        (shared, term, listener)
    }

    pub fn local(
        ctx: &egui::Context,
        settings: &Settings,
        id: u64,
        shell: &ShellSpec,
        directory: Option<&Path>,
    ) -> std::io::Result<Self> {
        let (shared, term, listener) = Self::create(ctx, settings, shell.name.clone());
        let path = local::startup_directory(directory);
        let pid = local::spawn(term.clone(), listener, shell, path.as_deref())?;
        Ok(Self {
            id,
            kind: Kind::Local,
            term,
            shared,
            directory: Mutex::new(DirectoryState {
                pid: (pid != 0).then_some(pid),
                path,
                refreshed: None,
            }),
            grid: INITIAL,
        })
    }

    pub fn ssh(
        ctx: &egui::Context,
        settings: &Settings,
        id: u64,
        config: &SshConfig,
        target: &str,
    ) -> Self {
        let host = config.resolve(target);
        let jumps = host.proxy_jump.iter().map(|j| config.resolve(j)).collect();
        let (shared, term, _) = Self::create(ctx, settings, host.alias.clone());
        let link = ssh::spawn(
            shared.clone(),
            term.clone(),
            host,
            jumps,
            settings.launch_bitwarden,
        );
        let sftp = SftpClient::new(link, ctx.clone());
        Self {
            id,
            kind: Kind::Ssh {
                target: target.to_owned(),
                sftp,
            },
            term,
            shared,
            directory: DirectoryState::idle(),
            grid: INITIAL,
        }
    }

    pub fn serial(
        ctx: &egui::Context,
        settings: &Settings,
        id: u64,
        port: &str,
        baud: u32,
    ) -> std::io::Result<Self> {
        let (shared, term, _) = Self::create(ctx, settings, port.to_owned());
        serial::spawn(shared.clone(), term.clone(), port, baud)?;
        Ok(Self {
            id,
            kind: Kind::Serial {
                port: port.to_owned(),
                baud,
            },
            term,
            shared,
            directory: DirectoryState::idle(),
            grid: INITIAL,
        })
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

    /// Tab text: `folder · shell`. The tooltip is the full directory.
    pub fn tab_title(&self) -> String {
        self.tab_text().0
    }

    pub fn tab_tooltip(&self) -> String {
        self.tab_text().1
    }

    fn tab_text(&self) -> (String, String) {
        tab_text(&self.title(), self.working_directory().as_deref())
    }

    fn working_directory(&self) -> Option<PathBuf> {
        let mut state = self.directory.lock().unwrap();
        let stale = state
            .refreshed
            .is_none_or(|at| at.elapsed() >= DIRECTORY_REFRESH);
        if stale && let Some(pid) = state.pid {
            if let Some(path) = crate::backend::cwd::of_process(pid) {
                state.path = Some(path);
            }
            state.refreshed = Some(Instant::now());
        }
        state.path.clone()
    }

    pub fn status(&self) -> Status {
        self.shared.status()
    }

    pub fn sftp(&self) -> Option<&SftpClient> {
        match &self.kind {
            Kind::Ssh { sftp, .. } => Some(sftp),
            Kind::Local | Kind::Serial { .. } => None,
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

/// `folder · title` when a directory is known. The second value is the full path.
pub(crate) fn tab_text(title: &str, directory: Option<&Path>) -> (String, String) {
    let Some(path) = directory.filter(|path| !path.as_os_str().is_empty()) else {
        return (title.to_owned(), title.to_owned());
    };
    let full = full_path(path);
    let leaf = directory_leaf(path);
    let label = if leaf.is_empty() {
        title.to_owned()
    } else if title.is_empty() || title == leaf || title == full {
        leaf
    } else {
        format!("{leaf} \u{00b7} {title}")
    };
    (label, full)
}

fn directory_leaf(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn full_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() || is_drive_prefix(trimmed) {
        text.into_owned()
    } else {
        trimmed.to_owned()
    }
}

fn is_drive_prefix(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    bytes.len() == 2 && bytes[1] == b':'
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::tab_text;

    #[test]
    fn tab_shows_the_directory_leaf_and_the_full_path() {
        let path = if cfg!(windows) {
            Path::new(r"C:\Users\kikeda\tool\IKTerminal")
        } else {
            Path::new("/home/kikeda/tool/IKTerminal")
        };
        let (label, tooltip) = tab_text("pwsh", Some(path));
        assert_eq!(label, "IKTerminal \u{00b7} pwsh");
        assert!(tooltip.ends_with("IKTerminal"));
        assert!(!tooltip.contains('\u{00b7}'));
    }

    #[test]
    fn tab_without_a_directory_uses_the_title() {
        let (label, tooltip) = tab_text("host", None);
        assert_eq!(label, "host");
        assert_eq!(tooltip, "host");
    }

    #[test]
    fn root_directory_keeps_the_root_text() {
        let path = if cfg!(windows) {
            Path::new(r"C:\")
        } else {
            Path::new("/")
        };
        let leaf = if cfg!(windows) { r"C:\" } else { "/" };
        let (label, tooltip) = tab_text("pwsh", Some(path));
        assert_eq!(label, format!("{leaf} \u{00b7} pwsh"));
        assert_eq!(tooltip, leaf);
    }
}
