//! Terminal widget: input handling, selection and mouse reporting.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{TermMode, viewport_to_point};
use egui::{
    CursorIcon, Event, EventFilter, Id, ImeEvent, MouseWheelUnit, PointerButton, Pos2, Rect, Sense,
    Stroke, Ui, Vec2,
};

use super::input::{self, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, MOTION, WHEEL_DOWN, WHEEL_UP};
use super::render;
use crate::session::Session;
use crate::ui::fonts::TermFont;
use crate::ui::theme;

const PADDING: f32 = 6.0;
const SCROLL_LINES: f32 = 3.0;

#[derive(Clone, Default)]
struct ViewState {
    scroll_acc: f32,
    preedit: String,
    held_button: Option<u8>,
    last_cell: Option<(usize, usize)>,
}

pub struct Output {
    /// Requested font size change (Ctrl + wheel).
    pub zoom: f32,
    /// Shell line submitted with Enter, including the prompt.
    pub command_line: Option<String>,
}

struct Grid {
    origin: Pos2,
    cell: Vec2,
    cols: usize,
    rows: usize,
}

impl Grid {
    fn cell_at(&self, pos: Pos2) -> (usize, usize, Side) {
        let rel = pos - self.origin;
        let x = (rel.x / self.cell.x).max(0.0);
        let col = (x as usize).min(self.cols - 1);
        let row = ((rel.y / self.cell.y).max(0.0) as usize).min(self.rows - 1);
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (col, row, side)
    }
}

