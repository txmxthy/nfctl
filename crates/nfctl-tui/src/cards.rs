//! The pipeline as columns of vertex cards, painted from
//! [`nfctl_graph::layout::Layout`]: bundled edges through the gaps, long edges
//! straight across pass rows, back edges through lanes under the cards.
//! Shard groups draw as one card; tagged edges take their combination's colour.

use nfctl_core::model::Topology;
use nfctl_core::service::{PipelineView, VertexView};
use nfctl_graph::layout::{
    self, Bundling, EdgeColour, EdgeId, Layout, LayoutOptions, Route, ViewGraph, ViewNode,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::style::{self, CrossingStyle, Palette};

/// Borders, name row, numbers row, badge row: odd, so edges meet a middle row.
pub const CARD_HEIGHT: u16 = 5;
/// Fits `999.9/s pend 999` inside the borders.
const MIN_CARD: u16 = 17;
/// Fits `999.9/s  pending 9999`.
const FULL_CARD: u16 = 24;
const GAP: u16 = 5;

/// A laid-out pipeline: the view graph plus cell coordinates for one width.
pub struct CardView {
    pub graph: ViewGraph,
    pub layout: Layout,
    pub card_w: u16,
}

impl CardView {
    pub fn new(t: &Topology, width: u16, expand_shards: bool, bundling: Bundling) -> Self {
        let graph = if expand_shards {
            ViewGraph::expanded(t)
        } else {
            ViewGraph::collapsed(t)
        };
        // Card width: the label row, or enough for the numbers row, whichever
        // is wider. Cards shrink to `MIN_CARD` when the columns would not fit;
        // past that the panel scrolls instead of squeezing further.
        let want = graph
            .nodes
            .iter()
            .map(|n| n.label.width() + n.kind.as_str().len() + 6)
            .max()
            .unwrap_or(0);
        let want = u16::try_from(want).unwrap_or(u16::MAX).max(FULL_CARD);
        let n = u16::try_from(t.ranks().len().max(1)).unwrap_or(1);
        let fits = width.saturating_sub(GAP * (n - 1)) / n;
        let card_w = if fits >= want {
            want
        } else {
            fits.max(MIN_CARD)
        };
        let layout = layout::layout(
            &graph,
            LayoutOptions {
                bundling,
                card_w,
                card_h: CARD_HEIGHT,
            },
        );
        Self {
            graph,
            layout,
            card_w,
        }
    }

    pub fn height(&self) -> u16 {
        self.layout.height
    }

    /// Column of the card holding `vertex`, if any.
    pub fn column_of(&self, vertex: &str) -> Option<usize> {
        let name = nfctl_core::model::VertexName::new(vertex).ok()?;
        let node = self.graph.node_of(&name)?;
        self.layout.card(node).map(|c| c.col)
    }

    /// First column whose right edge fits when column `first` is at x=0.
    pub fn last_visible(&self, first: usize, width: u16) -> usize {
        let x0 = self.layout.col_x.get(first).copied().unwrap_or(0);
        self.layout
            .col_x
            .iter()
            .rposition(|&x| x - x0 + i32::from(self.card_w) <= i32::from(width))
            .unwrap_or(first)
            .max(first)
    }
}

// ---- edge canvas ---------------------------------------------------------

const L: u8 = 1;
const R: u8 = 2;
const U: u8 = 4;
const D: u8 = 8;

/// One colour per cell: unset, one edge colour, or mixed (painted dim).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ink {
    None,
    One(Option<EdgeColour>),
    Mixed,
}

struct Canvas {
    w: usize,
    h: usize,
    dx: i32,
    cells: Vec<u8>,
    ink: Vec<Ink>,
    /// Colour of the longest vertical run through a cell, with its length:
    /// where a bus mixes colours, the branch that reaches furthest wins, so a
    /// trunk reads as one colour from the source to its far end.
    turn: Vec<(Ink, i32)>,
    heads: Vec<Option<Ink>>,
    /// Which edge left which bits in a cell, for telling a crossing from a
    /// junction after the fact.
    runs: Vec<Vec<(EdgeId, u8)>>,
    /// Glyph and ink painted instead of the cell's bits: the bridge style.
    over: Vec<Option<(char, Ink)>>,
    edge: EdgeId,
}

impl Canvas {
    fn new(w: u16, h: u16, dx: i32) -> Self {
        let (w, h) = (usize::from(w), usize::from(h));
        Self {
            w,
            h,
            dx,
            cells: vec![0; w * h],
            ink: vec![Ink::None; w * h],
            turn: vec![(Ink::None, 0); w * h],
            heads: vec![None; w * h],
            runs: vec![Vec::new(); w * h],
            over: vec![None; w * h],
            edge: EdgeId(0),
        }
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (usize::try_from(x - self.dx).ok()?, usize::try_from(y).ok()?);
        (x < self.w && y < self.h).then_some(y * self.w + x)
    }

