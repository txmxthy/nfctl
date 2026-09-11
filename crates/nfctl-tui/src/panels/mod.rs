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

/// A spiral that turns: eight braille cells whose weight travels round the
/// glyph, so it reads as a coil rather than a blinking dot.
const SPIRAL: [char; 8] = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

/// The frame of the spiral to draw now.
#[must_use]
pub fn spiral(frame: usize) -> char {
    SPIRAL[frame % SPIRAL.len()]
}

/// A turning spiral above what it is waiting for, in the middle of `area`.
pub fn waiting(frame: &mut ratatui::Frame, area: Rect, spin: usize, what: &str) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;
    let width = u16::try_from(what.chars().count()).unwrap_or(0).max(1);
    let at = centre(area, width, 3);
    let lines = vec![
        Line::from(Span::styled(spiral(spin).to_string(), crate::style::key())).centered(),
        Line::default(),
        Line::from(Span::styled(what.to_owned(), crate::style::dim())).centered(),
    ];
    frame.render_widget(Paragraph::new(lines), at);
}