pub fn show(ui: &mut Ui, session: &mut Session, font: &TermFont, want_focus: bool) -> Output {
    let rect = ui.available_rect_before_wrap();
    let id = Id::new(("terminal", session.id));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let inner = rect.shrink(PADDING);
    let grid = Grid {
        origin: inner.min,
        cell: font.cell,
        cols: ((inner.width() / font.cell.x) as usize).max(2),
        rows: ((inner.height() / font.cell.y) as usize).max(1),
    };
    session.resize(grid.cols, grid.rows, font.cell.x, font.cell.y);

    if want_focus || response.clicked() || response.drag_started() || response.secondary_clicked() {
        response.request_focus();
    }
    let focused = response.has_focus();
    if focused {
        let filter = EventFilter {
            tab: true,
            horizontal_arrows: true,
            vertical_arrows: true,
            escape: true,
        };
        ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
    }

    let mut state: ViewState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut out = Output {
        zoom: 0.0,
        command_line: None,
    };
    let (events, modifiers, press_origin) =
        ui.input(|i| (i.events.clone(), i.modifiers, i.pointer.press_origin()));
    let hovered = response.contains_pointer();

    let mut term = session.term.lock();
    let mode = *term.mode();
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift;
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let mut send: Vec<u8> = Vec::new();
    let mut typed = false;

    for event in &events {
        match event {
            Event::Text(text) if focused && (!modifiers.alt || modifiers.ctrl) => {
                send.extend_from_slice(text.as_bytes());
                typed = true;
            }
            Event::Ime(ImeEvent::Preedit { text, .. }) if focused => state.preedit = text.clone(),
            Event::Ime(ImeEvent::Commit(text)) if focused => {
                state.preedit.clear();
                send.extend_from_slice(text.as_bytes());
                typed = true;
            }
            Event::Key {
                key,
                pressed: true,
                modifiers: m,
                ..
            } if focused => {
                if *key == egui::Key::Enter && m.is_none() && !mode.contains(TermMode::ALT_SCREEN) {
                    out.command_line = Some(submitted_line(&term));
                }
                let page = match key {
                    egui::Key::PageUp => Some(Scroll::PageUp),
                    egui::Key::PageDown => Some(Scroll::PageDown),
                    _ => None,
                };
                if let Some(scroll) =
                    page.filter(|_| m.shift && !m.ctrl && !mode.contains(TermMode::ALT_SCREEN))
                {
                    term.scroll_display(scroll);
                } else if let Some(bytes) =
                    input::encode_key(*key, *m, mode.contains(TermMode::APP_CURSOR))
                {
                    send.extend(bytes);
                    typed = true;
                }
            }
            Event::Copy if focused => match term.selection_to_string().filter(|s| !s.is_empty()) {
                Some(text) => {
                    ui.ctx().copy_text(text);
                    term.selection = None;
                }
                None => {
                    send.push(0x03);
                    typed = true;
                }
            },
            Event::Cut if focused => {
                send.extend_from_slice(if modifiers.ctrl {
                    b"\x18"
                } else {
                    b"\x1b[3;2~"
                });
                typed = true;
            }
            Event::Paste(text) if focused => {
                send.extend(input::encode_paste(
                    text,
                    mode.contains(TermMode::BRACKETED_PASTE),
                ));
                typed = true;
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers: m,
                ..
            } if hovered => {
                if m.ctrl {
                    out.zoom += delta.y.signum();
                    continue;
                }
                state.scroll_acc += match unit {
                    MouseWheelUnit::Line => delta.y * SCROLL_LINES,
                    MouseWheelUnit::Point => delta.y / font.cell.y,
                    MouseWheelUnit::Page => delta.y * grid.rows as f32,
                };
                let lines = state.scroll_acc.trunc() as i32;
                state.scroll_acc -= lines as f32;
                if lines == 0 {
                    continue;
                }
                let pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(inner.min);
                let (col, row, _) = grid.cell_at(pos);
                for _ in 0..lines.unsigned_abs() {
                    if mouse_mode {
                        let button = if lines > 0 { WHEEL_UP } else { WHEEL_DOWN };
                        send.extend(
                            input::encode_mouse(button, true, col, row, *m, sgr)
                                .unwrap_or_default(),
                        );
                    } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                        let dir = if lines > 0 { 'A' } else { 'B' };
                        let prefix = if mode.contains(TermMode::APP_CURSOR) {
                            "\x1bO"
                        } else {
                            "\x1b["
                        };
                        send.extend(format!("{prefix}{dir}").bytes());
                    }
                }
                if !mouse_mode && !mode.contains(TermMode::ALT_SCREEN) {
                    term.scroll_display(Scroll::Delta(lines));
                }
            }
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: m,
            } if mouse_mode && rect.contains(*pos) => {
                let code = match button {
                    PointerButton::Primary => BUTTON_LEFT,
                    PointerButton::Middle => BUTTON_MIDDLE,
                    PointerButton::Secondary => BUTTON_RIGHT,
                    _ => continue,
                };
                let (col, row, _) = grid.cell_at(*pos);
                state.held_button = pressed.then_some(code);
                state.last_cell = Some((col, row));
                send.extend(
                    input::encode_mouse(code, *pressed, col, row, *m, sgr).unwrap_or_default(),
                );
            }
            Event::PointerMoved(pos) if mouse_mode && rect.contains(*pos) => {
                let (col, row, _) = grid.cell_at(*pos);
                if state.last_cell == Some((col, row)) {
                    continue;
                }
                state.last_cell = Some((col, row));
                let button = match state.held_button {
                    Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => b,
                    None if mode.contains(TermMode::MOUSE_MOTION) => input::BUTTON_RELEASE,
                    _ => continue,
                };
                send.extend(
                    input::encode_mouse(button + MOTION, true, col, row, modifiers, sgr)
                        .unwrap_or_default(),
                );
            }
            _ => {}
        }
    }

    if !mouse_mode {
        let offset = term.grid().display_offset();
        let to_point = |pos: Pos2| {
            let (col, row, side) = grid.cell_at(pos);
            (
                viewport_to_point(offset, Point::new(row, Column(col))),
                side,
            )
        };
        if let Some(pos) = response.interact_pointer_pos() {
            let (point, side) = to_point(pos);
            if response.triple_clicked() {
                term.selection = Some(Selection::new(SelectionType::Lines, point, side));
            } else if response.double_clicked() {
                term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
            } else if response.drag_started_by(PointerButton::Primary) {
                let (start, start_side) = to_point(press_origin.unwrap_or(pos));
                let mut selection = Selection::new(SelectionType::Simple, start, start_side);
                selection.update(point, side);
                term.selection = Some(selection);
            } else if response.dragged_by(PointerButton::Primary) {
                if let Some(selection) = term.selection.as_mut() {
                    selection.update(point, side);
                }
            } else if response.clicked() {
                term.selection = None;
            }
        }
        if response.secondary_clicked() {
            match term.selection_to_string().filter(|s| !s.is_empty()) {
                Some(text) => {
                    ui.ctx().copy_text(text);
                    term.selection = None;
                }
                None => {
                    if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        send.extend(input::encode_paste(
                            &text,
                            mode.contains(TermMode::BRACKETED_PASTE),
                        ));
                        typed = true;
                    }
                }
            }
        }
        if hovered {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
    }

    if typed {
        term.selection = None;
        term.scroll_display(Scroll::Bottom);
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::term_bg());
    let frame = render::Frame {
        painter: &painter,
        origin: inner.min,
        font,
        focused,
        preedit: &state.preedit,
    };
    let cursor_rect = frame.paint(&term);
    paint_scroll_indicator(
        &painter,
        rect,
        term.grid().display_offset(),
        term.grid().history_size(),
        grid.rows,
    );
    drop(term);

    if !send.is_empty() {
        session.write(send);
    }
    if focused {
        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                rect: inner,
                cursor_rect,
                should_interrupt_composition: false,
            })
        });
    }
    ui.data_mut(|d| d.insert_temp(id, state));
    out
}

/// The logical line at the cursor, joining rows the terminal wrapped.
fn submitted_line(term: &alacritty_terminal::Term<crate::terminal::Listener>) -> String {
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::cell::Flags;
    let grid = term.grid();
    let cols = grid.columns();
    if cols == 0 {
        return String::new();
    }
    let bottom = grid.cursor.point.line.0;
    let min = -(grid.history_size() as i32);
    let mut start = bottom;
    while start > min {
        let prev = Line(start - 1);
        if !grid[prev][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
            break;
        }
        start -= 1;
    }
    let mut text = String::new();
    for index in start..=bottom {
        for col in 0..cols {
            let cell = &grid[Line(index)][Column(col)];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                text.push(cell.c);
            }
        }
    }
    text.trim().to_owned()
}

fn paint_scroll_indicator(
    painter: &egui::Painter,
    rect: Rect,
    offset: usize,
    history: usize,
    rows: usize,
) {
    if offset == 0 || history == 0 {
        return;
    }
    let total = (history + rows) as f32;
    let height = (rect.height() * rows as f32 / total).max(16.0);
    let top = rect.min.y + (rect.height() - height) * (1.0 - offset as f32 / history as f32);
    let x = rect.max.x - 4.0;
    painter.line_segment(
        [Pos2::new(x, top), Pos2::new(x, top + height)],
        Stroke::new(4.0, theme::SURFACE_HOVER),
    );
}
