//! Painting of the terminal grid.
//!
//! Consecutive ASCII cells with identical style are drawn as one text run
//! (the monospace font keeps them aligned); everything else is placed per
//! cell so that wide and fallback-font glyphs stay on the grid.

use alacritty_terminal::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::point_to_viewport;
use alacritty_terminal::vte::ansi::CursorShape;
use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::terminal::{Listener, palette};
use crate::ui::fonts::TermFont;
use crate::ui::theme;

pub struct Frame<'a> {
    pub painter: &'a Painter,
    pub origin: Pos2,
    pub font: &'a TermFont,
    pub focused: bool,
    pub preedit: &'a str,
}

struct Run {
    row: usize,
    col: usize,
    end: usize,
    color: Color32,
    bold: bool,
    text: String,
}

impl Frame<'_> {
    fn cell_rect(&self, col: usize, row: usize, width: usize) -> Rect {
        let c = self.font.cell;
        Rect::from_min_size(
            self.origin + Vec2::new(col as f32 * c.x, row as f32 * c.y),
            Vec2::new(width as f32 * c.x, c.y),
        )
    }

    fn text(&self, pos: Pos2, text: &str, color: Color32, bold: bool) {
        self.painter.text(pos, Align2::LEFT_TOP, text, self.font.id.clone(), color);
        if bold {
            self.painter.text(pos + Vec2::new(0.6, 0.0), Align2::LEFT_TOP, text, self.font.id.clone(), color);
        }
    }

    fn flush(&self, run: &mut Option<Run>) {
        if let Some(r) = run.take() {
            self.text(self.cell_rect(r.col, r.row, 1).min, &r.text, r.color, r.bold);
        }
    }

    /// Paints the visible grid and returns the cursor cell rectangle.
    pub fn paint(&self, term: &Term<Listener>) -> Rect {
        let content = term.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset;
        let selection = content.selection;
        let default_bg = theme::rgb(palette::BACKGROUND);
        let selection_bg = theme::rgb(palette::SELECTION);
        let mut run: Option<Run> = None;
        let mut bg_run: Option<(Rect, Color32)> = None;
        let mut backgrounds = Vec::new();
        let bg_slot = self.painter.add(Shape::Noop);

        for indexed in content.display_iter {
            let cell = indexed.cell;
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let Some(vp) = point_to_viewport(offset, indexed.point) else { continue };
            let (row, col) = (vp.line, vp.column.0);
            let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            let rect = self.cell_rect(col, row, width);

            let bold = flags.contains(Flags::BOLD);
            let mut fg = theme::rgb(palette::resolve(cell.fg, colors, bold, flags.contains(Flags::DIM)));
            let mut bg = theme::rgb(palette::resolve(cell.bg, colors, false, false));
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if selection.is_some_and(|s| s.contains(indexed.point)) {
                bg = selection_bg;
            }
            if flags.contains(Flags::HIDDEN) {
                fg = bg;
            }

            match &mut bg_run {
                Some((r, c)) if *c == bg && r.max.x == rect.min.x && r.min.y == rect.min.y => r.max.x = rect.max.x,
                _ => {
                    if let Some((r, c)) = bg_run.take()
                        && c != default_bg
                    {
                        backgrounds.push(Shape::rect_filled(r, 0.0, c));
                    }
                    bg_run = Some((rect, bg));
                }
            }

            if flags.intersects(Flags::ALL_UNDERLINES) {
                let y = rect.max.y - 1.5;
                self.painter.hline(rect.x_range(), y, Stroke::new(1.0, fg));
            }
            if flags.contains(Flags::STRIKEOUT) {
                self.painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, fg));
            }

            let c = cell.c;
            if c == ' ' || c == '\t' || c == '\0' {
                continue;
            }
            let zerowidth = cell.zerowidth().filter(|z| !z.is_empty());
            if c.is_ascii_graphic() && zerowidth.is_none() {
                match &mut run {
                    Some(r) if r.row == row && r.end == col && r.color == fg && r.bold == bold => {
                        r.text.push(c);
                        r.end += 1;
                    }
                    _ => {
                        self.flush(&mut run);
                        run = Some(Run { row, col, end: col + 1, color: fg, bold, text: c.to_string() });
                    }
                }
            } else {
                self.flush(&mut run);
                let mut s = c.to_string();
                s.extend(zerowidth.into_iter().flatten());
                self.text(rect.min, &s, fg, bold);
            }
        }
        self.flush(&mut run);
        if let Some((r, c)) = bg_run
            && c != default_bg
        {
            backgrounds.push(Shape::rect_filled(r, 0.0, c));
        }
        self.painter.set(bg_slot, Shape::Vec(backgrounds));

        self.paint_cursor(term, content.cursor.shape, content.cursor.point, offset)
    }

    fn paint_cursor(
        &self,
        term: &Term<Listener>,
        shape: CursorShape,
        point: alacritty_terminal::index::Point,
        offset: usize,
    ) -> Rect {
        let Some(vp) = point_to_viewport(offset, point) else { return Rect::NOTHING };
        let cell = &term.grid()[point];
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        let rect = self.cell_rect(vp.column.0, vp.line, width);
        let color = theme::rgb(palette::CURSOR);

        if !self.preedit.is_empty() {
            let galley = self.painter.layout_no_wrap(self.preedit.to_owned(), self.font.id.clone(), theme::TEXT);
            let r = Rect::from_min_size(rect.min, Vec2::new(galley.size().x, rect.height()));
            self.painter.rect_filled(r, 0.0, theme::SURFACE_HOVER);
            self.painter.galley(r.min, galley, theme::TEXT);
            self.painter.hline(r.x_range(), r.max.y - 1.0, Stroke::new(1.0, theme::ACCENT));
            return r;
        }

        match shape {
            CursorShape::Hidden => {}
            CursorShape::Block if self.focused => {
                self.painter.rect_filled(rect, 0.0, color);
                if cell.c != ' ' && cell.c != '\0' {
                    self.text(rect.min, &cell.c.to_string(), theme::rgb(palette::BACKGROUND), false);
                }
            }
            CursorShape::Beam => {
                self.painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())), 0.0, color);
            }
            CursorShape::Underline => {
                let bar = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - 2.0), rect.max);
                self.painter.rect_filled(bar, 0.0, color);
            }
            _ => {
                self.painter.rect_stroke(rect.shrink(0.5), 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
        }
        rect
    }
}
