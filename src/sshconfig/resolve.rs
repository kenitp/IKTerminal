//! Read-only evaluation of OpenSSH config files (including `Include`),
//! following the "first obtained value wins" rule.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::document::split_option;

const MAX_INCLUDE_DEPTH: usize = 8;

/// Connection parameters for one host after applying the config.
#[derive(Clone, Debug)]
pub struct HostConfig {
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub user: String,
    pub identity_files: Vec<PathBuf>,
    pub identities_only: bool,
    pub proxy_jump: Vec<String>,
    pub server_alive_interval: Option<u64>,
    pub connect_timeout: Option<u64>,
}

/// A concrete host alias listed in the config (for the host list).
#[derive(Clone, Debug)]
pub struct HostEntry {
    pub alias: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
enum Condition {
    Always,
    Never,
    Hosts(Vec<String>),
}

impl Condition {
    fn matches(&self, host: &str) -> bool {
        match self {
            Condition::Always => true,
            Condition::Never => false,
            Condition::Hosts(patterns) => {
                let mut matched = false;
                for p in patterns {
                    if let Some(neg) = p.strip_prefix('!') {
                        if wildcard_match(neg, host) {
                            return false;
                        }
                    } else if wildcard_match(p, host) {
                        matched = true;
                    }
                }
                matched
            }
        }
    }
}

#[derive(Debug)]
struct Section {
    condition: Condition,
    options: Vec<(String, String)>,
}

#[derive(Debug, Default)]
pub struct SshConfig {
    sections: Vec<Section>,
}

impl SshConfig {
    /// `~/.ssh/config`
    pub fn default_path() -> Option<PathBuf> {
        ssh_dir().map(|d| d.join("config"))
    }

    pub fn load_default() -> Self {
        let mut cfg = SshConfig::default();
        if let Some(path) = Self::default_path() {
            cfg.load_file(&path, Condition::Always, 0);
        }
        cfg
    }

    fn load_file(&mut self, path: &Path, condition: Condition, depth: usize) {
        if depth > MAX_INCLUDE_DEPTH {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        self.sections.push(Section {
            condition,
            options: Vec::new(),
        });
        for line in text.lines() {
            let Some((key, value)) = split_option(line) else {
                continue;
            };
            let key = key.to_ascii_lowercase();
            match key.as_str() {
                "host" => {
                    let patterns = split_args(value)
                        .into_iter()
                        .map(|p| p.to_ascii_lowercase())
                        .collect();
                    self.sections.push(Section {
                        condition: Condition::Hosts(patterns),
                        options: Vec::new(),
                    });
                }
                "match" => {
                    let cond = if value.trim().eq_ignore_ascii_case("all") {
                        Condition::Always
                    } else {
                        Condition::Never
                    };
                    self.sections.push(Section {
                        condition: cond,
                        options: Vec::new(),
                    });
                }
                "include" => {
                    let inherited = self.sections.last().expect("section").condition.clone();
                    for file in split_args(value).iter().flat_map(|p| expand_include(p)) {
                        self.load_file(&file, inherited.clone(), depth + 1);
                    }
                    // Lines after an Include continue in the enclosing section.
                    self.sections.push(Section {
                        condition: inherited,
                        options: Vec::new(),
                    });
                }
                _ => self
                    .sections
                    .last_mut()
                    .expect("section")
                    .options
                    .push((key, unquote(value).to_owned())),
            }
        }
    }

    /// Concrete host aliases (patterns without wildcards or negation).
    pub fn hosts(&self) -> Vec<HostEntry> {
        let mut seen = Vec::<String>::new();
        let mut out = Vec::new();
        for section in &self.sections {
            let Condition::Hosts(patterns) = &section.condition else {
                continue;
            };
            for p in patterns {
                if p.contains(['*', '?', '!']) || seen.contains(p) {
                    continue;
                }
                seen.push(p.clone());
                let cfg = self.resolve(p);
                let detail = if cfg.port == 22 {
                    format!("{}@{}", cfg.user, cfg.hostname)
                } else {
                    format!("{}@{}:{}", cfg.user, cfg.hostname, cfg.port)
                };
                out.push(HostEntry {
                    alias: p.clone(),
                    detail,
                });
            }
        }
        out
    }

    /// True when `host` is already an alias, or a configured hostname with the same port and user.
    pub fn contains_target(&self, host: &str, user: Option<&str>, port: u16) -> bool {
        self.hosts().iter().any(|entry| {
            if entry.alias.eq_ignore_ascii_case(host) {
                return true;
            }
            let resolved = self.resolve(&entry.alias);
            let user_ok = user.is_none_or(|name| name.eq_ignore_ascii_case(&resolved.user));
            resolved.hostname.eq_ignore_ascii_case(host) && resolved.port == port && user_ok
        })
    }

    /// Resolves `target`, which is a host alias or `[user@]host[:port]`.
    pub fn resolve(&self, target: &str) -> HostConfig {
        let (user_override, rest) = match target.rsplit_once('@') {
            Some((u, r)) => (Some(u.to_owned()), r),
            None => (None, target),
        };
        let (alias, port_override) = match rest.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().ok()),
            _ => (rest, None),
        };
        let alias_lc = alias.to_ascii_lowercase();

