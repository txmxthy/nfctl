pub mod detail;
pub mod logs;
pub mod pipelines;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;

/// Only key presses matter; some terminals also report releases and repeats.
pub fn pressed(k: &KeyEvent) -> Option<KeyCode> {
    (k.kind == KeyEventKind::Press).then_some(k.code)
}

/// A `w` by `h` box in the middle of `area`, or as much of it as fits: a panel
/// smaller than the terminal floats in the middle rather than sitting in a
/// corner with the rest of the screen empty beside it.
#[must_use]
pub fn centre(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
