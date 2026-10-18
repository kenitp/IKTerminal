//! Detect an `ssh` invocation typed at a shell prompt and turn it into a config block.

use super::document::{Document, find_option, push_option, set_option, write_file};
use super::resolve::SshConfig;

/// Connection fields taken from one `ssh` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshCommand {
    /// Token after the options: a host alias or a hostname.
    pub destination: String,
    /// `HostName` when `-o HostName=` was given, otherwise the same as `destination`.
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

impl SshCommand {
    /// Short form shown in the confirmation dialog.
    pub fn summary(&self) -> String {
        let mut parts = vec!["ssh".to_owned()];
        if let Some(port) = self.port {
            parts.push(format!("-p {port}"));
        }
        if let Some(identity) = &self.identity_file {
            parts.push(format!("-i {identity}"));
        }
        if let Some(jump) = &self.proxy_jump {
            parts.push(format!("-J {jump}"));
        }
        let dest = match &self.user {
            Some(user) => format!("{user}@{}", self.destination),
            None => self.destination.clone(),
        };
        parts.push(dest);
        parts.join(" ")
    }
}

/// Values edited in the "add to SSH config" dialog.
#[derive(Clone, Debug)]
pub struct HostDraft {
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
    pub identity_file: String,
    pub proxy_jump: String,
}

impl HostDraft {
    pub fn from_command(cmd: &SshCommand) -> Self {
        Self {
            alias: cmd.destination.clone(),
            hostname: cmd.hostname.clone(),
            user: cmd.user.clone().unwrap_or_default(),
            port: cmd.port.map(|p| p.to_string()).unwrap_or_default(),
            identity_file: cmd.identity_file.clone().unwrap_or_default(),
            proxy_jump: cmd.proxy_jump.clone().unwrap_or_default(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.alias.trim().is_empty() || self.hostname.trim().is_empty() {
            return Err("エイリアスとホスト名は必須です".to_owned());
        }
        validate_alias(&self.alias)?;
        let port = self.port.trim();
        if !port.is_empty() && port.parse::<u16>().ok().filter(|p| *p > 0).is_none() {
            return Err("ポートが不正です".to_owned());
        }
        Ok(())
    }
}

fn validate_alias(alias: &str) -> Result<(), String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("エイリアスを入力してください".to_owned());
    }
    if alias.contains(|c: char| c.is_whitespace() || matches!(c, '*' | '?' | '!')) {
        return Err("エイリアスに空白やワイルドカードは使えません".to_owned());
    }
    Ok(())
}

/// The last `ssh` command on a shell line (prompt included), if it has a destination.
pub fn find_ssh_command(line: &str) -> Option<SshCommand> {
    let tokens = tokenize(line);
    let mut found = None;
    let mut at_command = true;
    for (i, token) in tokens.iter().enumerate() {
        if token.starts_with('#') {
            break;
        }
        if is_separator(token) {
            at_command = true;
            continue;
        }
        if is_redirect(token) {
            at_command = false;
            continue;
        }
        if is_prompt_token(token) || (at_command && is_cwd_token(token)) {
            at_command = true;
            continue;
        }
        if at_command
            && is_ssh_program(token)
            && let Some(cmd) = parse_ssh_args(&tokens[i + 1..])
        {
            found = Some(cmd);
        }
        at_command = false;
    }
    found
}

/// What the terminal showed after an `ssh` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshOutcome {
    /// Still connecting, or waiting on a prompt.
    Pending,
    /// The command returned without a remote shell.
    Failed,
    /// Remote prompt. `host` is the name in that prompt.
    Connected { host: String },
}