        let mut opts: HashMap<&str, &str> = HashMap::new();
        let mut identity_files: Vec<&str> = Vec::new();
        for section in self
            .sections
            .iter()
            .filter(|s| s.condition.matches(&alias_lc))
        {
            for (k, v) in &section.options {
                if k == "identityfile" {
                    identity_files.push(v);
                } else {
                    opts.entry(k.as_str()).or_insert(v.as_str());
                }
            }
        }

        let hostname = opts
            .get("hostname")
            .map_or_else(|| alias.to_owned(), |h| h.replace("%h", alias));
        let port = port_override
            .or_else(|| opts.get("port").and_then(|p| p.parse().ok()))
            .unwrap_or(22);
        let user = user_override
            .or_else(|| opts.get("user").map(|u| u.to_string()))
            .unwrap_or_else(local_user);
        let identity_files = identity_files
            .into_iter()
            .filter(|f| !f.eq_ignore_ascii_case("none"))
            .map(|f| expand_tokens(f, &hostname, &user))
            .collect();
        let proxy_jump = opts
            .get("proxyjump")
            .filter(|v| !v.eq_ignore_ascii_case("none"))
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        HostConfig {
            alias: alias.to_owned(),
            hostname,
            port,
            user,
            identity_files,
            identities_only: opts
                .get("identitiesonly")
                .is_some_and(|v| v.eq_ignore_ascii_case("yes")),
            proxy_jump,
            server_alive_interval: opts
                .get("serveraliveinterval")
                .and_then(|v| v.parse().ok())
                .filter(|v| *v > 0),
            connect_timeout: opts
                .get("connecttimeout")
                .and_then(|v| v.parse().ok())
                .filter(|v| *v > 0),
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

pub fn ssh_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ssh"))
}

fn local_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

/// Splits whitespace-separated arguments, honoring double quotes.
fn split_args(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in value.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

fn expand_home(path: &str) -> PathBuf {
    match (
        path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")),
        home_dir(),
    ) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

fn expand_tokens(path: &str, hostname: &str, user: &str) -> PathBuf {
    let home = home_dir()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    let expanded = path
        .replace("%d", &home)
        .replace("%u", &local_user())
        .replace("%h", hostname)
        .replace("%r", user)
        .replace("%%", "%");
    expand_home(&expanded)
}

/// Resolves an `Include` argument (relative to `~/.ssh`, `*`/`?` allowed in the file name).
fn expand_include(arg: &str) -> Vec<PathBuf> {
    let mut path = expand_home(arg);
    if path.is_relative()
        && let Some(dir) = ssh_dir()
    {
        path = dir.join(path);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            wildcard_match(
                &name.to_ascii_lowercase(),
                &e.file_name().to_string_lossy().to_ascii_lowercase(),
            )
        })
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// `*` and `?` glob matching.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> SshConfig {
        let dir = std::env::temp_dir().join(format!("ikterm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("cfg-{}", text.len()));
        std::fs::write(&path, text).unwrap();
        let mut cfg = SshConfig::default();
        cfg.load_file(&path, Condition::Always, 0);
        cfg
    }

    #[test]
    fn first_value_wins_and_wildcards() {
        let cfg = config(
            "Host web\n  HostName 10.0.0.1\n  User alice\nHost *\n  User bob\n  Port 2200\n",
        );
        let h = cfg.resolve("web");
        assert_eq!(
            (h.hostname.as_str(), h.user.as_str(), h.port),
            ("10.0.0.1", "alice", 2200)
        );
        let o = cfg.resolve("carol@other:22");
        assert_eq!(
            (o.hostname.as_str(), o.user.as_str(), o.port),
            ("other", "carol", 22)
        );
        assert_eq!(cfg.hosts().len(), 1);
        assert!(cfg.contains_target("web", None, 2200));
        assert!(cfg.contains_target("10.0.0.1", Some("alice"), 2200));
        assert!(!cfg.contains_target("10.0.0.1", Some("carol"), 2200));
        assert!(!cfg.contains_target("other", None, 22));
    }

    #[test]
    fn negation_and_proxyjump() {
        let cfg = config("Host *.lan !gw.lan\n  ProxyJump gw.lan,hop\n");
        assert_eq!(cfg.resolve("db.lan").proxy_jump, vec!["gw.lan", "hop"]);
        assert!(cfg.resolve("gw.lan").proxy_jump.is_empty());
    }

    #[test]
    fn glob() {
        assert!(wildcard_match("*.example.com", "a.example.com"));
        assert!(wildcard_match("h?st", "host"));
        assert!(!wildcard_match("*.example.com", "example.org"));
    }
}
