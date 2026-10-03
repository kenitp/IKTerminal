//! Encoding of keyboard and mouse input into xterm escape sequences.

use egui::{Key, Modifiers};

/// xterm modifier parameter (1 = none).
fn modifier_param(m: Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8
}

fn with_alt(m: Modifiers, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if m.alt {
        out.push(0x1b);
    }
    out.extend_from_slice(bytes);
    out
}

/// Bytes for a key press, or `None` when the key produces text handled elsewhere.
pub fn encode_key(key: Key, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let p = modifier_param(m);
    let cursor = |c: char| -> Vec<u8> {
        match (p, app_cursor) {
            (1, true) => format!("\x1bO{c}"),
            (1, false) => format!("\x1b[{c}"),
            _ => format!("\x1b[1;{p}{c}"),
        }
        .into_bytes()
    };
    let tilde = |n: u8| -> Vec<u8> {
        if p == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{p}~") }.into_bytes()
    };
    let ss3 = |c: char| -> Vec<u8> {
        if p == 1 { format!("\x1bO{c}") } else { format!("\x1b[1;{p}{c}") }.into_bytes()
    };

    let bytes = match key {
        Key::ArrowUp => cursor('A'),
        Key::ArrowDown => cursor('B'),
        Key::ArrowRight => cursor('C'),
        Key::ArrowLeft => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => ss3('P'),
        Key::F2 => ss3('Q'),
        Key::F3 => ss3('R'),
        Key::F4 => ss3('S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter => with_alt(m, b"\r"),
        Key::Escape => b"\x1b".to_vec(),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => with_alt(m, b"\t"),
        Key::Backspace if m.ctrl => with_alt(m, b"\x08"),
        Key::Backspace => with_alt(m, b"\x7f"),
        _ if m.ctrl => with_alt(m, &[ctrl_byte(key)?]),
        _ if m.alt => with_alt(m, alt_text(key, m.shift)?.as_bytes()),
        _ => return None,
    };
    Some(bytes)
}

fn ctrl_byte(key: Key) -> Option<u8> {
    let name = key.name();
    if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
        return Some(name.as_bytes()[0] - b'A' + 1);
    }
    Some(match key {
        Key::Space | Key::Num2 => 0x00,
        Key::OpenBracket | Key::Num3 => 0x1b,
        Key::Backslash | Key::Num4 => 0x1c,
        Key::CloseBracket | Key::Num5 => 0x1d,
        Key::Num6 => 0x1e,
        Key::Minus | Key::Slash | Key::Num7 => 0x1f,
        Key::Num8 => 0x7f,
        _ => return None,
    })
}

fn alt_text(key: Key, shift: bool) -> Option<String> {
    let name = key.name();
    let c = name.chars().next().filter(|c| name.len() == 1 && c.is_ascii_alphanumeric())?;
    Some(if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }.to_string())
}

/// Text to send for a paste, honoring bracketed paste mode.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", normalized.replace('\x1b', "")).into_bytes()
    } else {
        normalized.into_bytes()
    }
}

/// Button codes used in mouse reports.
pub const BUTTON_LEFT: u8 = 0;
pub const BUTTON_MIDDLE: u8 = 1;
pub const BUTTON_RIGHT: u8 = 2;
pub const BUTTON_RELEASE: u8 = 3;
pub const MOTION: u8 = 32;
pub const WHEEL_UP: u8 = 64;
pub const WHEEL_DOWN: u8 = 65;

/// Mouse report in SGR (1006) or legacy X10 encoding. `col`/`row` are 0-based.
pub fn encode_mouse(button: u8, pressed: bool, col: usize, row: usize, m: Modifiers, sgr: bool) -> Option<Vec<u8>> {
    let mods = 4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.ctrl as u8;
    if sgr {
        let suffix = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{};{};{}{suffix}", button + mods, col + 1, row + 1).into_bytes());
    }
    if col >= 223 || row >= 223 {
        return None;
    }
    let b = if pressed { button } else { BUTTON_RELEASE };
    Some(vec![0x1b, b'[', b'M', 32 + b + mods, 33 + col as u8, 33 + row as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        let none = Modifiers::NONE;
        assert_eq!(encode_key(Key::ArrowUp, none, false).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(Key::ArrowUp, none, true).unwrap(), b"\x1bOA");
        assert_eq!(encode_key(Key::ArrowLeft, Modifiers::CTRL, false).unwrap(), b"\x1b[1;5D");
        assert_eq!(encode_key(Key::C, Modifiers::CTRL, false).unwrap(), b"\x03");
        assert_eq!(encode_key(Key::X, Modifiers::ALT, false).unwrap(), b"\x1bx");
        assert!(encode_key(Key::A, none, false).is_none());
    }

    #[test]
    fn paste_and_mouse() {
        assert_eq!(encode_paste("a\nb", true), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(encode_mouse(WHEEL_UP, true, 0, 0, Modifiers::NONE, true).unwrap(), b"\x1b[<64;1;1M");
    }
}
