//! Content-fitted column widths: each column as wide as its widest cell, the
//! last one taking the rest. Fixed widths either crop names or waste the row.

use ratatui::layout::Constraint;
use unicode_width::UnicodeWidthStr;

/// `header` and `rows` are the cell texts; returns one constraint per column.
pub fn fit(header: &[&str], rows: &[Vec<String>]) -> Vec<Constraint> {
    let n = header.len();
    let mut w: Vec<usize> = header.iter().map(|h| h.width()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate().take(n) {
            w[i] = w[i].max(c.width());
        }
    }
    w.iter()
        .enumerate()
        .map(|(i, &x)| {
            let x = u16::try_from(x).unwrap_or(u16::MAX);
            if i + 1 == n {
                Constraint::Min(x)
            } else {
                Constraint::Length(x)
            }
        })
        .collect()
}