    fn set(&mut self, x: i32, y: i32, bits: u8, colour: Option<EdgeColour>, reach: i32) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] |= bits;
            self.ink[i] = match self.ink[i] {
                Ink::None => Ink::One(colour),
                Ink::One(c) if c == colour => Ink::One(c),
                _ => Ink::Mixed,
            };
            if bits & (U | D) != 0 && reach > self.turn[i].1 {
                self.turn[i] = (Ink::One(colour), reach);
            }
            match self.runs[i].iter_mut().find(|(e, _)| *e == self.edge) {
                Some(run) => run.1 |= bits,
                None => self.runs[i].push((self.edge, bits)),
            }
        }
    }

    fn head(&mut self, x: i32, y: i32, colour: Option<EdgeColour>) {
        if let Some(i) = self.idx(x, y) {
            self.heads[i] = Some(match self.heads[i] {
                None => Ink::One(colour),
                Some(Ink::One(c)) if c == colour => Ink::One(c),
                Some(_) => Ink::Mixed,
            });
        }
    }

    fn hline(&mut self, y: i32, x0: i32, x1: i32, colour: Option<EdgeColour>) {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let bits = if x > a { L } else { 0 } | if x < b { R } else { 0 };
            self.set(x, y, bits, colour, 0);
        }
    }

    fn vline(&mut self, x: i32, y0: i32, y1: i32, colour: Option<EdgeColour>) {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let bits = if y > a { U } else { 0 } | if y < b { D } else { 0 };
            self.set(x, y, bits, colour, b - a + 1);
        }
    }

    /// Orthogonal path through the given corner points.
    fn path(&mut self, edge: EdgeId, pts: &[(i32, i32)], colour: Option<EdgeColour>) {
        self.edge = edge;
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if y0 == y1 {
                self.hline(y0, x0, x1, colour);
            } else {
                self.vline(x0, y0, y1, colour);
            }
        }
    }

    /// Bridge a cell where one edge runs straight through horizontally and
    /// another straight through vertically. A junction glyph says the two are
    /// one line, which is true only when they meet at a shared end and carry
    /// the same colour; anything else is bridged, so the cell takes the
    /// vertical's glyph and colour and the horizontal is cut one cell either
    /// side, where it is a plain run.
    fn bridge(&mut self, graph: &ViewGraph, routes: &[Route]) {
        let ends = |e: EdgeId| {
            let e = &graph.edges[e.0 as usize];
            (e.from, e.to)
        };
        let colour = |e: EdgeId| {
            routes
                .get(e.0 as usize)
                .filter(|r| r.edge == e)
                .or_else(|| routes.iter().find(|r| r.edge == e))
                .and_then(|r| r.colour)
        };
        for i in 0..self.cells.len() {
            let runs = &self.runs[i];
            let Some(&(h, _)) = runs.iter().find(|(_, b)| *b == L | R) else {
                continue;
            };
            let Some(&(v, _)) = runs.iter().find(|(_, b)| *b == U | D) else {
                continue;
            };
            let (hs, ht) = ends(h);
            let (vs, vt) = ends(v);
            // Unrelated lines are never one line; related ones are only when
            // they carry the same colour.
            let related = hs == vs || ht == vt;
            if related && colour(h) == colour(v) {
                continue;
            }
            // A bridge is only legible if the horizontal is broken beside
            // it. Where neither neighbour is a plain horizontal to break, an
            // edge's own end or corner sits there, and the vertical drawn
            // over the cell would read as the horizontal stopping for no
            // reason; the junction is the honest glyph.
            let x = i % self.w;
            let left = x > 0 && self.cells[i - 1] == L | R;
            let right = x + 1 < self.w && self.cells[i + 1] == L | R;
            if !left && !right {
                continue;
            }
            self.over[i] = Some(('│', self.turn[i].0));
            if left {
                self.over[i - 1] = Some(('╴', self.ink[i - 1]));
            }
            if right {
                self.over[i + 1] = Some(('╶', self.ink[i + 1]));
            }
        }
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

    fn lines(&self, palette: Palette) -> Vec<Line<'static>> {
        let paint = |ink: Ink| match ink {
            Ink::One(c) => palette.edge(c),
            Ink::None | Ink::Mixed => style::dim(),
        };
        (0..self.h)
            .map(|y| {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut run = String::new();
                let mut run_style = style::dim();
                for x in 0..self.w {
                    let i = y * self.w + x;
                    let (ch, st) = if let Some(ink) = self.heads[i] {
                        ('▶', paint(ink))
                    } else if let Some((ch, ink)) = self.over[i] {
                        (ch, paint(ink))
                    } else {
                        let ink = match (self.ink[i], self.turn[i].0) {
                            (Ink::Mixed, Ink::One(c)) => Ink::One(c),
                            (ink, _) => ink,
                        };
                        (Self::glyph(self.cells[i]), paint(ink))
                    };
                    if st != run_style && !run.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut run), run_style));
                    }
                    run_style = st;
                    run.push(ch);
                }
                spans.push(Span::styled(run, run_style));
                Line::from(spans)
            })
            .collect()
    }
}

