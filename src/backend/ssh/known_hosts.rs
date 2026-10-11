//! Replace a changed host key in `~/.ssh/known_hosts`.

use std::collections::HashSet;
use std::path::PathBuf;

use russh::keys::PublicKey;

/// Removes this host's old key of the same algorithm and records `key`.
pub fn replace(host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
    let path = known_hosts_path()?;
    let drop_lines = conflicting_lines(host, port, key)?;
    if drop_lines.is_empty() {
        return Err("該当する登録が見つかりません".to_owned());
    }
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let kept = retain(&text, &drop_lines);
    if path.exists() {
        std::fs::copy(&path, path.with_extension("bak")).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, kept).map_err(|error| error.to_string())?;
    russh::keys::known_hosts::learn_known_hosts(host, port, key).map_err(|error| error.to_string())
}

fn known_hosts_path() -> Result<PathBuf, String> {
    std::env::home_dir()
        .map(|home| home.join(".ssh").join("known_hosts"))
        .ok_or_else(|| "ホームディレクトリがありません".to_owned())
}

fn conflicting_lines(host: &str, port: u16, key: &PublicKey) -> Result<HashSet<usize>, String> {
    let recorded =
        russh::keys::known_hosts::known_host_keys(host, port).map_err(|error| error.to_string())?;
    Ok(recorded
        .into_iter()
        .filter(|(_, current)| current.algorithm() == key.algorithm() && *current != *key)
        .map(|(line, _)| line)
        .collect())
}

/// Drops 1-based line numbers. Comment lines are not counted, matching russh.
fn retain(text: &str, drop_lines: &HashSet<usize>) -> String {
    let mut line = 1usize;
    let mut out = String::new();
    for raw in split_lines(text) {
        if raw.as_bytes().first() == Some(&b'#') {
            out.push_str(raw);
            continue;
        }
        if !drop_lines.contains(&line) {
            out.push_str(raw);
        }
        line += 1;
    }
    out
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_lines_are_not_numbered() {
        let text = "# note\nhost ssh-ed25519 AAAA\nother ssh-ed25519 BBBB\n";
        let drop_lines = HashSet::from([1]);
        assert_eq!(
            retain(text, &drop_lines),
            "# note\nother ssh-ed25519 BBBB\n"
        );
    }

    #[test]
    fn other_hosts_stay() {
        let text = "host ssh-ed25519 AAAA\nkeep ssh-ed25519 BBBB";
        let drop_lines = HashSet::from([1]);
        assert_eq!(retain(text, &drop_lines), "keep ssh-ed25519 BBBB");
    }
}
