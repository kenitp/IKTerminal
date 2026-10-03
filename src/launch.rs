//! Command line: `ikt [directory]` opens a shell in that directory.
//!
//! Explorer's address bar runs the command with the viewed folder as the working
//! directory, so `ikt .` resolves to that folder. A second process hands the folder
//! to the running instance (`instance`) instead of opening another window.

use std::path::{Path, PathBuf};

/// `Ok(None)` starts in the home directory. `Err` is shown and the home directory is used.
pub fn from_args() -> Result<Option<PathBuf>, String> {
    let current = std::env::current_dir().unwrap_or_default();
    working_directory(std::env::args().nth(1).as_deref(), &current)
}

pub fn working_directory(arg: Option<&str>, current: &Path) -> Result<Option<PathBuf>, String> {
    let Some(arg) = arg.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if arg.starts_with('-') {
        return Err(format!("不明なオプションです: {arg}"));
    }
    let path = Path::new(arg);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current.join(path)
    };
    if !path.is_dir() {
        return Err(format!("{} はフォルダではありません", path.display()));
    }
    Ok(Some(strip_verbatim(path.canonicalize().unwrap_or(path))))
}

/// `canonicalize` on Windows prefixes `\\?\`, which some shells handle poorly.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_argument_uses_current_directory() {
        let dir = std::env::temp_dir().join(format!("ikterm-launch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let found = working_directory(Some("sub"), &dir).unwrap().unwrap();
        assert_eq!(
            found,
            strip_verbatim(dir.join("sub").canonicalize().unwrap())
        );
        assert!(working_directory(Some("missing"), &dir).is_err());
        assert!(working_directory(None, &dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
