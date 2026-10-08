//! Open the current SSH directory in local Cursor over Remote-SSH.
//!
//! `cursor .` typed in an SSH session is not sent to the remote shell. Cursor
//! is started locally with a `vscode-remote://ssh-remote+...` folder URI.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::sshconfig::SshConfig;
use crate::terminal::RemoteDir;

/// Bytes that ask the remote shell to report `$PWD` and `$HOME` as OSC 1337.
pub const REPORT_DIRS: &[u8] =
    b"printf '\\033]1337;CurrentDir=%s\\007\\033]1337;IkHome=%s\\007' \"$PWD\" \"$HOME\"\r";

const FLAGS: &[&str] = &["-n", "--new-window", "-r", "--reuse-window", "-w", "--wait"];

/// Path operand of a lone `cursor` command. `.` when the command has no path.
pub fn command(line: &str) -> Option<String> {
    let tokens = after_prompt(line);
    if tokens.first().is_none_or(|token| !is_cursor(token)) {
        return None;
    }
    if tokens
        .iter()
        .any(|token| is_separator(token) || is_redirect(token))
    {
        return None;
    }
    path_operand(&tokens[1..])
}

/// Directory shown by the prompt, the line above it, or the window title.
pub fn directory_hint(line: &str, above: Option<&str>, title: &str) -> Option<String> {
    path_before_cursor(line)
        .or_else(|| above.and_then(lone_path))
        .or_else(|| path_in_text(title))
}

/// Applies a lone `cd` to the tracked remote directory. Returns whether it changed.
pub fn note_cd(line: &str, dir: &mut RemoteDir) -> bool {
    let Some(arg) = cd_argument(line) else {
        return false;
    };
    let next = match arg.as_deref() {
        None => dir.home.clone(),
        Some("-") => dir.previous.clone(),
        Some(path) => Some(join_cd(path, dir)),
    };
    let Some(next) = next.filter(|path| is_usable(path)) else {
        return false;
    };
    let next = normalize(&next);
    if dir.cwd.as_deref() == Some(next.as_str()) {
        return false;
    }
    dir.previous = dir.cwd.clone();
    dir.cwd = Some(next);
    dir.generation = dir.generation.wrapping_add(1);
    true
}

/// Absolute remote path for `spec` (`.` is the current directory).
pub fn resolve(spec: &str, hint: Option<&str>, dir: &RemoteDir) -> Result<String, ()> {
    let spec = if spec.is_empty() { "." } else { spec };
    let spec = expand_tilde(spec, dir.home.as_deref());
    if is_absolute(&spec) {
        return Ok(normalize(&spec));
    }
    if spec.starts_with('~') {
        return Err(());
    }
    let base = hint
        .map(|path| expand_tilde(path, dir.home.as_deref()))
        .filter(|path| is_absolute(path))
        .or_else(|| {
            dir.cwd
                .as_deref()
                .map(|path| expand_tilde(path, dir.home.as_deref()))
                .filter(|path| is_absolute(path))
        })
        .ok_or(())?;
    if spec == "." {
        Ok(normalize(&base))
    } else {
        Ok(normalize(&format!(
            "{}/{}",
            base.trim_end_matches('/'),
            spec
        )))
    }
}

/// Host string Cursor Remote-SSH passes to OpenSSH.
///
/// A config alias wins, so port, user, key and ProxyJump come from `~/.ssh/config`.
pub fn remote_authority(config: &SshConfig, target: &str) -> String {
    let hosts = config.hosts();
    if let Some(entry) = hosts
        .iter()
        .find(|entry| entry.alias.eq_ignore_ascii_case(target))
    {
        return entry.alias.clone();
    }
    let resolved = config.resolve(target);
    if let Some(entry) = hosts.iter().find(|entry| {
        let host = config.resolve(&entry.alias);
        host.hostname.eq_ignore_ascii_case(&resolved.hostname)
            && host.port == resolved.port
            && host.user.eq_ignore_ascii_case(&resolved.user)
    }) {
        return entry.alias.clone();
    }
    if !target.contains('@') && !target.contains(':') {
        return target.to_owned();
    }
    if resolved.port == 22 {
        format!("{}@{}", resolved.user, resolved.hostname)
    } else {
        format!("{}@{}:{}", resolved.user, resolved.hostname, resolved.port)
    }
}

