//! The pipeline as a row of columns (one per rank) of vertex cards with live
//! numbers. This is what the Mermaid path cannot do: draw the domain directly.

use nfctl_core::service::{PipelineView, VertexView};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::style;

pub const CARD_HEIGHT: u16 = 4;
const CARD_WIDTH: u16 = 22;
const GAP: u16 = 3;

fn fmt_rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |r| format!("{r:.1}/s"))
}

fn fmt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

/// Draw the cards. `selected` highlights one vertex by name.
pub fn render(frame: &mut Frame, area: Rect, view: &PipelineView, selected: Option<&str>) {
    let ranks = view.pipeline.spec.topology.ranks();
    let by_name = |n: &str| view.vertices.iter().find(|v| v.name.as_str() == n);
    let cols = u16::try_from(ranks.len()).unwrap_or(u16::MAX);
    if cols == 0 {
        return;
    }
    let widths: Vec<Constraint> = (0..cols)
        .flat_map(|i| {
            let mut v = vec![Constraint::Length(CARD_WIDTH)];
            if i + 1 < cols {
                v.push(Constraint::Length(GAP));
            }
            v
        })
        .collect();
    let columns = Layout::horizontal(widths).split(area);
    for (r, rank) in ranks.iter().enumerate() {
        let col = columns[r * 2];
        let rows =
            Layout::vertical(rank.iter().map(|_| Constraint::Length(CARD_HEIGHT))).split(col);
        for (i, vertex) in rank.iter().enumerate() {
            if let Some(vv) = by_name(vertex.name.as_str()) {
                card(frame, rows[i], vv, selected == Some(vertex.name.as_str()));
            }
        }
        if r * 2 + 1 < columns.len() {
            arrow(frame, columns[r * 2 + 1], rows.len());
        }
    }
}

fn card(frame: &mut Frame, area: Rect, v: &VertexView, selected: bool) {
    let border = if selected { style::key() } else { style::dim() };
    let block = Block::default().borders(Borders::ALL).border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let head = Line::from(vec![
        Span::styled(v.name.to_string(), style::title()),
        Span::styled(format!("  {}", v.kind.as_str()), style::dim()),
        if v.partitions > 1 {
            Span::styled(format!(" x{}", v.partitions), style::dim())
        } else {
            Span::raw("")
        },
    ]);
    let nums = Line::from(vec![
        Span::raw(fmt_rate(v.rate.m1)),
        Span::styled("  pending ", style::dim()),
        Span::raw(fmt_i64(v.pending.default.or(v.pending.m1))),
    ]);
    frame.render_widget(Paragraph::new(vec![head, nums]), inner);
}

/// A right-pointing arrow in the gap between two rank columns, vertically centred
/// on the first card row so single-path pipelines read as one line.
fn arrow(frame: &mut Frame, area: Rect, _rows: usize) {
    if area.height < 2 || area.width == 0 {
        return;
    }
    let y = area.y + CARD_HEIGHT / 2;
    let line = Rect {
        x: area.x,
        y,
        width: area.width,
        height: 1,
    };
    let s = format!("{}▶", "─".repeat(usize::from(area.width.saturating_sub(1))));
    frame.render_widget(Paragraph::new(Span::styled(s, style::dim())), line);
}
