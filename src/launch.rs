//! Command line: `ikt [directory]` opens a shell in that directory.
//!
//! Explorer's address bar runs the command with the viewed folder as the working
//! directory, so `ikt .` resolves to that folder. A second process hands the folder
//! to the running instance (`instance`) instead of opening another window.
//!
//! `ikterminal -e <program> [args...]` opens the initial tab with that command
//! (for xdg-terminal-exec and similar).

use std::path::{Path, PathBuf};

use crate::backend::local::ShellSpec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub directory: Option<PathBuf>,
    pub execute: Option<ShellSpec>,
}

/// `Ok` with empty directory starts in the home directory. `Err` is shown and the home directory is used.
pub fn from_args() -> Result<Launch, String> {
    let current = std::env::current_dir().unwrap_or_default();
    parse(std::env::args().skip(1).collect(), &current)
}

fn parse(args: Vec<String>, current: &Path) -> Result<Launch, String> {
    if args.is_empty() {
        return Ok(Launch {
            directory: None,
            execute: None,
        });
    }
    if args[0] == "-e" {
        if args.len() < 2 {
            return Err("-e のあとにコマンドを指定してください".to_owned());
        }
        let program = args[1].clone();
        let name = Path::new(&program)
            .file_stem()
            .map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
        return Ok(Launch {
            directory: None,
            execute: Some(ShellSpec {
                name,
                program,
                args: args[2..].to_vec(),
            }),
        });
    }
    if args[0].starts_with('-') {
        return Err(format!("不明なオプションです: {}", args[0]));
    }
    if args.len() > 1 {
        return Err("引数はフォルダ 1 つだけです".to_owned());
    }
    Ok(Launch {
        directory: working_directory(Some(args[0].as_str()), current)?,
        execute: None,
    })
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

    #[test]
    fn execute_flag_builds_shell_spec() {
        let launch = parse(
            vec!["-e".into(), "bash".into(), "-lc".into(), "true".into()],
            Path::new("/"),
        )
        .unwrap();
        assert!(launch.directory.is_none());
        let shell = launch.execute.unwrap();
        assert_eq!(shell.program, "bash");
        assert_eq!(shell.args, vec!["-lc", "true"]);
    }

    #[test]
    fn execute_flag_requires_program() {
        assert!(parse(vec!["-e".into()], Path::new("/")).is_err());
    }
}
