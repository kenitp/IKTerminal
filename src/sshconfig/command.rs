//! Detect an `ssh` invocation typed at a shell prompt and turn it into a config block.

use super::document::{Document, set_option, write_file};
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
        if is_prompt_token(token) {
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

/// Inserts or updates a host block in `~/.ssh/config` and rewrites the file.
pub fn save_host(draft: &HostDraft) -> Result<(), String> {
    let alias = draft.alias.trim();
    let hostname = draft.hostname.trim();
    if alias.is_empty() || hostname.is_empty() {
        return Err("エイリアスとホスト名は必須です".to_owned());
    }
    if alias.split_whitespace().nth(1).is_some() || alias.contains(['*', '?', '!', ' ']) {
        return Err("エイリアスに空白やワイルドカードは使えません".to_owned());
    }
    let port = draft.port.trim();
    if !port.is_empty() && port.parse::<u16>().ok().filter(|p| *p > 0).is_none() {
        return Err("ポートが不正です".to_owned());
    }

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

/// A prompt fragment such as `C:\dir>` or `user@host:~$`, not a redirection operator.
fn is_prompt_token(token: &str) -> bool {
    if is_redirect(token) {
        return false;
    }
    token.ends_with(['>', '$', '#', '%'])
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
    fn end_of_options() {
        let cmd = find_ssh_command("ssh -p 2200 -- -odd-host").unwrap();
        assert_eq!(cmd.destination, "-odd-host");
        assert_eq!(cmd.port, Some(2200));
    }
}
