//! The pipeline as columns of vertex cards (one column per rank) with every
//! edge drawn: adjacent edges through the gap between columns, edges that skip
//! columns or point backwards through lane rows under the cards.

use std::collections::HashMap;

use nfctl_core::model::Topology;
use nfctl_core::service::{PipelineView, VertexView};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::style;

pub const CARD_HEIGHT: u16 = 4;
const GAP: u16 = 5;
const MIN_CARD: u16 = 18;

/// Where every card goes, plus how tall the whole drawing is.
pub struct Plan {
    pub card_w: u16,
    pub cols: Vec<Vec<String>>,
    pub place: HashMap<String, (usize, usize)>,
    /// Edges that cannot go through a gap: (from, to), each gets a lane row.
    pub lanes: Vec<(String, String)>,
}

impl Plan {
    pub fn new(t: &Topology, width: u16) -> Self {
        let ranks = t.ranks();
        let cols: Vec<Vec<String>> = ranks
            .iter()
            .map(|r| r.iter().map(|v| v.name.to_string()).collect())
            .collect();
        let mut place = HashMap::new();
        for (c, col) in cols.iter().enumerate() {
            for (r, name) in col.iter().enumerate() {
                place.insert(name.clone(), (c, r));
            }
        }
        // Card width: widest "name  kind" plus borders, bounded by what the area allows.
        let want = t
            .vertices()
            .iter()
            .map(|v| v.name.as_str().width() + v.kind.as_str().len() + 6)
            .max()
            .unwrap_or(0);
        let want = u16::try_from(want).unwrap_or(u16::MAX).max(MIN_CARD);
        let n = u16::try_from(cols.len().max(1)).unwrap_or(1);
        let fits = width.saturating_sub(GAP * (n - 1)) / n;
        let card_w = want.min(fits).max(MIN_CARD.min(fits.max(8)));
        let lanes = t
            .edges()
            .iter()
            .filter(|e| {
                let (Some(&(a, _)), Some(&(b, _))) =
                    (place.get(e.from.as_str()), place.get(e.to.as_str()))
                else {
                    return false;
                };
                b != a + 1
            })
            .map(|e| (e.from.to_string(), e.to.to_string()))
            .collect();
        Self {
            card_w,
            cols,
            place,
            lanes,
        }
    }

    pub fn height(&self) -> u16 {
        let tallest = self.cols.iter().map(Vec::len).max().unwrap_or(1);
        u16::try_from(tallest).unwrap_or(1) * CARD_HEIGHT
            + u16::try_from(self.lanes.len()).unwrap_or(0)
    }

    fn card_rect(&self, area: Rect, c: usize, r: usize) -> Rect {
        let x = area.x + u16::try_from(c).unwrap_or(0) * (self.card_w + GAP);
        let y = area.y + u16::try_from(r).unwrap_or(0) * CARD_HEIGHT;
        Rect {
            x,
            y,
            width: self.card_w,
            height: CARD_HEIGHT,
        }
    }
}

// ---- edge canvas ---------------------------------------------------------

const L: u8 = 1;
const R: u8 = 2;
const U: u8 = 4;
const D: u8 = 8;

struct Canvas {
    w: usize,
    h: usize,
    cells: Vec<u8>,
    heads: Vec<bool>,
}

impl Canvas {
    fn new(w: u16, h: u16) -> Self {
        let (w, h) = (usize::from(w), usize::from(h));
        Self {
            w,
            h,
            cells: vec![0; w * h],
            heads: vec![false; w * h],
        }
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        (x < self.w && y < self.h).then_some(y * self.w + x)
    }

