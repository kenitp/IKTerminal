//! User preferences persisted as a small `key = value` file.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub font_size: f32,
    /// Terminal font file. Empty selects a system monospace font.
    pub font_path: String,
    /// Default shell command line. Empty selects the first detected shell.
    pub shell: String,
    pub scrollback: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { font_size: 14.0, font_path: String::new(), shell: String::new(), scrollback: 5000 }
    }
}

impl Settings {
    pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 8.0..=32.0;

    fn path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::home_dir().map(|h| h.join(".config")))
        };
        base.map(|b| b.join("IkTerminal").join("settings.conf"))
    }

    pub fn load() -> Self {
        let mut s = Settings::default();
        let Some(text) = Self::path().and_then(|p| std::fs::read_to_string(p).ok()) else { return s };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "font_size" => s.font_size = value.parse().unwrap_or(s.font_size),
                "font_path" => s.font_path = value.to_owned(),
                "shell" => s.shell = value.to_owned(),
                "scrollback" => s.scrollback = value.parse().unwrap_or(s.scrollback),
                _ => {}
            }
        }
        s.font_size = s.font_size.clamp(*Self::FONT_SIZE_RANGE.start(), *Self::FONT_SIZE_RANGE.end());
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("config directory not found"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!(
            "font_size = {}\nfont_path = {}\nshell = {}\nscrollback = {}\n",
            self.font_size, self.font_path, self.shell, self.scrollback
        );
        std::fs::write(path, text)
    }
}
