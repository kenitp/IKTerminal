//! Starts the Bitwarden desktop app so its SSH agent can answer.
//!
//! On Windows the agent listens on `\\.\pipe\openssh-ssh-agent` while the app is running.
//! The OpenSSH Authentication Agent service must be disabled, or it owns that pipe instead.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// If Bitwarden is installed and not running, start it and wait briefly for the agent pipe.
pub async fn ensure_running() {
    #[cfg(windows)]
    windows::ensure().await;
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub async fn ensure() {
        if process_running() {
            return;
        }
        let Some(exe) = find_exe() else { return };
        if Command::new(&exe).spawn().is_err() {
            return;
        }
        for _ in 0..30 {
            if agent_pipe_present() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn process_running() -> bool {
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq Bitwarden.exe", "/NH", "/FO", "CSV"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("bitwarden.exe")
    }

    fn find_exe() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            candidates.push(
                local
                    .join("Programs")
                    .join("Bitwarden")
                    .join("Bitwarden.exe"),
            );
            candidates.push(local.join("Bitwarden").join("Bitwarden.exe"));
        }
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            candidates.push(
                PathBuf::from(programs)
                    .join("Bitwarden")
                    .join("Bitwarden.exe"),
            );
        }
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .or_else(app_path)
    }

    fn app_path() -> Option<PathBuf> {
        for key in [
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\Bitwarden.exe",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths\Bitwarden.exe",
        ] {
            let Ok(output) = Command::new("reg")
                .args(["query", key, "/ve"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            else {
                continue;
            };
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let Some(index) = line.find(":\\") else {
                    continue;
                };
                if index == 0 {
                    continue;
                }
                let path = PathBuf::from(line[index - 1..].trim());
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn agent_pipe_present() -> bool {
        std::fs::read_dir(Path::new(r"\\.\pipe\"))
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.file_name() == "openssh-ssh-agent")
    }
}
