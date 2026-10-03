//! System font discovery. Font files are memory-mapped instead of copied
//! into the heap, so large CJK fonts cost almost no private memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily, FontId, Vec2};

use crate::settings::Settings;

pub const TERM_FAMILY: &str = "term";

#[cfg(windows)]
mod candidates {
    pub const MONO: &[(&str, u32)] = &[("CascadiaMono.ttf", 0), ("consola.ttf", 0)];
    pub const CJK_MONO: &[(&str, u32)] =
        &[("BIZ-UDGothicR.ttc", 0), ("YuGothM.ttc", 0), ("meiryo.ttc", 0), ("msgothic.ttc", 0)];
    /// Yu Gothic UI / Meiryo UI cover Latin and Japanese with one baseline.
    pub const UI: &[(&str, u32)] = &[("YuGothM.ttc", 1), ("meiryo.ttc", 2), ("segoeui.ttf", 0)];
    pub const UI_FALLBACK: &[(&str, u32)] = &[];
    pub const SYMBOLS: &[(&str, u32)] = &[("seguisym.ttf", 0)];
}

#[cfg(not(windows))]
mod candidates {
    pub const MONO: &[(&str, u32)] = &[
        ("truetype/jetbrains-mono/JetBrainsMono-Regular.ttf", 0),
        ("truetype/cascadia/CascadiaMono.ttf", 0),
        ("truetype/dejavu/DejaVuSansMono.ttf", 0),
        ("TTF/DejaVuSansMono.ttf", 0),
        ("truetype/liberation/LiberationMono-Regular.ttf", 0),
        ("truetype/noto/NotoSansMono-Regular.ttf", 0),
    ];
    pub const CJK_MONO: &[(&str, u32)] = &[
        ("opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("opentype/noto/NotoSansCJK-VF.ttc", 0),
        ("opentype/noto/NotoSansCJKjp-Regular.otf", 0),
        ("noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ("noto-cjk/NotoSansCJK-VF.ttc", 0),
        ("google-noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ("google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc", 0),
        ("opentype/ipafont-gothic/ipag.ttf", 0),
        ("truetype/ipafont-gothic/ipag.ttf", 0),
    ];
    pub const UI: &[(&str, u32)] = CJK_MONO;
    pub const UI_FALLBACK: &[(&str, u32)] = &[
        ("truetype/noto/NotoSans-Regular.ttf", 0),
        ("truetype/dejavu/DejaVuSans.ttf", 0),
        ("TTF/DejaVuSans.ttf", 0),
    ];
    pub const SYMBOLS: &[(&str, u32)] = &[
        ("truetype/noto/NotoSansSymbols2-Regular.ttf", 0),
        ("truetype/noto/NotoSansSymbols-Regular.ttf", 0),
    ];
}

fn font_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        let mut dirs = vec![root.join("Fonts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
        dirs
    } else {
        let mut dirs = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            dirs.push(PathBuf::from(data).join("fonts"));
        } else if let Some(home) = std::env::home_dir() {
            dirs.push(home.join(".local/share/fonts"));
        }
        if let Some(home) = std::env::home_dir() {
            dirs.push(home.join(".fonts"));
        }
        dirs
    }
}

/// Maps a font file for the lifetime of the process (each file at most once).
fn map_file(path: &Path) -> Option<&'static [u8]> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(bytes) = cache.get(path) {
        return Some(bytes);
    }
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: system font files are not modified while the application runs.
    let mmap = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let bytes: &'static [u8] = &Box::leak(Box::new(mmap))[..];
    cache.insert(path.to_owned(), bytes);
    Some(bytes)
}

struct Builder {
    defs: FontDefinitions,
    dirs: Vec<PathBuf>,
}

impl Builder {
    /// Registers the first available candidate and returns its key.
    fn first(&mut self, candidates: &[(&str, u32)]) -> Option<String> {
        let dirs = self.dirs.clone();
        candidates
            .iter()
            .find_map(|(name, index)| dirs.iter().map(|d| d.join(name)).find_map(|p| self.register(&p, *index)))
    }

    fn register(&mut self, path: &Path, index: u32) -> Option<String> {
        let key = format!("{}#{index}", path.display());
        if !self.defs.font_data.contains_key(&key) {
            let bytes = map_file(path)?;
            self.defs.font_data.insert(key.clone(), Arc::new(FontData { index, ..FontData::from_static(bytes) }));
        }
        Some(key)
    }

    fn prepend(&mut self, family: FontFamily, keys: &[Option<String>]) {
        let list = self.defs.families.entry(family).or_default();
        for key in keys.iter().flatten().rev() {
            list.insert(0, key.clone());
        }
    }
}

pub fn install(ctx: &egui::Context, settings: &Settings) {
    let mut b = Builder { defs: FontDefinitions::default(), dirs: font_dirs() };
    let user = Some(settings.font_path.trim()).filter(|p| !p.is_empty()).and_then(|p| b.register(Path::new(p), 0));
    let mono = user.or_else(|| b.first(candidates::MONO));
    let cjk_mono = b.first(candidates::CJK_MONO);
    let symbols = b.first(candidates::SYMBOLS);
    let ui = b.first(candidates::UI).or_else(|| b.first(candidates::UI_FALLBACK));

    let monospace = b.defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    b.defs.families.insert(FontFamily::Name(TERM_FAMILY.into()), monospace);
    let term_keys = [mono, cjk_mono.clone(), symbols.clone()];
    b.prepend(FontFamily::Name(TERM_FAMILY.into()), &term_keys);
    b.prepend(FontFamily::Monospace, &term_keys);
    b.prepend(FontFamily::Proportional, &[ui, cjk_mono, symbols]);
    ctx.set_fonts(b.defs);
}

/// Terminal font and the size of one character cell.
#[derive(Clone)]
pub struct TermFont {
    pub id: FontId,
    pub cell: Vec2,
}

impl TermFont {
    pub fn new(ctx: &egui::Context, size: f32) -> Self {
        let id = FontId::new(size, FontFamily::Name(TERM_FAMILY.into()));
        let (w, h) = ctx.fonts_mut(|f| (f.glyph_width(&id, 'M'), f.row_height(&id)));
        Self { id, cell: Vec2::new(w.max(1.0), h.max(1.0)) }
    }
}