/// Reads the screen after `command_line` and decides whether `ssh` logged in.
pub fn ssh_outcome(command_line: &str, screen: &str) -> SshOutcome {
    let Some(after) = output_after(command_line, screen) else {
        return SshOutcome::Pending;
    };
    if after.trim().is_empty() {
        return SshOutcome::Pending;
    }
    let local = local_prompt(command_line).trim();
    let lines: Vec<&str> = after
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let Some(last) = lines.last().copied() else {
        return SshOutcome::Pending;
    };
    if !local.is_empty() && last == local {
        if is_bare_prompt_line(last)
            && let Some(host) = lines.iter().rev().nth(1).and_then(|line| status_host(line))
        {
            return SshOutcome::Connected {
                host: host.to_owned(),
            };
        }
        return SshOutcome::Failed;
    }
    if let Some(host) = shell_prompt_host(last) {
        return SshOutcome::Connected { host };
    }
    if is_bare_prompt_line(last) {
        let host = lines
            .iter()
            .rev()
            .nth(1)
            .and_then(|line| status_host(line))
            .unwrap_or("")
            .to_owned();
        return SshOutcome::Connected { host };
    }
    let lower_last = last.to_ascii_lowercase();
    if SSH_FAILURES.iter().any(|mark| lower_last.contains(mark)) {
        return SshOutcome::Failed;
    }
    SshOutcome::Pending
}

/// Alias becomes `hostname.local`. `HostName` stays the IP used to connect.
pub fn apply_connected_host(cmd: &mut SshCommand, host: &str) {
    let host = host.trim();
    if host.is_empty() {
        return;
    }
    if is_ip_host(host) {
        if !is_ip_host(&cmd.hostname) {
            cmd.hostname = bare_ip(host);
        }
        return;
    }
    let ip = if is_ip_host(&cmd.hostname) {
        Some(bare_ip(&cmd.hostname))
    } else if is_ip_host(&cmd.destination) {
        Some(bare_ip(&cmd.destination))
    } else {
        None
    };
    cmd.destination = format!("{}.local", short_host(host));
    if let Some(ip) = ip {
        cmd.hostname = ip;
    }
}

/// `hostname.local` for a host name. `None` for an IP address or empty text.
pub fn local_alias(host: &str) -> Option<String> {
    let host = host.trim();
    (!host.is_empty() && !is_ip_host(host)).then(|| format!("{}.local", short_host(host)))
}

fn bare_ip(host: &str) -> String {
    host.trim_matches(['[', ']']).to_owned()
}

fn short_host(host: &str) -> String {
    host.split('.')
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or(host)
        .to_owned()
}

pub fn is_ip_host(host: &str) -> bool {
    host.trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok()
}

const SSH_FAILURES: &[&str] = &[
    "connection refused",
    "connection timed out",
    "connection timeout",
    "operation timed out",
    "could not resolve",
    "name or service not known",
    "no route to host",
    "network is unreachable",
    "host key verification failed",
    "remote host identification has changed",
    "connection closed",
    "connection reset",
    "kex_exchange_identification",
    "too many authentication failures",
    "permission denied (",
    "no matching host key",
];

fn local_prompt(line: &str) -> &str {
    let tokens = tokenize(line);
    let mut from = 0;
    for token in &tokens {
        let Some(rel) = line[from..].find(token.as_str()) else {
            break;
        };
        let at = from + rel;
        if is_ssh_program(token) {
            return line[..at].trim();
        }
        from = at + token.len();
    }
    ""
}

/// Hostname in a shell prompt. Empty when the prompt has no hostname.
fn shell_prompt_host(line: &str) -> Option<String> {
    if is_bare_prompt_line(line.trim()) {
        return None;
    }
    let head = prompt_body(line)?;
    if head.is_empty() || head.eq_ignore_ascii_case("ps") {
        return Some(String::new());
    }
    if let Some(host) = extract_host(head) {
        return Some(host.to_owned());
    }
    if head.contains('\\') || head.contains('~') || head.contains('@') || head.contains(':') {
        return Some(String::new());
    }
    None
}

fn prompt_body(line: &str) -> Option<&str> {
    let line = line.trim();
    let line = line.strip_prefix('[').unwrap_or(line);
    if let Some(head) = rprompt_body(line) {
        return Some(head);
    }
    let end = line.chars().next_back()?;
    if !matches!(end, '$' | '#' | '>') && end != '%' {
        return None;
    }
    if end == '%'
        && line
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    let head = &line[..line.len() - end.len_utf8()];
    Some(head.trim().trim_end_matches(']'))
}

