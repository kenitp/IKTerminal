//! Round-trip editable model of a single OpenSSH config file.
//!
//! Comments, blank lines and unknown options are preserved as-is so that
//! saving an edited file only changes what the user touched.

const INDENT: &str = "    ";

#[derive(Clone, Debug)]
pub enum Line {
    Option {
        indent: String,
        key: String,
        value: String,
    },
    Raw(String),
}

impl Line {
    pub fn option(key: &str, value: &str) -> Self {
        Line::Option {
            indent: INDENT.to_owned(),
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }
}

/// A `Host` or `Match` section and the lines that follow it.
#[derive(Clone, Debug)]
pub struct Block {
    pub keyword: String,
    pub patterns: String,
    pub lines: Vec<Line>,
}

impl Block {
    pub fn new_host(patterns: &str) -> Self {
        Self {
            keyword: "Host".to_owned(),
            patterns: patterns.to_owned(),
            lines: vec![Line::Raw(String::new())],
        }
    }

    pub fn is_host(&self) -> bool {
        self.keyword.eq_ignore_ascii_case("host")
    }
}

/// Index of the first option line with `key` (case-insensitive).
pub fn find_option(lines: &[Line], key: &str) -> Option<usize> {
    lines
        .iter()
        .position(|l| matches!(l, Line::Option { key: k, .. } if k.eq_ignore_ascii_case(key)))
}

/// Inserts an option after the last option line.
pub fn push_option(lines: &mut Vec<Line>, key: &str, value: &str) {
    let pos = lines
        .iter()
        .rposition(|l| matches!(l, Line::Option { .. }))
        .map_or(0, |i| i + 1);
    lines.insert(pos, Line::option(key, value));
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    /// Lines before the first `Host`/`Match` (apply to every host).
    pub global: Vec<Line>,
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn parse(text: &str) -> Self {
        let mut doc = Document::default();
        for raw in text.lines() {
            let raw = raw.trim_end_matches('\r');
            let line = parse_line(raw);
            if let Line::Option { key, value, .. } = &line
                && (key.eq_ignore_ascii_case("host") || key.eq_ignore_ascii_case("match"))
            {
                doc.blocks.push(Block {
                    keyword: key.clone(),
                    patterns: value.clone(),
                    lines: Vec::new(),
                });
                continue;
            }
            match doc.blocks.last_mut() {
                Some(block) => block.lines.push(line),
                None => doc.global.push(line),
            }
        }
        doc
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        write_lines(&mut out, &self.global);
        for block in &self.blocks {
            out.push_str(&block.keyword);
            out.push(' ');
            out.push_str(block.patterns.trim());
            out.push('\n');
            write_lines(&mut out, &block.lines);
        }
        out
    }

    /// Inserts a host block before any catch-all `Host *` block so that it takes effect.
    pub fn add_host(&mut self, patterns: &str) -> usize {
        let pos = self
            .blocks
            .iter()
            .position(|b| b.is_host() && b.patterns.trim() == "*")
            .unwrap_or(self.blocks.len());
        self.blocks.insert(pos, Block::new_host(patterns));
        pos
    }
}

/// Sets an option, or appends it when missing. An empty value removes nothing and writes nothing.
pub fn set_option(lines: &mut Vec<Line>, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if let Some(index) = find_option(lines, key) {
        if let Line::Option { value: current, .. } = &mut lines[index] {
            *current = value.to_owned();
        }
    } else {
        push_option(lines, key, value);
    }
}

/// Writes `text`, copying an existing file to `config.bak` first.
pub fn write_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        std::fs::copy(path, path.with_extension("bak"))?;
    }
    std::fs::write(path, text)
}

/// Splits an option line into key and value (`Key value` or `Key=value`).
pub fn split_option(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let key = &line[..end];
    let rest = line[end..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
    Some((key, rest))
}

fn parse_line(raw: &str) -> Line {
    match split_option(raw) {
        Some((key, value)) => {
            let indent = raw[..raw.len() - raw.trim_start().len()].to_owned();
            Line::Option {
                indent,
                key: key.to_owned(),
                value: value.to_owned(),
            }
        }
        None => Line::Raw(raw.to_owned()),
    }
}

fn write_lines(out: &mut String, lines: &[Line]) {
    for line in lines {
        match line {
            Line::Option { indent, key, value } => {
                let (key, value) = (key.trim(), value.trim());
                if key.is_empty() || value.is_empty() {
                    continue;
                }
                out.push_str(indent);
                out.push_str(key);
                out.push(' ');
                out.push_str(value);
            }
            Line::Raw(text) => out.push_str(text),
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_comments() {
        let text =
            "# top\nUser me\n\nHost a b\n    HostName a.example\n    # note\n    Port=2222\n";
        let doc = Document::parse(text);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].patterns, "a b");
        assert_eq!(
            doc.to_text(),
            "# top\nUser me\n\nHost a b\n    HostName a.example\n    # note\n    Port 2222\n"
        );
    }

    #[test]
    fn add_host_goes_before_wildcard() {
        let mut doc = Document::parse("Host a\nHost *\n    User x\n");
        let idx = doc.add_host("b");
        assert_eq!(idx, 1);
        assert_eq!(doc.blocks[2].patterns, "*");
    }

    #[test]
    fn set_option_replaces_or_appends() {
        let mut lines = vec![Line::option("User", "a"), Line::Raw(String::new())];
        set_option(&mut lines, "User", "b");
        set_option(&mut lines, "Port", "22");
        set_option(&mut lines, "HostName", "");
        assert!(matches!(&lines[0], Line::Option { value, .. } if value == "b"));
        assert!(
            matches!(&lines[1], Line::Option { key, value, .. } if key == "Port" && value == "22")
        );
        assert!(matches!(&lines[2], Line::Raw(_)));
    }
}
