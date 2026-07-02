//! Mouse support, fully self-contained. The renderer publishes clickable
//! regions into `app.ui.hits` (geometry only); everything here reads that map
//! and the scroll wheel, and acts through the same `AppState` methods the
//! keyboard uses. No mouse logic leaks into render/control/state.

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, MouseButton, MouseEvent, MouseEventKind,
};

use arte_core::state::{AppState, Cursor, Hit, PageKind};

/// Turn mouse reporting on/off (paired with ratatui::init/restore).
pub fn enable() {
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
}
pub fn disable() {
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
}

/// Apply a mouse event to app state. Returns true if anything changed.
pub fn handle(app: &mut AppState, me: MouseEvent) -> bool {
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => click(app, me.column, me.row),
        MouseEventKind::ScrollUp => scroll(app, -1),
        MouseEventKind::ScrollDown => scroll(app, 1),
        _ => false,
    }
}

/// Left click: move the cursor to the grid cell (col, row) under the pointer.
fn click(app: &mut AppState, x: u16, y: u16) -> bool {
    let hit = app
        .ui
        .hits
        .borrow()
        .iter()
        .find(|r| y == r.y && x >= r.x && x < r.x + r.w)
        .map(|r| r.hit);
    match hit {
        Some(Hit::Cell { col, row }) => {
            app.ui.cursor = Cursor { col, row };
            true
        }
        Some(Hit::Button(b)) => {
            app.press(b);
            true
        }
        None => false,
    }
}

/// Wheel: move the selection (board) or walk the tree.
fn scroll(app: &mut AppState, delta: isize) -> bool {
    match app.active_page_kind() {
        PageKind::Board => app.board_select_vert(delta),
        PageKind::Other => return false,
    }
    true
}
