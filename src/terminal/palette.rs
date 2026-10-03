//! Default color scheme and resolution of terminal cell colors.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

const fn rgb(hex: u32) -> Rgb {
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

pub const FOREGROUND: Rgb = rgb(0xc0caf5);
pub const BACKGROUND: Rgb = rgb(0x1a1b26);
pub const CURSOR: Rgb = rgb(0xc0caf5);
pub const SELECTION: Rgb = rgb(0x33467c);

const ANSI: [Rgb; 16] = [
    rgb(0x15161e),
    rgb(0xf7768e),
    rgb(0x9ece6a),
    rgb(0xe0af68),
    rgb(0x7aa2f7),
    rgb(0xbb9af7),
    rgb(0x7dcfff),
    rgb(0xa9b1d6),
    rgb(0x414868),
    rgb(0xff899d),
    rgb(0xb9f27c),
    rgb(0xffc777),
    rgb(0x8db0ff),
    rgb(0xc7a9ff),
    rgb(0xa4daff),
    rgb(0xc0caf5),
];

fn dim(c: Rgb) -> Rgb {
    Rgb { r: (c.r as u16 * 2 / 3) as u8, g: (c.g as u16 * 2 / 3) as u8, b: (c.b as u16 * 2 / 3) as u8 }
}

/// Default value for any palette index (0..=255 plus the named extras).
pub fn default_color(index: usize) -> Rgb {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let i = index - 16;
            Rgb { r: CUBE[i / 36], g: CUBE[(i / 6) % 6], b: CUBE[i % 6] }
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u8;
            Rgb { r: v, g: v, b: v }
        }
        i if i == NamedColor::Background as usize => BACKGROUND,
        i if i == NamedColor::Cursor as usize => CURSOR,
        i if i == NamedColor::DimForeground as usize => dim(FOREGROUND),
        i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
            dim(ANSI[i - NamedColor::DimBlack as usize])
        }
        _ => FOREGROUND,
    }
}

fn lookup(colors: &Colors, index: usize) -> Rgb {
    colors[index].unwrap_or_else(|| default_color(index))
}

/// Resolves a cell color, applying bold-as-bright and dim attributes.
pub fn resolve(color: Color, colors: &Colors, bold: bool, dimmed: bool) -> Rgb {
    match color {
        Color::Spec(rgb) => {
            if dimmed {
                dim(rgb)
            } else {
                rgb
            }
        }
        Color::Indexed(i) => {
            let mut i = i as usize;
            if bold && i < 8 {
                i += 8;
            }
            let c = lookup(colors, i);
            if dimmed { dim(c) } else { c }
        }
        Color::Named(named) => {
            let named = match (bold, dimmed) {
                (true, false) => named.to_bright(),
                (false, true) => named.to_dim(),
                _ => named,
            };
            lookup(colors, named as usize)
        }
    }
}