pub fn folder_uri(host: &str, path: &str) -> String {
    format!(
        "vscode-remote://ssh-remote+{}{}",
        encode_host(host),
        encode_path(path)
    )
}

pub fn launch(host: &str, path: &str) -> Result<(), String> {
    let uri = folder_uri(host, path);
    let mut command = cursor_command(&uri)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Cursor を起動できません: {error}"))
}

fn cursor_command(uri: &str) -> Result<Command, String> {
    match find_cursor().ok_or("Cursor が見つかりません")? {
        CursorBin::Exe(path) => {
            let mut command = Command::new(path);
            command.arg("--folder-uri").arg(uri);
            Ok(command)
        }
        CursorBin::Cmd(path) => {
            let mut command = Command::new(if cfg!(windows) { "cmd.exe" } else { "sh" });
            if cfg!(windows) {
                command.arg("/C").arg(path);
            } else {
                command.arg(path);
            }
            command.arg("--folder-uri").arg(uri);
            Ok(command)
        }
    }
}

enum CursorBin {
    Exe(PathBuf),
    Cmd(PathBuf),
}

fn find_cursor() -> Option<CursorBin> {
    for path in install_candidates() {
        if path.is_file() {
            return Some(classify(path));
        }
    }
    for name in ["Cursor.exe", "cursor.exe", "cursor", "cursor.cmd"] {
        if let Some(path) = find_in_path(name) {
            return Some(classify(path));
        }
    }
    None
}

fn classify(path: PathBuf) -> CursorBin {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") => {
            CursorBin::Cmd(path)
        }
        _ => CursorBin::Exe(path),
    }
}

fn install_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let root = PathBuf::from(local).join("Programs");
        paths.push(root.join("cursor").join("Cursor.exe"));
        paths.push(root.join("Cursor").join("Cursor.exe"));
    }
    if let Some(program) = std::env::var_os("ProgramFiles") {
        paths.push(PathBuf::from(program).join("Cursor").join("Cursor.exe"));
    }
    if let Some(home) = std::env::home_dir() {
        paths.push(home.join(".local").join("bin").join("cursor"));
    }
    paths.push(PathBuf::from("/usr/bin/cursor"));
    paths.push(PathBuf::from("/opt/cursor/cursor"));
    paths
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn path_operand(tokens: &[String]) -> Option<String> {
    let mut index = 0;
    let mut path = None;
    let mut options_ended = false;
    while index < tokens.len() {
        let token = &tokens[index];
        if !options_ended && token == "--" {
            options_ended = true;
            index += 1;
            continue;
        }
        if !options_ended && token.starts_with('-') {
            if !FLAGS.contains(&token.as_str()) {
                return None;
            }
            index += 1;
            continue;
        }
        if path.is_some() {
            return None;
        }
        path = Some(token.clone());
        index += 1;
    }
    Some(path.unwrap_or_else(|| ".".to_owned()))
}

fn path_before_cursor(line: &str) -> Option<String> {
    let tokens = tokenize(line);
    let cursor_at = tokens.iter().position(|token| is_cursor(token))?;
    tokens[..cursor_at]
        .iter()
        .rev()
        .find_map(|token| path_in_token(token))
}

fn path_in_text(text: &str) -> Option<String> {
    tokenize(text)
        .iter()
        .rev()
        .find_map(|token| path_in_token(token))
}

fn lone_path(line: &str) -> Option<String> {
    let token = line.split_whitespace().next()?;
    path_in_token(token)
}

fn path_in_token(token: &str) -> Option<String> {
    let token = token.trim_end_matches(['$', '#', '%', '>', ']', ')', ':']);
    if let Some((_, rest)) = token.rsplit_once(':')
        && looks_like_path(rest)
    {
        return Some(rest.to_owned());
    }
    looks_like_path(token).then(|| token.to_owned())
}