    fn set(&mut self, x: i32, y: i32, bits: u8) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] |= bits;
        }
    }

    fn head(&mut self, x: i32, y: i32) {
        if let Some(i) = self.idx(x, y) {
            self.heads[i] = true;
        }
    }

    fn hline(&mut self, y: i32, x0: i32, x1: i32) {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let mut bits = 0;
            if x > a {
                bits |= L;
            }
            if x < b {
                bits |= R;
            }
            self.set(x, y, bits);
        }
    }

    fn vline(&mut self, x: i32, y0: i32, y1: i32) {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let mut bits = 0;
            if y > a {
                bits |= U;
            }
            if y < b {
                bits |= D;
            }
            self.set(x, y, bits);
        }
    }

    /// Orthogonal path through the given corner points.
    fn path(&mut self, pts: &[(i32, i32)]) {
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if y0 == y1 {
                self.hline(y0, x0, x1);
            } else {
                self.vline(x0, y0, y1);
            }
        }
        // Joints: a corner cell needs both directions set, which hline/vline do
        // because each segment includes its endpoints.
    }

    fn glyph(bits: u8) -> char {
        match bits {
            0 => ' ',
            x if x == L || x == R || x == (L | R) => '─',
            x if x == U || x == D || x == (U | D) => '│',
            x if x == (R | D) => '┌',
            x if x == (L | D) => '┐',
            x if x == (R | U) => '└',
            x if x == (L | U) => '┘',
            x if x == (L | R | D) => '┬',
            x if x == (L | R | U) => '┴',
            x if x == (U | D | R) => '├',
            x if x == (U | D | L) => '┤',
            _ => '┼',
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        (0..self.h)
            .map(|y| {
                let s: String = (0..self.w)
                    .map(|x| {
                        let i = y * self.w + x;
                        if self.heads[i] {
                            '▶'
                        } else {
                            Self::glyph(self.cells[i])
                        }
                    })
                    .collect();
                Line::styled(s, style::dim())
            })
            .collect()
    }
}

/// Draw cards and edges. `selected` highlights one vertex by name.
pub fn render(frame: &mut Frame, area: Rect, view: &PipelineView, selected: Option<&str>) {
    let t = &view.pipeline.spec.topology;
    let plan = Plan::new(t, area.width);
    let by_name = |n: &str| view.vertices.iter().find(|v| v.name.as_str() == n);
    if plan.cols.is_empty() || area.height == 0 {
        return;
    }

    // Edges first, cards on top.
    let mut canvas = Canvas::new(area.width, area.height);
    let cards_h = i32::from(plan.height() - u16::try_from(plan.lanes.len()).unwrap_or(0));
    let rel = |r: Rect| (i32::from(r.x - area.x), i32::from(r.y - area.y));
    let mut lane = 0i32;
    for e in t.edges() {
        let (Some(&(ca, ra)), Some(&(cb, rb))) = (
            plan.place.get(e.from.as_str()),
            plan.place.get(e.to.as_str()),
        ) else {
            continue;
        };
        let (ax, ay) = rel(plan.card_rect(area, ca, ra));
        let (bx, by) = rel(plan.card_rect(area, cb, rb));
        let w = i32::from(plan.card_w);
        let mid_y = |y: i32| y + i32::from(CARD_HEIGHT) / 2 - 1; // the numbers row
        let (x_out, y_out) = (ax + w, mid_y(ay));
        let (x_in, y_in) = (bx - 1, mid_y(by));
        if cb == ca + 1 {
            // Through the gap: out, across to its middle, down/up, across, in.
            let xm = x_out + i32::from(GAP) / 2;
            canvas.path(&[(x_out, y_out), (xm, y_out), (xm, y_in), (x_in, y_in)]);
        } else {
            // Through a lane row under the cards.
            let ly = cards_h + lane;
            lane += 1;
            let xa = x_out + i32::from(GAP) / 2;
            let xb = if bx > 0 {
                bx - i32::from(GAP) / 2 - 1
            } else {
                0
            };
            canvas.path(&[
                (x_out, y_out),
                (xa, y_out),
                (xa, ly),
                (xb, ly),
                (xb, y_in),
                (x_in, y_in),
            ]);
        }
        canvas.head(x_in, y_in);
    }
    frame.render_widget(Paragraph::new(canvas.lines()), area);

    for (c, col) in plan.cols.iter().enumerate() {
        for (r, name) in col.iter().enumerate() {
            let rect = plan.card_rect(area, c, r);
            if rect.right() > area.right() || rect.bottom() > area.bottom() {
                continue;
            }
            if let Some(vv) = by_name(name) {
                card(frame, rect, vv, selected == Some(name.as_str()));
            }
        }
    }
}

fn fmt_rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |r| format!("{r:.1}/s"))
}

fn fmt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

fn card(frame: &mut Frame, area: Rect, v: &VertexView, selected: bool) {
    let border = if selected { style::key() } else { style::dim() };
    let block = Block::default().borders(Borders::ALL).border_style(border);
    let inner = block.inner(area);
    frame.render_widget(ratatui::widgets::Clear, area);
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