/// Prompt text before a right-side prompt (`user@host:~$          15:04`).
fn rprompt_body(line: &str) -> Option<&str> {
    for (index, ch) in line.char_indices() {
        if !matches!(ch, '$' | '#' | '>') && ch != '%' {
            continue;
        }
        if ch == '%'
            && line[..index]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit())
        {
            continue;
        }
        let rest = &line[index + ch.len_utf8()..];
        if rest.starts_with("  ") && !rest.trim().is_empty() {
            let head = line[..index].trim().trim_end_matches(']');
            if head.is_empty()
                || head.contains('@')
                || head.contains('~')
                || head.contains(':')
                || head.contains('\\')
            {
                return Some(head);
            }
        }
    }
    None
}

fn is_bare_prompt_line(line: &str) -> bool {
    let line = line.trim();
    if matches!(line, "$" | "#" | "%" | ">") {
        return true;
    }
    let mut chars = line.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_prompt_glyph(first) && chars.all(|c| c.is_whitespace() || c == '~' || c == '/')
}

fn is_prompt_glyph(ch: char) -> bool {
    matches!(ch, '❯' | '➜' | 'λ' | '›' | '»' | '≻')
}

/// Host on the status line above a bare prompt (`user@host in ~` then `❯`).
fn status_host(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.to_ascii_lowercase().contains("password") {
        return None;
    }
    if !(line.contains(" in ") || line.contains('~') || line.contains(':')) {
        return None;
    }
    let (user, rest) = line.split_once('@')?;
    if user.is_empty() || user.chars().any(|c| c.is_whitespace() || c == '/') {
        return None;
    }
    let host = host_token(rest);
    valid_host_token(host).then_some(host)
}

fn extract_host(head: &str) -> Option<&str> {
    let head = head.trim().trim_end_matches(']');
    if head.is_empty() || head.eq_ignore_ascii_case("ps") || head.contains('\\') {
        return None;
    }
    if let Some((user, rest)) = head.split_once('@') {
        if user.is_empty() || user.chars().any(|c| c.is_whitespace() || c == '/') {
            return None;
        }
        let host = host_token(rest);
        return valid_host_token(host).then_some(host);
    }
    let host = host_token(head);
    valid_host_token(host).then_some(host)
}

fn host_token(rest: &str) -> &str {
    let rest = rest.trim_start_matches('[');
    if let Some((host, _)) = rest.split_once(']') {
        return host;
    }
    let token = rest.split([' ', ']']).next().unwrap_or(rest);
    if let Some((host, path)) = token.rsplit_once(':')
        && (path.is_empty() || path.starts_with('~') || path.starts_with('/'))
    {
        return host;
    }
    token
}

fn valid_host_token(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if is_ip_host(host) {
        return true;
    }
    let Some(first) = host.chars().next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && host.chars().any(|c| c.is_ascii_alphabetic())
}

/// Text that follows the command, including when the grid wrapped it.
fn output_after<'a>(command_line: &str, screen: &'a str) -> Option<&'a str> {
    let needle = command_line.trim();
    if needle.is_empty() {
        return None;
    }
    if let Some(index) = screen.rfind(needle) {
        return Some(&screen[index + needle.len()..]);
    }
    let mut stripped = String::new();
    let mut origins = Vec::new();
    for (index, ch) in screen.char_indices() {
        if ch == '\n' || ch == '\r' {
            continue;
        }
        stripped.push(ch);
        while origins.len() < stripped.len() {
            origins.push(index);
        }
    }
    let at = stripped.rfind(needle)?;
    let next = at + needle.len();
    let start = origins.get(next).copied().unwrap_or(screen.len());
    Some(&screen[start..])
}

