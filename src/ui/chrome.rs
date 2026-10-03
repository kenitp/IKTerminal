//! Resize edges for a window without an OS title bar.
//!
//! Dragging an edge asks the OS to resize (`BeginResize`), which keeps snap and
//! the minimum size. The tab bar is the drag region for moving the window.

use egui::{CursorIcon, Id, Rect, ResizeDirection, Sense, Ui, Vec2};

const THICKNESS: f32 = 6.0;
const CORNER: f32 = 16.0;

pub fn resize_borders(ui: &mut Ui) {
    if ui.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let rect = ui.ctx().content_rect();
    if rect.width() < CORNER * 2.0 || rect.height() < CORNER * 2.0 {
        return;
    }
    let edges = [
        (
            "n",
            ResizeDirection::North,
            CursorIcon::ResizeNorth,
            band(rect, Side::North),
        ),
        (
            "s",
            ResizeDirection::South,
            CursorIcon::ResizeSouth,
            band(rect, Side::South),
        ),
        (
            "w",
            ResizeDirection::West,
            CursorIcon::ResizeWest,
            band(rect, Side::West),
        ),
        (
            "e",
            ResizeDirection::East,
            CursorIcon::ResizeEast,
            band(rect, Side::East),
        ),
        (
            "nw",
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNorthWest,
            corner(rect, Corner::NorthWest),
        ),
        (
            "ne",
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNorthEast,
            corner(rect, Corner::NorthEast),
        ),
        (
            "sw",
            ResizeDirection::SouthWest,
            CursorIcon::ResizeSouthWest,
            corner(rect, Corner::SouthWest),
        ),
        (
            "se",
            ResizeDirection::SouthEast,
            CursorIcon::ResizeSouthEast,
            corner(rect, Corner::SouthEast),
        ),
    ];
    for (id, direction, cursor, area) in edges {
        let response = ui.interact(
            area,
            Id::new(("resize", id, ui.ctx().viewport_id())),
            Sense::click_and_drag(),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if response.drag_started() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

enum Side {
    North,
    South,
    West,
    East,
}

enum Corner {
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

fn band(rect: Rect, side: Side) -> Rect {
    match side {
        Side::North => Rect::from_min_size(
            egui::pos2(rect.min.x + CORNER, rect.min.y),
            Vec2::new(rect.width() - CORNER * 2.0, THICKNESS),
        ),
        Side::South => Rect::from_min_size(
            egui::pos2(rect.min.x + CORNER, rect.max.y - THICKNESS),
            Vec2::new(rect.width() - CORNER * 2.0, THICKNESS),
        ),
        Side::West => Rect::from_min_size(
            egui::pos2(rect.min.x, rect.min.y + CORNER),
            Vec2::new(THICKNESS, rect.height() - CORNER * 2.0),
        ),
        Side::East => Rect::from_min_size(
            egui::pos2(rect.max.x - THICKNESS, rect.min.y + CORNER),
            Vec2::new(THICKNESS, rect.height() - CORNER * 2.0),
        ),
    }
}

fn corner(rect: Rect, which: Corner) -> Rect {
    let size = Vec2::splat(CORNER);
    let origin = match which {
        Corner::NorthWest => rect.min,
        Corner::NorthEast => egui::pos2(rect.max.x - CORNER, rect.min.y),
        Corner::SouthWest => egui::pos2(rect.min.x, rect.max.y - CORNER),
        Corner::SouthEast => rect.max - size,
    };
    Rect::from_min_size(origin, size)
}