/// Draw cards and edges with column `scroll` at the left edge. `selected`
/// highlights the card holding one vertex.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    view: &PipelineView,
    cards: &CardView,
    scroll: usize,
    selected: Option<&str>,
    palette: Palette,
) {
    if cards.layout.cards.is_empty() || area.height == 0 {
        return;
    }
    let lay = &cards.layout;
    let dx = lay.col_x.get(scroll).copied().unwrap_or(0);
    let mut canvas = Canvas::new(area.width, area.height, dx);
    for r in &lay.routes {
        canvas.path(r.edge, &r.polyline, r.colour);
        canvas.head(r.head.0, r.head.1, r.colour);
    }
    if palette.crossing() == CrossingStyle::Bridge {
        canvas.bridge(&cards.graph, &cards.layout.routes);
    }
    frame.render_widget(Paragraph::new(canvas.lines(palette)), area);

    let selected_node = selected
        .and_then(|s| nfctl_core::model::VertexName::new(s).ok())
        .and_then(|n| cards.graph.node_of(&n));
    for c in &lay.cards {
        let x = c.x - dx;
        if x < 0 || c.y < 0 {
            continue;
        }
        let rect = Rect {
            x: area.x + u16::try_from(x).unwrap_or(u16::MAX),
            y: area.y + u16::try_from(c.y).unwrap_or(u16::MAX),
            width: cards.card_w,
            height: c.h,
        };
        if rect.right() > area.right() || rect.bottom() > area.bottom() {
            continue;
        }
        let node = &cards.graph.nodes[c.node.0 as usize];
        let badges = lay
            .badges
            .get(&c.node)
            .map(Vec::as_slice)
            .unwrap_or_default();
        card(
            frame,
            rect,
            node,
            &numbers(view, node),
            badges,
            selected_node == Some(c.node),
            palette,
        );
    }

    // Overflow markers.
    let hidden_left = scroll;
    let hidden_right = lay
        .col_x
        .len()
        .saturating_sub(cards.last_visible(scroll, area.width) + 1);
    if hidden_left > 0 {
        let s = format!("◀ {hidden_left} more");
        frame.render_widget(
            Paragraph::new(Line::styled(s, style::key())),
            Rect { height: 1, ..area },
        );
    }
    if hidden_right > 0 {
        let s = format!("{hidden_right} more ▶");
        let w = u16::try_from(s.width()).unwrap_or(0).min(area.width);
        let r = Rect {
            x: area.right() - w,
            y: area.y,
            width: w,
            height: 1,
        };
        frame.render_widget(Paragraph::new(Line::styled(s, style::key())), r);
    }
}

/// Rate and pending summed over a node's members.
fn numbers(view: &PipelineView, node: &ViewNode) -> (Option<f64>, Option<i64>) {
    let members: Vec<&VertexView> = view
        .vertices
        .iter()
        .filter(|v| node.members.contains(&v.name))
        .collect();
    let sum = |f: &dyn Fn(&VertexView) -> Option<f64>| {
        let vals: Vec<f64> = members.iter().filter_map(|v| f(v)).collect();
        (!vals.is_empty()).then(|| vals.iter().sum::<f64>())
    };
    let rate = sum(&|v| v.rate.m1);
    let pending: Vec<i64> = members
        .iter()
        .filter_map(|v| v.pending.default.or(v.pending.m1))
        .collect();
    (rate, (!pending.is_empty()).then(|| pending.iter().sum()))
}

fn fmt_rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |r| format!("{r:.1}/s"))
}

fn fmt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

fn card(
    frame: &mut Frame,
    area: Rect,
    node: &ViewNode,
    (rate, pending): &(Option<f64>, Option<i64>),
    badges: &[layout::Badge],
    selected: bool,
    palette: Palette,
) {
    let border = if selected { style::key() } else { style::dim() };
    let block = Block::default().borders(Borders::ALL).border_style(border);
    let inner = block.inner(area);
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);
    let head = Line::from(vec![
        Span::styled(node.label.clone(), style::title()),
        Span::styled(format!("  {}", node.kind.as_str()), style::dim()),
        if node.partitions > 1 {
            Span::styled(format!(" x{}", node.partitions), style::dim())
        } else {
            Span::raw("")
        },
    ]);
    let word = if inner.width >= FULL_CARD - 2 {
        "  pending "
    } else {
        " pend "
    };
    let nums = Line::from(vec![
        Span::raw(fmt_rate(*rate)),
        Span::styled(word, style::dim()),
        Span::raw(fmt_i64(*pending)),
    ]);
    let mut lines = vec![head, nums];
    if !badges.is_empty() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, b) in badges.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", Style::default()));
            }
            spans.push(Span::styled(b.label.clone(), palette.edge(b.colour)));
        }
        lines.push(Line::from(spans));
    }
    // A card is as tall as its busiest side needs, which can be taller than
    // its three lines of text; the text sits in the middle of the box rather
    // than against the top border.
    let pad = (usize::from(inner.height).saturating_sub(lines.len())) / 2;
    for _ in 0..pad {
        lines.insert(0, Line::default());
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