fn looks_like_path(token: &str) -> bool {
    !token.is_empty()
        && (token.starts_with('/')
            || token.starts_with('~')
            || token.starts_with("./")
            || token.starts_with("../")
            || token == "."
            || token == ".."
            || windows_prefix(token))
}

fn after_prompt(line: &str) -> Vec<String> {
    let tokens = tokenize(line);
    let mut start = 0;
    for (i, token) in tokens.iter().enumerate() {
        if token.starts_with('#') {
            return tokens[start..i].to_vec();
        }
        if is_prompt(token) {
            start = i + 1;
        }
    }
    tokens[start..].to_vec()
}

fn cd_argument(line: &str) -> Option<Option<String>> {
    let tokens = after_prompt(line);
    if tokens.first().map(String::as_str) != Some("cd") {
        return None;
    }
    if tokens
        .iter()
        .any(|token| is_separator(token) || is_redirect(token))
    {
        return None;
    }
    let rest = &tokens[1..];
    if rest.is_empty() {
        return Some(None);
    }
    if rest.len() == 1 && rest[0] != "--" {
        return Some(Some(rest[0].clone()));
    }
    if rest.len() == 2 && rest[0] == "--" {
        return Some(Some(rest[1].clone()));
    }
    None
}

fn join_cd(path: &str, dir: &RemoteDir) -> String {
    let path = expand_tilde(path, dir.home.as_deref());
    if is_absolute(&path) {
        return path;
    }
    match dir.cwd.as_deref() {
        Some(cwd) => format!("{}/{}", cwd.trim_end_matches('/'), path),
        None => path,
    }
}

fn expand_tilde(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|home| is_absolute(home)) else {
        return path.to_owned();
    };
    if path == "~" {
        home.trim_end_matches('/').to_owned()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        path.to_owned()
    }
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || windows_prefix(path)
}

fn windows_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    let offset = usize::from(bytes.first() == Some(&b'/'));
    bytes.len() >= offset + 3
        && bytes[offset].is_ascii_alphabetic()
        && bytes[offset + 1] == b':'
        && (bytes[offset + 2] == b'\\' || bytes[offset + 2] == b'/')
}

fn is_usable(path: &str) -> bool {
    !path.is_empty() && !path.contains(['\n', '\r'])
}

fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let (drive, rest) = split_drive(&path);
    let mut parts = Vec::new();
    for part in rest.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop();
            continue;
        }
        parts.push(part);
    }
    match drive {
        Some(drive) => format!("/{drive}/{}", parts.join("/")),
        None => format!("/{}", parts.join("/")),
    }
}

fn split_drive(path: &str) -> (Option<String>, &str) {
    let bytes = path.as_bytes();
    let offset = usize::from(bytes.first() == Some(&b'/'));
    if bytes.len() >= offset + 2 && bytes[offset].is_ascii_alphabetic() && bytes[offset + 1] == b':'
    {
        let drive = path[offset..offset + 2].to_ascii_lowercase();
        return (Some(drive), &path[offset + 2..]);
    }
    (None, path)
}

fn encode_host(host: &str) -> String {
    encode(host, false)
}

fn encode_path(path: &str) -> String {
    encode(path, true)
}

fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        let plain = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && byte == b'/');
        if plain {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn is_cursor(token: &str) -> bool {
    let name = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let stem = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .or_else(|| name.strip_suffix(".cmd"))
        .or_else(|| name.strip_suffix(".CMD"))
        .unwrap_or(name);
    stem.eq_ignore_ascii_case("cursor")
}

fn is_separator(token: &str) -> bool {
    matches!(token, ";" | "&&" | "||" | "|" | "&" | "(" | ")")
}

fn is_redirect(token: &str) -> bool {
    let rest = token.trim_start_matches(|c: char| c.is_ascii_digit());
    matches!(rest, ">" | ">>" | "<" | "<<" | ">&" | "<&")
}

fn is_prompt(token: &str) -> bool {
    !is_redirect(token) && token.ends_with(['>', '$', '#', '%'])
}

fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut quote = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            '&' | '|' => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                let mut op = String::from(c);
                if chars.peek() == Some(&c) {
                    op.push(chars.next().unwrap());
                }
                out.push(op);
            }
            ';' | '(' | ')' => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                out.push(c.to_string());
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(cwd: Option<&str>, home: Option<&str>) -> RemoteDir {
        RemoteDir {
            cwd: cwd.map(str::to_owned),
            home: home.map(str::to_owned),
            previous: None,
            generation: 0,
        }
    }

    #[test]
    fn cursor_command_from_prompt() {
        assert_eq!(command("user@host:~/app$ cursor .").as_deref(), Some("."));
        assert_eq!(command(r"PS C:\Users\a> cursor .").as_deref(), Some("."));
        assert_eq!(command("user@host:~$ cursor").as_deref(), Some("."));
        assert_eq!(command("user@host:~$ cursor src").as_deref(), Some("src"));
        assert_eq!(
            command(r#"user@host:~$ cursor -n "My App""#).as_deref(),
            Some("My App")
        );
        assert!(command("echo cursor .").is_none());
        assert!(command("user@host:~$ cd /tmp && cursor .").is_none());
        assert!(command("user@host:~$ cursor . && ls").is_none());
        assert!(command("user@host:~$ cursor --install-extension rust").is_none());
    }

    #[test]
    fn hint_from_prompt_title_and_line_above() {
        assert_eq!(
            directory_hint("user@host:~/projects/app$ cursor .", None, "").as_deref(),
            Some("~/projects/app")
        );
        assert_eq!(
            directory_hint("user@host:/var/log# cursor .", None, "").as_deref(),
            Some("/var/log")
        );
        assert_eq!(
            directory_hint("❯ cursor .", Some("~/projects/app"), "").as_deref(),
            Some("~/projects/app")
        );
        assert_eq!(
            directory_hint("❯ cursor .", None, "user@host: ~/work").as_deref(),
            Some("~/work")
        );
        assert!(directory_hint("[user@host app]$ cursor .", None, "app").is_none());
    }

    #[test]
    fn resolve_relative_absolute_and_tilde() {
        let dir = remote(Some("/home/user/app"), Some("/home/user"));
        assert_eq!(resolve(".", None, &dir).unwrap(), "/home/user/app");
        assert_eq!(resolve("src", None, &dir).unwrap(), "/home/user/app/src");
        assert_eq!(resolve("..", None, &dir).unwrap(), "/home/user");
        assert_eq!(
            resolve(".", Some("~/projects/app"), &dir).unwrap(),
            "/home/user/projects/app"
        );
        assert_eq!(resolve("/var/log", None, &dir).unwrap(), "/var/log");
        assert_eq!(resolve(r"C:\Users\a", None, &dir).unwrap(), "/c:/Users/a");
        assert!(resolve(".", None, &remote(None, None)).is_err());
    }

    #[test]
    fn cd_updates_tracked_directory() {
        let mut dir = remote(Some("/home/user"), Some("/home/user"));
        assert!(note_cd(r"PS C:\Users\a> cd projects/app", &mut dir));
        assert_eq!(dir.cwd.as_deref(), Some("/home/user/projects/app"));
        assert!(note_cd("user@host:~$ cd -", &mut dir));
        assert_eq!(dir.cwd.as_deref(), Some("/home/user"));
        assert!(!note_cd("user@host:~$ ls", &mut dir));
        assert!(!note_cd("user@host:~$ cd /tmp && ls", &mut dir));
    }

    #[test]
    fn folder_uri_encodes_host_and_path() {
        assert_eq!(
            folder_uri("lab", "/home/user/app"),
            "vscode-remote://ssh-remote+lab/home/user/app"
        );
        assert_eq!(
            folder_uri("alice@lab:2200", "/home/my app"),
            "vscode-remote://ssh-remote+alice%40lab%3A2200/home/my%20app"
        );
    }

    #[test]
    fn authority_keeps_quick_connect() {
        let config = SshConfig::default();
        assert_eq!(remote_authority(&config, "web"), "web");
        assert_eq!(remote_authority(&config, "alice@web"), "alice@web");
        assert_eq!(
            remote_authority(&config, "alice@web:2200"),
            "alice@web:2200"
        );
    }
}
