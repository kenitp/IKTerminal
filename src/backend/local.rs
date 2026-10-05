//! Local shell over the platform PTY (ConPTY on Windows, POSIX PTY on Linux).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
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
        let name = Path::new(&program)
            .file_stem()
            .map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
        Some(Self {
            name,
            program,
            args: parts.collect(),
        })
    }
}

/// Shells available on this machine, most preferred first.
pub fn detect_shells() -> Vec<ShellSpec> {
    detect_platform_shells()
}

#[cfg(windows)]
fn detect_platform_shells() -> Vec<ShellSpec> {
    let system32 = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32"));
    let mut shells = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |name: &str, program: Option<PathBuf>| {
        if let Some(program) = program.filter(|p| p.is_file()) {
            push_shell(&mut shells, &mut seen, name, program);
        }
    };
    add("PowerShell 7", find_in_path("pwsh.exe"));
    add(
        "Windows PowerShell",
        system32
            .as_ref()
            .map(|s| s.join(r"WindowsPowerShell\v1.0\powershell.exe")),
    );
    add(
        "コマンド プロンプト",
        system32.as_ref().map(|s| s.join("cmd.exe")),
    );
    add("WSL", system32.as_ref().map(|s| s.join("wsl.exe")));
    shells
}

#[cfg(not(windows))]
fn detect_platform_shells() -> Vec<ShellSpec> {
    let mut shells = Vec::new();
    let mut seen = HashSet::new();
    if let Some(program) = std::env::var_os("SHELL").map(PathBuf::from) {
        let name = program
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shell".to_owned());
        push_shell(&mut shells, &mut seen, &name, program);
    }
    for (name, exe) in [("bash", "bash"), ("zsh", "zsh"), ("fish", "fish")] {
        if let Some(program) = find_in_path(exe) {
            push_shell(&mut shells, &mut seen, name, program);
        }
    }
    push_shell(&mut shells, &mut seen, "sh", PathBuf::from("/bin/sh"));
    shells
}

fn push_shell(
    shells: &mut Vec<ShellSpec>,
    seen: &mut HashSet<PathBuf>,
    name: &str,
    program: PathBuf,
) {
    if !program.is_file() {
        return;
    }
    let key = program.canonicalize().unwrap_or_else(|_| program.clone());
    if seen.insert(key) {
        shells.push(ShellSpec {
            name: name.to_owned(),
            program: program.to_string_lossy().into_owned(),
            args: Vec::new(),
        });
    }
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
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

/// Directory a new local shell starts in.
pub fn startup_directory(directory: Option<&Path>) -> Option<PathBuf> {
    directory.map(Path::to_path_buf).or_else(std::env::home_dir)
}

/// Starts `shell` in a PTY and feeds its output into `term`. Returns the shell pid.
pub fn spawn(
    term: TermHandle,
    listener: Listener,
    shell: &ShellSpec,
    directory: Option<&Path>,
) -> std::io::Result<u32> {
    let shared = listener.0.clone();
    let options = tty::Options {
        shell: Some(tty::Shell::new(shell.program.clone(), shell.args.clone())),
        working_directory: startup_directory(directory),
        drain_on_exit: true,
        env: HashMap::from([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ]),
        #[cfg(windows)]
        escape_args: true,
    };
    let pty = tty::new(&options, shared.size(), 0)?;
    let pid = shell_pid(&pty);
    let event_loop = EventLoop::new(term, listener, pty, options.drain_on_exit, false)?;
    shared.set_io(Box::new(LocalIo(event_loop.channel())));
    shared.set_status(Status::Running);
    event_loop.spawn();
    Ok(pid)
}

fn shell_pid(pty: &tty::Pty) -> u32 {
    #[cfg(windows)]
    {
        pty.child_watcher().pid().map(|pid| pid.get()).unwrap_or(0)
    }
    #[cfg(not(windows))]
    {
        pty.child().id()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn detect_shells_points_at_existing_programs() {
        let shells = super::detect_shells();
        assert!(!shells.is_empty());
        for shell in shells {
            assert!(
                std::path::Path::new(&shell.program).is_file(),
                "{}",
                shell.program
            );
        }
    }
}