/// Inserts or updates a host block in `~/.ssh/config` and rewrites the file.
pub fn save_host(draft: &HostDraft) -> Result<(), String> {
    draft.validate()?;
    let alias = draft.alias.trim();
    let hostname = draft.hostname.trim();
    let port = draft.port.trim();
    let path = SshConfig::default_path().ok_or("ホームディレクトリが見つかりません")?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc = Document::parse(&existing);
    let index = doc
        .blocks
        .iter()
        .position(|b| {
            b.is_host()
                && b.patterns
                    .split_whitespace()
                    .any(|p| p.eq_ignore_ascii_case(alias))
        })
        .unwrap_or_else(|| doc.add_host(alias));
    let lines = &mut doc.blocks[index].lines;
    set_option(lines, "HostName", hostname);
    set_option(lines, "User", draft.user.trim());
    if !port.is_empty() && port != "22" {
        set_option(lines, "Port", port);
    }
    set_option(lines, "IdentityFile", draft.identity_file.trim());
    set_option(lines, "ProxyJump", draft.proxy_jump.trim());
    write_file(&path, &doc.to_text()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Renames the `Host` pattern `old` to `new` in `~/.ssh/config`.
pub fn rename_host(old: &str, new: &str) -> Result<(), String> {
    let path = SshConfig::default_path().ok_or("ホームディレクトリが見つかりません")?;
    let existing =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc = Document::parse(&existing);
    rename_in(&mut doc, old, new)?;
    write_file(&path, &doc.to_text()).map_err(|e| format!("{}: {e}", path.display()))
}

/// A block without `HostName` gets `HostName old`, so it still reaches the same address.
fn rename_in(doc: &mut Document, old: &str, new: &str) -> Result<(), String> {
    let (old, new) = (old.trim(), new.trim());
    validate_alias(new)?;
    let has = |patterns: &str, name: &str| {
        patterns
            .split_whitespace()
            .any(|p| p.eq_ignore_ascii_case(name))
    };
    if doc
        .blocks
        .iter()
        .any(|b| b.is_host() && has(&b.patterns, new))
    {
        return Err(format!("{new} は既に config にあります"));
    }
    let block = doc
        .blocks
        .iter_mut()
        .find(|b| b.is_host() && has(&b.patterns, old))
        .ok_or_else(|| format!("{old} が config にありません"))?;
    block.patterns = block
        .patterns
        .split_whitespace()
        .map(|p| if p.eq_ignore_ascii_case(old) { new } else { p })
        .collect::<Vec<_>>()
        .join(" ");
    if find_option(&block.lines, "HostName").is_none() {
        push_option(&mut block.lines, "HostName", old);
    }
    Ok(())
}

fn parse_ssh_args(tokens: &[String]) -> Option<SshCommand> {
    let mut index = 0;
    let mut user = None;
    let mut port = None;
    let mut identity = None;
    let mut jump = None;
    let mut hostname = None;
    while index < tokens.len() {
        let token = &tokens[index];
        if is_separator(token) || token.starts_with('#') {
            break;
        }
        if token == "--" {
            index += 1;
            let token = tokens.get(index)?;
            if is_separator(token) || token.starts_with('#') {
                return None;
            }
            return destination(token, user, port, identity, jump, hostname);
        }
        if let Some(rest) = token.strip_prefix('-')
            && !rest.is_empty()
            && !rest.starts_with('-')
        {
            let chars: Vec<char> = rest.chars().collect();
            let mut cursor = 0;
            while cursor < chars.len() {
                let flag = chars[cursor];
                if takes_argument(flag) {
                    let inline: String = chars[cursor + 1..].iter().collect();
                    let value = if inline.is_empty() {
                        index += 1;
                        tokens.get(index)?.clone()
                    } else {
                        inline
                    };
                    apply_flag(
                        flag,
                        &value,
                        &mut user,
                        &mut port,
                        &mut identity,
                        &mut jump,
                        &mut hostname,
                    );
                    break;
                }
                cursor += 1;
            }
            index += 1;
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        return destination(token, user, port, identity, jump, hostname);
    }
    None
}

fn destination(
    token: &str,
    user: Option<String>,
    port: Option<u16>,
    identity: Option<String>,
    jump: Option<String>,
    hostname: Option<String>,
) -> Option<SshCommand> {
    if token.contains('/') {
        return None;
    }
    let (user_at, host) = match token.rsplit_once('@') {
        Some((name, host)) if !name.is_empty() && !host.is_empty() => (Some(name.to_owned()), host),
        _ => (None, token),
    };
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    Some(SshCommand {
        destination: host.to_owned(),
        hostname: hostname.unwrap_or_else(|| host.to_owned()),
        user: user.or(user_at),
        port,
        identity_file: identity,
        proxy_jump: jump,
    })
}

fn apply_flag(
    flag: char,
    value: &str,
    user: &mut Option<String>,
    port: &mut Option<u16>,
    identity: &mut Option<String>,
    jump: &mut Option<String>,
    hostname: &mut Option<String>,
) {
    match flag {
        'p' => *port = value.parse().ok().filter(|p| *p > 0),
        'l' => *user = Some(value.to_owned()),
        'i' => *identity = Some(value.to_owned()),
        'J' => *jump = Some(value.to_owned()),
        'o' => apply_option(value, user, port, identity, jump, hostname),
        _ => {}
    }
}

fn apply_option(
    value: &str,
    user: &mut Option<String>,
    port: &mut Option<u16>,
    identity: &mut Option<String>,
    jump: &mut Option<String>,
    hostname: &mut Option<String>,
) {
    let (key, raw) = value.split_once('=').unwrap_or((value, ""));
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    match key.trim().to_ascii_lowercase().as_str() {
        "hostname" => *hostname = Some(raw.to_owned()),
        "user" => *user = Some(raw.to_owned()),
        "port" => *port = raw.parse().ok().filter(|p| *p > 0),
        "identityfile" => *identity = Some(raw.to_owned()),
        "proxyjump" => *jump = Some(raw.to_owned()),
        _ => {}
    }
}

fn takes_argument(flag: char) -> bool {
    matches!(
        flag,
        'B' | 'b'
            | 'c'
            | 'D'
            | 'E'
            | 'e'
            | 'F'
            | 'I'
            | 'i'
            | 'J'
            | 'L'
            | 'l'
            | 'm'
            | 'O'
            | 'o'
            | 'p'
            | 'Q'
            | 'R'
            | 'S'
            | 'W'
            | 'w'
    )
}

fn is_ssh_program(token: &str) -> bool {
    let name = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let stem = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name);
    stem.eq_ignore_ascii_case("ssh")
}

fn is_separator(token: &str) -> bool {
    matches!(token, ";" | "&&" | "||" | "|" | "&" | "(" | ")")
}

fn is_redirect(token: &str) -> bool {
    let rest = token.trim_start_matches(|c: char| c.is_ascii_digit());
    matches!(rest, ">" | ">>" | "<" | "<<" | ">&" | "<&")
}

/// A prompt fragment such as `C:\dir>`, `user@host:~$`, or `❯`, not a redirection operator.
fn is_prompt_token(token: &str) -> bool {
    if is_redirect(token) {
        return false;
    }
    if token.ends_with(['>', '$', '#', '%']) {
        return true;
    }
    let mut chars = token.chars();
    let Some(ch) = chars.next() else {
        return false;
    };
    chars.next().is_none() && is_prompt_glyph(ch)
}

/// `~` or `~/dir` in a prompt (`➜  ~`), while a command is expected.
fn is_cwd_token(token: &str) -> bool {
    token == "~" || token.starts_with("~/")
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

    #[test]
    fn prompt_and_options() {
        let cmd = find_ssh_command(
            r"PS C:\Users\a> ssh -p 2222 -i ~/.ssh/id_ed25519 -J bastion kenichi@lab.example",
        )
        .unwrap();
        assert_eq!(cmd.destination, "lab.example");
        assert_eq!(cmd.user.as_deref(), Some("kenichi"));
        assert_eq!(cmd.port, Some(2222));
        assert_eq!(cmd.identity_file.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(cmd.proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn ignores_ssh_that_is_not_the_command() {
        assert!(find_ssh_command("echo ssh host").is_none());
        assert!(find_ssh_command("PS C:\\> ssh-keygen -t ed25519").is_none());
        assert!(find_ssh_command(r"C:\Users\a> ssh").is_none());
        assert!(find_ssh_command("echo hi # ssh host").is_none());
    }

    #[test]
    fn chained_command_and_option_equals() {
        let cmd =
            find_ssh_command("cd /tmp && ssh.exe -o HostName=10.0.0.8 -o Port=2200 web").unwrap();
        assert_eq!(cmd.destination, "web");
        assert_eq!(cmd.hostname, "10.0.0.8");
        assert_eq!(cmd.port, Some(2200));
    }

    #[test]
    fn login_flag_wins_over_user_at() {
        let cmd = find_ssh_command("user@host:~$ ssh -l root admin@db").unwrap();
        assert_eq!(cmd.destination, "db");
        assert_eq!(cmd.user.as_deref(), Some("root"));
    }

    #[test]
    fn draft_validation() {
        let draft = HostDraft {
            alias: "pi".to_owned(),
            hostname: "192.168.1.5".to_owned(),
            user: "pi".to_owned(),
            port: "22".to_owned(),
            identity_file: String::new(),
            proxy_jump: String::new(),
        };
        assert!(draft.validate().is_ok());
        let spaced = HostDraft {
            alias: "my pi".to_owned(),
            ..draft.clone()
        };
        assert!(spaced.validate().is_err());
        let bad_port = HostDraft {
            port: "0".to_owned(),
            ..draft
        };
        assert!(bad_port.validate().is_err());
    }

    #[test]
    fn end_of_options() {
        let cmd = find_ssh_command("ssh -p 2200 -- -odd-host").unwrap();
        assert_eq!(cmd.destination, "-odd-host");
        assert_eq!(cmd.port, Some(2200));
    }

    #[test]
    fn config_waits_until_the_remote_prompt() {
        let line = "kenichi@desktop:~$ ssh pi@192.168.1.20";
        let password = format!("{line}\npi@192.168.1.20's password:");
        assert_eq!(ssh_outcome(line, &password), SshOutcome::Pending);
        let hostkey =
            format!("{line}\nAre you sure you want to continue connecting (yes/no/[fingerprint])?");
        assert_eq!(ssh_outcome(line, &hostkey), SshOutcome::Pending);
        let ok = format!("{line}\nLast login: Mon Oct 12\npi@raspberrypi:~$ ");
        assert_eq!(
            ssh_outcome(line, &ok),
            SshOutcome::Connected {
                host: "raspberrypi".to_owned()
            }
        );
        let wrapped = "kenichi@desktop:~$ ssh pi@\nlab\npi@raspberrypi:~$";
        assert_eq!(
            ssh_outcome("kenichi@desktop:~$ ssh pi@lab", wrapped),
            SshOutcome::Connected {
                host: "raspberrypi".to_owned()
            }
        );
        let refused = format!(
            "{line}\nssh: connect to host 192.168.1.20 port 22: Connection refused\nkenichi@desktop:~$"
        );
        assert_eq!(ssh_outcome(line, &refused), SshOutcome::Failed);
        let back = format!("{line}\nkenichi@desktop:~$");
        assert_eq!(ssh_outcome(line, &back), SshOutcome::Failed);
    }

    #[test]
    fn rename_keeps_the_address_and_rejects_duplicates() {
        let mut doc = Document::parse(
            "Host 192.168.1.20\n    User pi\nHost nas\n    HostName 10.0.0.9\nHost *\n",
        );
        rename_in(&mut doc, "192.168.1.20", "raspberrypi.local").unwrap();
        assert_eq!(
            doc.to_text(),
            "Host raspberrypi.local\n    User pi\n    HostName 192.168.1.20\nHost nas\n    HostName 10.0.0.9\nHost *\n"
        );

        let mut doc = Document::parse("Host 10.0.0.9 box\n    HostName 10.0.0.9\nHost nas\n");
        rename_in(&mut doc, "10.0.0.9", "box2.local").unwrap();
        assert_eq!(doc.blocks[0].patterns, "box2.local box");
        assert_eq!(
            doc.to_text().matches("HostName").count(),
            1,
            "an existing HostName is kept"
        );

        assert!(rename_in(&mut doc, "box", "nas").is_err());
        assert!(rename_in(&mut doc, "missing", "x.local").is_err());
        assert!(rename_in(&mut doc, "box", "a b").is_err());
    }

    #[test]
    fn local_alias_skips_addresses() {
        assert_eq!(
            local_alias("raspberrypi.lan").as_deref(),
            Some("raspberrypi.local")
        );
        assert_eq!(local_alias("nas").as_deref(), Some("nas.local"));
        assert_eq!(local_alias("192.168.1.20"), None);
        assert_eq!(local_alias(" "), None);
    }

    #[test]
    fn connected_alias_is_hostname_local_and_hostname_stays_ip() {
        let mut cmd = find_ssh_command("ssh pi@192.168.1.20").unwrap();
        apply_connected_host(&mut cmd, "raspberrypi");
        assert_eq!(cmd.destination, "raspberrypi.local");
        assert_eq!(cmd.hostname, "192.168.1.20");
        assert_eq!(cmd.user.as_deref(), Some("pi"));

        let mut named = find_ssh_command("ssh pi").unwrap();
        apply_connected_host(&mut named, "raspberrypi.local");
        assert_eq!(named.destination, "raspberrypi.local");
        assert_eq!(named.hostname, "pi");

        let mut only_ip = find_ssh_command("ssh 10.0.0.8").unwrap();
        apply_connected_host(&mut only_ip, "10.0.0.8");
        assert_eq!(only_ip.destination, "10.0.0.8");
        assert_eq!(only_ip.hostname, "10.0.0.8");

        let mut short = find_ssh_command("ssh nas").unwrap();
        apply_connected_host(&mut short, "10.0.0.9");
        assert_eq!(short.destination, "nas");
        assert_eq!(short.hostname, "10.0.0.9");

        let mut unnamed = find_ssh_command("ssh 10.0.0.8").unwrap();
        apply_connected_host(&mut unnamed, "");
        assert_eq!(unnamed.destination, "10.0.0.8");
        assert_eq!(unnamed.hostname, "10.0.0.8");
    }

    fn judge(line: &str, after: &str) -> SshOutcome {
        ssh_outcome(line, &format!("{line}\n{after}"))
    }

    #[test]
    fn ssh_after_common_local_prompts() {
        let lines = [
            r"PS C:\Users\a> ssh 10.0.0.8",
            r"C:\Users\a> ssh 10.0.0.8",
            "kenichi@desktop:~$ ssh 10.0.0.8",
            "$ ssh 10.0.0.8",
            "❯ ssh 10.0.0.8",
            "➜  ~ ssh 10.0.0.8",
            "user@host ~> ssh 10.0.0.8",
            "pi@raspberrypi:~# ssh 10.0.0.8",
            "λ ssh 10.0.0.8",
        ];
        for line in lines {
            let cmd = find_ssh_command(line).unwrap_or_else(|| panic!("missed ssh in {line}"));
            assert_eq!(cmd.destination, "10.0.0.8", "{line}");
        }
        assert!(find_ssh_command("❯ echo ssh 10.0.0.8").is_none());
        assert!(find_ssh_command("➜ ssh-keygen -t ed25519").is_none());
        assert!(find_ssh_command("echo ~ ssh 10.0.0.8").is_none());
    }

    #[test]
    fn remote_shell_prompts_count_as_connected() {
        let line = r"PS C:\Users\a> ssh pi@192.168.1.20";
        let cases = [
            ("pi@raspberrypi:~$ ", "raspberrypi"),
            ("[pi@raspberrypi ~]$", "raspberrypi"),
            ("pi@raspberrypi ~ %", "raspberrypi"),
            ("pi@raspberrypi ~>", "raspberrypi"),
            ("pi@raspberrypi:~$          15:04", "raspberrypi"),
            ("raspberrypi:~$", "raspberrypi"),
            ("raspberrypi:~#", "raspberrypi"),
            ("raspberrypi:~ pi$", "raspberrypi"),
            ("pi@192.168.1.20:~$", "192.168.1.20"),
            ("pi@fe80::1:~$", "fe80::1"),
            ("router#", "router"),
            (r"C:\Users\pi>", ""),
            (r"PS C:\Users\admin>", ""),
            ("#", ""),
            ("$", ""),
        ];
        for (prompt, host) in cases {
            assert_eq!(
                judge(line, &format!("Last login: Mon Oct 12\n{prompt}")),
                SshOutcome::Connected {
                    host: host.to_owned()
                },
                "{prompt}"
            );
        }
    }

    #[test]
    fn starship_and_oh_my_zsh_prompts() {
        let bash = "kenichi@desktop:~$ ssh pi@192.168.1.20";
        assert_eq!(
            judge(bash, "pi@raspberrypi in ~\n❯"),
            SshOutcome::Connected {
                host: "raspberrypi".to_owned()
            }
        );
        assert_eq!(
            judge(bash, "➜  ~"),
            SshOutcome::Connected {
                host: String::new()
            }
        );

        let starship = "❯ ssh pi@192.168.1.20";
        assert_eq!(
            judge(starship, "pi@raspberrypi in ~\n❯"),
            SshOutcome::Connected {
                host: "raspberrypi".to_owned()
            }
        );
        assert_eq!(
            judge(
                starship,
                "ssh: connect to host 192.168.1.20 port 22: Connection refused\n❯"
            ),
            SshOutcome::Failed
        );
    }

    #[test]
    fn password_retry_stays_pending_until_the_shell() {
        let line = "kenichi@desktop:~$ ssh pi@192.168.1.20";
        let retry = "\
pi@192.168.1.20's password:
Permission denied, please try again.
pi@192.168.1.20's password:";
        assert_eq!(judge(line, retry), SshOutcome::Pending);
        let ok = format!("{retry}\nLast login: Mon Oct 12\npi@raspberrypi:~$");
        assert_eq!(
            judge(line, &ok),
            SshOutcome::Connected {
                host: "raspberrypi".to_owned()
            }
        );
        assert_eq!(
            judge(
                line,
                "Are you sure you want to continue connecting (yes/no/[fingerprint])?"
            ),
            SshOutcome::Pending
        );
    }

    #[test]
    fn banner_text_does_not_hide_or_fake_a_prompt() {
        let line = "kenichi@desktop:~$ ssh pi@192.168.1.20";
        assert_eq!(
            judge(
                line,
                "Notice: idle sessions get a connection closed.\npi@raspberrypi:~$"
            ),
            SshOutcome::Connected {
                host: "raspberrypi".to_owned()
            }
        );
        assert_eq!(
            judge(line, "Mail the admin@example.com team.\nkenichi@desktop:~$"),
            SshOutcome::Failed
        );
        assert_eq!(judge(line, "Downloading 100%"), SshOutcome::Pending);
        assert_eq!(
            judge("ssh pi@192.168.1.20", "Downloading 100%"),
            SshOutcome::Pending
        );
        assert_eq!(
            judge(
                line,
                "Notice: idle sessions get a connection closed.\nLast login: Mon Oct 12"
            ),
            SshOutcome::Pending
        );
    }

    #[test]
    fn connection_failure_and_local_prompt_stay_failed() {
        let line = r"PS C:\Users\a> ssh pi@192.168.1.20";
        assert_eq!(judge(line, r"PS C:\Users\a>"), SshOutcome::Failed);
        assert_eq!(
            judge(
                line,
                "ssh: connect to host 192.168.1.20 port 22: Connection refused"
            ),
            SshOutcome::Failed
        );
        assert_eq!(
            judge(
                line,
                "ssh: connect to host 192.168.1.20 port 22: Connection timed out"
            ),
            SshOutcome::Failed
        );
        assert_eq!(
            judge(line, "Permission denied (publickey).\nPS C:\\Users\\a>"),
            SshOutcome::Failed
        );
        assert_eq!(
            judge(
                line,
                "pi@192.168.1.20: Permission denied (publickey,password)."
            ),
            SshOutcome::Failed
        );
    }
}
