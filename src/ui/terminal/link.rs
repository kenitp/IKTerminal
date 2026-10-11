//! http(s) URL under a terminal cell.
//!
//! Wrapped rows are one line. The click hits the URL when its column falls inside the span.

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::Term;

use crate::terminal::Listener;

pub fn at(term: &Term<Listener>, point: Point) -> Option<String> {
    let grid = term.grid();
    let cols = grid.columns();
    if cols == 0 {
        return None;
    }
    let line = point.line.0;
    let start = line_start(grid, line);
    let end = line_end(grid, line);
    let click = (line - start) as usize * cols + point.column.0;
    let mut text = String::new();
    let mut indexes = Vec::new();
    for index in start..=end {
        for col in 0..cols {
            let cell = &grid[Line(index)][Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                indexes.push(indexes.last().copied().unwrap_or(text.len()));
                continue;
            }
            indexes.push(text.len());
            text.push(cell.c);
        }
    }
    let at = *indexes.get(click)?;
    covering(&text, at).map(str::to_owned)
}

fn line_start(grid: &Grid<Cell>, line: i32) -> i32 {
    let cols = grid.columns();
    let min = grid.topmost_line().0;
    let mut start = line;
    while start > min
        && grid[Line(start - 1)][Column(cols - 1)]
            .flags
            .contains(Flags::WRAPLINE)
    {
        start -= 1;
    }
    start
}

fn line_end(grid: &Grid<Cell>, line: i32) -> i32 {
    let cols = grid.columns();
    let max = grid.bottommost_line().0;
    let mut end = line;
    while end < max
        && grid[Line(end)][Column(cols - 1)]
            .flags
            .contains(Flags::WRAPLINE)
    {
        end += 1;
    }
    end
}

fn covering(text: &str, at: usize) -> Option<&str> {
    if !text.is_char_boundary(at) {
        return None;
    }
    let mut i = 0;
    while i < text.len() {
        if let Some(len) = scheme_len(text, i) {
            let end = url_end(text, i, i + len);
            if (i..end).contains(&at) {
                return Some(&text[i..end]);
            }
            i += len;
            continue;
        }
        i += next_char(text, i);
    }
    None
}

fn scheme_len(text: &str, i: usize) -> Option<usize> {
    let rest = text.get(i..)?.as_bytes();
    let len = if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case(b"https://") {
        8
    } else if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case(b"http://") {
        7
    } else {
        return None;
    };
    if text[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(len)
}

fn url_end(text: &str, start: usize, scheme_end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut end = scheme_end;
    while end < bytes.len() && is_url_byte(bytes[end]) {
        end += 1;
    }
    while end > scheme_end {
        match bytes[end - 1] {
            b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'"' => end -= 1,
            b')' | b']' | b'}' => {
                let (open, close) = match bytes[end - 1] {
                    b')' => (b'(', b')'),
                    b']' => (b'[', b']'),
                    _ => (b'{', b'}'),
                };
                let body = &bytes[start..end];
                let opens = body.iter().filter(|byte| **byte == open).count();
                let closes = body.iter().filter(|byte| **byte == close).count();
                if closes > opens {
                    end -= 1;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    end
}

fn is_url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b':'
                | b'/'
                | b'?'
                | b'#'
                | b'['
                | b']'
                | b'@'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'%'
                | b'='
        )
}

fn next_char(text: &str, i: usize) -> usize {
    let mut j = i + 1;
    while j < text.len() && !text.is_char_boundary(j) {
        j += 1;
    }
    j - i
}

#[cfg(test)]
mod tests {
    use super::covering;

    #[test]
    fn click_inside_an_https_url() {
        let text = "see https://example.com/a?b=1&c=2 next";
        let at = text.find("example").unwrap();
        assert_eq!(
            covering(text, at),
            Some("https://example.com/a?b=1&c=2")
        );
    }

    #[test]
    fn http_scheme_and_trailing_punctuation() {
        let text = "docs: http://example.com/rust_(lang).";
        let at = text.find("http").unwrap();
        assert_eq!(covering(text, at), Some("http://example.com/rust_(lang)"));
        let wrapped = "(https://example.com/foo).";
        assert_eq!(
            covering(wrapped, 1),
            Some("https://example.com/foo")
        );
    }

    #[test]
    fn scheme_must_start_at_a_boundary() {
        let text = "foohttp://example.com";
        assert!(covering(text, text.find("http").unwrap()).is_none());
    }
}
