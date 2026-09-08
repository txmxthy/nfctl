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

/// Content-fitted widths that together span `width`: the columns in `grow`
/// share whatever is left over, in proportion to their fitted widths.
pub fn fill(
    header: &[&str],
    rows: &[Vec<String>],
    width: u16,
    spacing: u16,
    grow: &[usize],
) -> Vec<Constraint> {
    let mut w: Vec<u16> = fit(header, rows)
        .iter()
        .map(|c| match c {
            Constraint::Length(x) | Constraint::Min(x) => *x,
            _ => 0,
        })
        .collect();
    let used: u16 =
        w.iter().sum::<u16>() + spacing * u16::try_from(w.len().saturating_sub(1)).unwrap_or(0);
    let leftover = width.saturating_sub(used);
    let basis: u16 = grow.iter().filter_map(|&i| w.get(i)).sum::<u16>().max(1);
    let mut given = 0;
    for (k, &i) in grow.iter().enumerate() {
        if let Some(col) = w.get_mut(i) {
            let share = if k + 1 == grow.len() {
                leftover - given
            } else {
                leftover * *col / basis
            };
            *col += share;
            given += share;
        }
    }
    w.into_iter().map(Constraint::Length).collect()
}
