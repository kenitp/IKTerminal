//! Local shell over the platform PTY (ConPTY on Windows).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::tty;

use crate::terminal::{Listener, PtyIo, Status, TermHandle};

#[derive(Clone, Debug, PartialEq)]
pub struct ShellSpec {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

impl ShellSpec {
    /// Builds a spec from a user supplied command line (`program arg1 arg2`).
    pub fn from_command_line(cmd: &str) -> Option<Self> {
        let mut parts = cmd.split_whitespace().map(str::to_owned);
        let program = parts.next()?;
        let name = Path::new(&program).file_stem().map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
        Some(Self { name, program, args: parts.collect() })
    }
}

/// Shells available on this machine, most preferred first.
pub fn detect_shells() -> Vec<ShellSpec> {
    let mut shells = Vec::new();
    let mut add = |name: &str, program: Option<PathBuf>| {
        if let Some(p) = program {
            shells.push(ShellSpec { name: name.to_owned(), program: p.to_string_lossy().into_owned(), args: Vec::new() });
        }
    };
    if cfg!(windows) {
        let system32 = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32"));
        add("PowerShell 7", find_in_path("pwsh.exe"));
        add(
            "Windows PowerShell",
            system32.as_ref().map(|s| s.join("WindowsPowerShell\\v1.0\\powershell.exe")).filter(|p| p.exists()),
        );
        add("コマンド プロンプト", system32.as_ref().map(|s| s.join("cmd.exe")).filter(|p| p.exists()));
        add("WSL", system32.as_ref().map(|s| s.join("wsl.exe")).filter(|p| p.exists()));
    } else {
        add("Shell", std::env::var_os("SHELL").map(PathBuf::from).or_else(|| Some(PathBuf::from("/bin/sh"))));
    }
    shells
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(exe)).find(|p| p.is_file())
}

struct LocalIo(EventLoopSender);

impl PtyIo for LocalIo {
    fn write(&self, data: Cow<'static, [u8]>) {
        if !data.is_empty() {
            let _ = self.0.send(Msg::Input(data));
        }
    }

    fn resize(&self, size: WindowSize) {
        let _ = self.0.send(Msg::Resize(size));
    }

    fn shutdown(&self) {
        let _ = self.0.send(Msg::Shutdown);
    }
}

/// Starts `shell` in a PTY and feeds its output into `term`.
pub fn spawn(term: TermHandle, listener: Listener, shell: &ShellSpec) -> std::io::Result<()> {
    let shared = listener.0.clone();
    let options = tty::Options {
        shell: Some(tty::Shell::new(shell.program.clone(), shell.args.clone())),
        working_directory: std::env::home_dir(),
        drain_on_exit: true,
        env: HashMap::from([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ]),
        #[cfg(windows)]
        escape_args: true,
    };
    let pty = tty::new(&options, shared.size(), 0)?;
    let event_loop = EventLoop::new(term, listener, pty, options.drain_on_exit, false)?;
    shared.set_io(Box::new(LocalIo(event_loop.channel())));
    shared.set_status(Status::Running);
    event_loop.spawn();
    Ok(())
}
