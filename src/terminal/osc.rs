//! Directory reports carried in OSC sequences.
//!
//! Shells that integrate with the terminal announce the working directory as
//! OSC 7 (`file://`), OSC 9;9, or OSC 1337 `CurrentDir`. A directory probe uses
//! OSC 1337 `IkHome` for the home directory.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscDir {
    Cwd(String),
    Home(String),
}

pub struct OscSniffer {
    buf: Vec<u8>,
}

impl OscSniffer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<OscDir> {
        self.buf.extend_from_slice(data);
        if self.buf.len() > 8192 {
            let drop = self.buf.len() - 4096;
            self.buf.drain(..drop);
        }
        let mut found = Vec::new();
        loop {
            let Some(start) = self.buf.windows(2).position(|w| w == [0x1b, b']']) else {
                if self.buf.last() == Some(&0x1b) {
                    self.buf.clear();
                    self.buf.push(0x1b);
                } else {
                    self.buf.clear();
                }
                break;
            };
            if start > 0 {
                self.buf.drain(..start);
            }
            let Some(end) = osc_end(&self.buf[2..]) else {
                break;
            };
            let payload = self.buf[2..2 + end.payload].to_vec();
            self.buf.drain(..2 + end.total);
            if let Some(dir) = parse_payload(&payload) {
                found.push(dir);
            }
        }
        found
    }
}

struct End {
    payload: usize,
    total: usize,
}

fn osc_end(data: &[u8]) -> Option<End> {
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x07 {
            return Some(End {
                payload: i,
                total: i + 1,
            });
        }
        if data[i] == 0x1b {
            if i + 1 >= data.len() {
                return None;
            }
            if data[i + 1] == b'\\' {
                return Some(End {
                    payload: i,
                    total: i + 2,
                });
            }
        }
        i += 1;
    }
    None
}

fn parse_payload(payload: &[u8]) -> Option<OscDir> {
    let text = String::from_utf8_lossy(payload);
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("7;") {
        return file_url_path(rest).map(OscDir::Cwd);
    }
    if let Some(rest) = text.strip_prefix("9;9;") {
        let path = tidy(rest);
        return is_remote_path(&path).then_some(OscDir::Cwd(path));
    }
    let rest = text.strip_prefix("1337;")?;
    for part in rest.split(';') {
        if let Some(path) = part.strip_prefix("CurrentDir=") {
            let path = tidy(&percent_decode(path));
            if is_remote_path(&path) {
                return Some(OscDir::Cwd(path));
            }
        }
        if let Some(path) = part.strip_prefix("IkHome=") {
            let path = tidy(&percent_decode(path));
            if is_remote_path(&path) {
                return Some(OscDir::Home(path));
            }
        }
    }
    None
}

fn file_url_path(value: &str) -> Option<String> {
    let rest = value.trim().strip_prefix("file://")?;
    let path = rest.find('/').map(|i| &rest[i..])?;
    let path = tidy(&percent_decode(path));
    is_remote_path(&path).then_some(path)
}

fn tidy(path: &str) -> String {
    let path = path.trim();
    if path.len() > 1 && path.ends_with('/') {
        path.trim_end_matches('/').to_owned()
    } else {
        path.to_owned()
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn is_remote_path(path: &str) -> bool {
    if path.is_empty() || path.contains(['\n', '\r']) {
        return false;
    }
    path.starts_with('/') || path.starts_with('~') || windows_drive(path)
}

fn windows_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_across_chunks() {
        let mut sniffer = OscSniffer::new();
        assert!(sniffer.push(b"\x1b]7;file://host/ho").is_empty());
        let found = sniffer.push(b"me/user/my%20app\x07ready");
        assert_eq!(found, vec![OscDir::Cwd("/home/user/my app".to_owned())]);
        assert!(sniffer.push(b"more").is_empty());
    }

    #[test]
    fn osc_1337_and_st_terminator() {
        let mut sniffer = OscSniffer::new();
        let found = sniffer.push(b"\x1b]1337;CurrentDir=/var/log\x1b\\");
        assert_eq!(found, vec![OscDir::Cwd("/var/log".to_owned())]);
        let found = sniffer.push(b"\x1b]1337;IkHome=/home/user\x07");
        assert_eq!(found, vec![OscDir::Home("/home/user".to_owned())]);
    }
}
