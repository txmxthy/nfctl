pub mod detail;
pub mod logs;
pub mod pipelines;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

/// Only key presses matter; some terminals also report releases and repeats.
pub fn pressed(k: &KeyEvent) -> Option<KeyCode> {
    (k.kind == KeyEventKind::Press).then_some(k.code)
}
