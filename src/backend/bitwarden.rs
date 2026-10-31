//! Starts the Bitwarden desktop app so its SSH agent can answer.
//!
//! On Windows the agent listens on `\\.\pipe\openssh-ssh-agent` while the app is running.
//! The OpenSSH Authentication Agent service must be disabled, or it owns that pipe instead.
//! On Linux the agent listens on a Unix socket, `~/.bitwarden-ssh-agent.sock` for the
//! native package. Snap, Flatpak, and `BITWARDEN_SSH_AUTH_SOCK` use their own paths.
//! The desktop launcher execs `bitwarden-app`.

/// If Bitwarden is installed and not running, start it and wait briefly for the agent.
pub async fn ensure_running() {
    #[cfg(windows)]
    windows::ensure().await;
    #[cfg(target_os = "linux")]
    linux::ensure().await;
}

/// Bitwarden agent socket that is accepting connections.
#[cfg(target_os = "linux")]
pub fn agent_socket() -> Option<std::path::PathBuf> {
    linux::live_socket()
}

#[cfg(windows)]
mod windows {
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Official Microsoft Store package. The family name does not change across versions.
    const STORE_FAMILY: &str = "8bitSolutionsLLC.bitwardendesktop_h4e712dmw3xyy";
    const STORE_AUMID: &str = "8bitSolutionsLLC.bitwardendesktop_h4e712dmw3xyy!bitwardendesktop";

    pub async fn ensure() {
        if process_running() {
            return;
        }
        if !start() {
            return;
        }
        for _ in 0..30 {
            if agent_pipe_present() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn start() -> bool {
        if let Some(exe) = find_exe() {
            return Command::new(exe).spawn().is_ok();
        }
        if !store_registered() {
            return false;
        }
        // Packaged apps do not start from Bitwarden.exe. Activate the registered app id.
        Command::new("explorer.exe")
            .arg(format!(r"shell:AppsFolder\{STORE_AUMID}"))
            .spawn()
            .is_ok()
    }

    fn store_registered() -> bool {
        let key = format!(
            r"HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData\{STORE_FAMILY}"
        );
        Command::new("reg")
            .args(["query", &key])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok_and(|output| output.status.success())
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

#[cfg(target_os = "linux")]
mod linux {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    const SOCKET_NAME: &str = ".bitwarden-ssh-agent.sock";

    pub async fn ensure() {
        if process_running() || live_socket().is_some() {
            return;
        }
        if !start() {
            return;
        }
        for _ in 0..30 {
            if live_socket().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub fn live_socket() -> Option<PathBuf> {
        socket_candidates().into_iter().find(|path| serves(path))
    }

    fn start() -> bool {
        if let Some(exe) = find_exe() {
            return spawn(Command::new(exe));
        }
        start_flatpak()
    }

    fn find_exe() -> Option<PathBuf> {
        let mut candidates = vec![
            PathBuf::from("/opt/Bitwarden/bitwarden"),
            PathBuf::from("/usr/bin/bitwarden"),
            PathBuf::from("/usr/local/bin/bitwarden"),
            PathBuf::from("/snap/bin/bitwarden"),
        ];
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                candidates.push(dir.join("bitwarden"));
            }
        }
        candidates.into_iter().find(|path| path.is_file())
    }

    fn start_flatpak() -> bool {
        let flatpak = ["/usr/bin/flatpak", "/usr/local/bin/flatpak"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file());
        let Some(flatpak) = flatpak else {
            return false;
        };
        let home = home_dir();
        let installed = Path::new("/var/lib/flatpak/app/com.bitwarden.desktop").is_dir()
            || home.is_some_and(|dir| {
                dir.join(".local/share/flatpak/app/com.bitwarden.desktop")
                    .is_dir()
            });
        if !installed {
            return false;
        }
        let mut command = Command::new(flatpak);
        command.arg("run").arg("com.bitwarden.desktop");
        spawn(command)
    }

    fn spawn(mut command: Command) -> bool {
        // A new process group so closing a terminal that started IkTerminal
        // does not deliver SIGHUP to Bitwarden.
        let child = command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match child {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                true
            }
            Err(_) => false,
        }
    }

    fn process_running() -> bool {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            if !name.bytes().all(|byte| byte.is_ascii_digit()) {
                return false;
            }
            std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| is_app(&comm))
        })
    }

    /// The launcher script execs `bitwarden-app`. `comm` is at most 15 bytes.
    fn is_app(comm: &str) -> bool {
        let comm = comm.trim();
        comm == "bitwarden" || comm == "bitwarden-app"
    }

    fn socket_candidates() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(custom) = std::env::var_os("BITWARDEN_SSH_AUTH_SOCK") {
            paths.push(PathBuf::from(custom));
        }
        if let Some(home) = home_dir() {
            paths.push(home.join(SOCKET_NAME));
            paths.push(home.join("snap/bitwarden/current").join(SOCKET_NAME));
            paths.push(
                home.join(".var/app/com.bitwarden.desktop/data")
                    .join(SOCKET_NAME),
            );
            paths.push(
                home.join(".var/app/com.bitwarden.desktop")
                    .join(SOCKET_NAME),
            );
        }
        paths
    }

    fn serves(path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|meta| meta.file_type().is_socket())
            && std::os::unix::net::UnixStream::connect(path).is_ok()
    }

    fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn native_socket_is_in_the_home_directory() {
            let Some(home) = home_dir() else {
                return;
            };
            assert!(
                socket_candidates()
                    .iter()
                    .any(|path| path == &home.join(SOCKET_NAME))
            );
        }
    }
}
