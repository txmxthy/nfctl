//! The pipeline as columns of vertex cards, painted from an
//! [`orthodag::Drawing`]: edges through the gaps, long edges straight across,
//! back edges up out of the lanes under the cards. Shard groups draw as one
//! card; tagged edges take their combination's colour.

use std::collections::HashMap;
use std::ops::Range;

use nfctl_core::model::Topology;
use nfctl_core::service::{PipelineView, VertexView};
use nfctl_graph::{NodeId, ViewGraph, ViewNode};
use orthodag::{Colour, Crossing, Heading, Options};
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
/// However long the tags, a card stops growing here and wraps them instead.
const WIDEST_CARD: u16 = 34;
const GAP: u16 = 5;

/// One tag combination arriving at a card, in the colour its edge is drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub label: String,
    pub colour: Option<Colour>,
}

/// A laid-out pipeline: the view graph plus the drawing orthodag made of it.
pub struct CardView {
    pub graph: ViewGraph,
    pub drawing: orthodag::Drawing,
    /// Palette slot per view edge, in `graph.edges` order.
    pub colours: Vec<Option<Colour>>,
    pub badges: HashMap<NodeId, Vec<Badge>>,
    /// Leftmost cell of each column, in column order.
    pub col_x: Vec<i32>,
    pub card_w: u16,
}

impl CardView {
    #[must_use]
    pub fn new(t: &Topology, width: u16, expand_shards: bool) -> Self {
        let graph = if expand_shards {
            ViewGraph::expanded(t)
        } else {
            ViewGraph::collapsed(t)
        };
        // Card width: the widest of the label row, the numbers row and the
        // tags a card carries. Tags were left out and were the thing that got
        // cut. Cards shrink to `MIN_CARD` when the columns would not fit;
        // past that the panel scrolls instead of squeezing further.
        let want = graph
            .nodes
            .iter()
            .map(|n| n.label.width() + n.kind.as_str().len() + 6)
            .chain(tag_widths(&graph))
            .max()
            .unwrap_or(0);
        let want = u16::try_from(want)
            .unwrap_or(u16::MAX)
            .clamp(FULL_CARD, WIDEST_CARD);
        let n = u16::try_from(t.ranks().len().max(1)).unwrap_or(1);
        let fits = width.saturating_sub(GAP * (n - 1)) / n;
        let card_w = if fits >= want {
            want
        } else {
            fits.max(MIN_CARD)
        };

        // The card's text goes into the graph so orthodag's own height rule
        // sizes the box: a row for the numbers, then one for every row the
        // tags wrap onto. Nodes and edges go in in view order, so the
        // drawing's ids and `colour::of` both line up with `graph`.
        let labels = badge_labels(&graph);
        let inner = usize::from(card_w).saturating_sub(2);
        let mut og = orthodag::Graph::new();
        let ids: Vec<orthodag::NodeId> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let at = NodeId(u32::try_from(i).unwrap_or(u32::MAX));
                let mine: Vec<String> = labels
                    .get(&at)
                    .map(|v| v.iter().map(|(l, _)| l.clone()).collect())
                    .unwrap_or_default();
                let rows = 1 + badge_rows(&mine, inner).len();
                let mut node = orthodag::Node::new(&n.label);
                for _ in 0..rows {
                    node = node.line("");
                }
                og.add_node(node)
            })
            .collect();
        for e in &graph.edges {
            let (Some(&from), Some(&to)) = (ids.get(e.from.0 as usize), ids.get(e.to.0 as usize))
            else {
                continue;
            };
            let Ok(_) = og.add_tagged_edge(from, to, e.tags.iter().cloned()) else {
                continue;
            };
        }
        let colours = orthodag::colour::of(&og);
        let badges = labels
            .into_iter()
            .map(|(node, v)| {
                let held: Vec<Badge> = v
                    .into_iter()
                    .map(|(label, at)| Badge {
                        label,
                        colour: colours.get(at).copied().flatten(),
                    })
                    .collect();
                (node, held)
            })
            .collect();
        let drawing = orthodag::layout(
            &og,
            Options::new()
                .box_width(usize::from(card_w))
                .box_height(usize::from(CARD_HEIGHT))
                .crossings(Crossing::Cross),
        );
        let mut col_x: Vec<i32> = Vec::new();
        for b in drawing.boxes() {
            if col_x.len() <= b.column {
                col_x.resize(b.column + 1, i32::MAX);
            }
            col_x[b.column] = col_x[b.column].min(b.rect.x);
        }
        Self {
            graph,
            drawing,
            colours,
            badges,
            col_x,
            card_w,
        }
    }

    #[must_use]
    pub fn height(&self) -> u16 {
        u16::try_from(self.drawing.size().1).unwrap_or(u16::MAX)
    }

    /// The box drawing one node.
    #[must_use]
    pub fn card(&self, node: NodeId) -> Option<orthodag::Boxed> {
        self.drawing
            .boxes()
            .find(|b| b.node.index() == node.0 as usize)
    }

    /// Column of the card holding `vertex`, if any.
    #[must_use]
    pub fn column_of(&self, vertex: &str) -> Option<usize> {
        let name = nfctl_core::model::VertexName::new(vertex).ok()?;
        let node = self.graph.node_of(&name)?;
        self.card(node).map(|b| b.column)
    }

    /// First column whose right edge fits when column `first` is at x=0.
    #[must_use]
    pub fn last_visible(&self, first: usize, width: u16) -> usize {
        let x0 = self.col_x.get(first).copied().unwrap_or(0);
        self.col_x
            .iter()
            .rposition(|&x| x - x0 + i32::from(self.card_w) <= i32::from(width))
            .unwrap_or(first)
            .max(first)
    }
}

/// How wide each card's tag row wants to be, borders included. The layout
/// works these out too, but not until it has been given a card width, so they
/// are counted here from the same edges it counts them from.
fn tag_widths(graph: &ViewGraph) -> impl Iterator<Item = usize> {
    let mut per: std::collections::HashMap<usize, Vec<String>> = std::collections::HashMap::new();
    for e in &graph.edges {
        if e.tags.is_empty() {
            continue;
        }
        let label = e.tags.join(", ");
        let at = per.entry(e.to.0 as usize).or_default();
        if !at.contains(&label) {
            at.push(label);
        }
    }
    per.into_values()
        .map(|labels| labels.join(" ").width() + 2)
        .collect::<Vec<_>>()
        .into_iter()
}

/// The badges of one card, split into the rows they take at `width`. Whole
/// badges only: a tag is never broken across two lines. The painter draws
/// these rows and the card is given a text line per row, so a card is as tall
/// as its tags.
#[must_use]
pub fn badge_rows(labels: &[String], width: usize) -> Vec<Range<usize>> {
    if labels.is_empty() || width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let (mut start, mut used) = (0, 0usize);
    for (i, label) in labels.iter().enumerate() {
        let w = label.chars().count();
        let gap = usize::from(used > 0);
        if used > 0 && used + gap + w > width {
            rows.push(start..i);
            (start, used) = (i, w);
        } else {
            used += gap + w;
        }
    }
    rows.push(start..labels.len());
    rows
}

/// The tag combinations arriving at each node, in first-seen order, each with
/// the edge it was first seen on so it can take that edge's colour.
fn badge_labels(g: &ViewGraph) -> HashMap<NodeId, Vec<(String, usize)>> {
    let mut out: HashMap<NodeId, Vec<(String, usize)>> = HashMap::new();
    for (i, e) in g.edges.iter().enumerate() {
        if e.tags.is_empty() {
            continue;
        }
        let label = e.tags.join(", ");
        let at = out.entry(e.to).or_default();
        if !at.iter().any(|(held, _)| *held == label) {
            at.push((label, i));
        }
    }
    out
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
    One(Option<Colour>),
    Mixed,
}

struct Canvas {
    w: usize,
    h: usize,
    dx: i32,
    dy: i32,
    cells: Vec<u8>,
    ink: Vec<Ink>,
    /// Colour of the longest vertical run through a cell, with its length:
    /// where a bus mixes colours, the branch that reaches furthest wins, so a
    /// trunk reads as one colour from the source to its far end.
    turn: Vec<(Ink, i32)>,
    /// Arrowhead ink, and which way the head points.
    heads: Vec<Option<(Ink, Heading)>>,
    /// Which edge left which bits in a cell, by its index in the view graph,
    /// for telling a crossing from a junction after the fact.
    runs: Vec<Vec<(usize, u8)>>,
    /// Glyph and ink painted instead of the cell's bits: the bridge style.
    over: Vec<Option<(char, Ink)>>,
    edge: usize,
}

impl Canvas {
    fn new(w: u16, h: u16, dx: i32, dy: i32) -> Self {
        let (w, h) = (usize::from(w), usize::from(h));
        Self {
            w,
            h,
            dx,
            dy,
            cells: vec![0; w * h],
            ink: vec![Ink::None; w * h],
            turn: vec![(Ink::None, 0); w * h],
            heads: vec![None; w * h],
            runs: vec![Vec::new(); w * h],
            over: vec![None; w * h],
            edge: 0,
        }
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (
            usize::try_from(x - self.dx).ok()?,
            usize::try_from(y - self.dy).ok()?,
        );
        (x < self.w && y < self.h).then_some(y * self.w + x)
    }

    fn set(&mut self, x: i32, y: i32, bits: u8, colour: Option<Colour>, reach: i32) {
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

    fn head(&mut self, x: i32, y: i32, colour: Option<Colour>, heading: Heading) {
        if let Some(i) = self.idx(x, y) {
            self.heads[i] = Some(match self.heads[i] {
                None => (Ink::One(colour), heading),
                Some((Ink::One(c), h)) if c == colour => (Ink::One(c), h),
                Some((_, h)) => (Ink::Mixed, h),
            });
        }
    }

    fn hline(&mut self, y: i32, x0: i32, x1: i32, colour: Option<Colour>) {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let bits = if x > a { L } else { 0 } | if x < b { R } else { 0 };
            self.set(x, y, bits, colour, 0);
        }
    }

    fn vline(&mut self, x: i32, y0: i32, y1: i32, colour: Option<Colour>) {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let bits = if y > a { U } else { 0 } | if y < b { D } else { 0 };
            self.set(x, y, bits, colour, b - a + 1);
        }
    }

    /// Orthogonal path through the given corner points.
    fn path(&mut self, edge: usize, pts: &[(i32, i32)], colour: Option<Colour>) {
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
    fn bridge(&mut self, graph: &ViewGraph, colours: &[Option<Colour>]) {
        let ends = |e: usize| graph.edges.get(e).map(|e| (e.from, e.to));
        let colour = |e: usize| colours.get(e).copied().flatten();
        for i in 0..self.cells.len() {
            let runs = &self.runs[i];
            let Some(&(h, _)) = runs.iter().find(|(_, b)| *b == L | R) else {
                continue;
            };
            let Some(&(v, _)) = runs.iter().find(|(_, b)| *b == U | D) else {
                continue;
            };
            let (Some((hs, ht)), Some((vs, vt))) = (ends(h), ends(v)) else {
                continue;
            };
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
                    let (ch, st) = if let Some((ink, heading)) = self.heads[i] {
                        let ch = match heading {
                            Heading::Right => '▶',
                            Heading::Up => '▲',
                        };
                        (ch, paint(ink))
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

/// How the card view is drawn right now: where it is scrolled to, in what
/// colours, and how far through the animation anything that moves is.
#[derive(Debug, Clone, Copy, Default)]
pub struct Paint {
    pub scroll: Scroll,
    pub palette: Palette,
    /// Frame counter; a ticker moves on it.
    pub spin: usize,
}

/// Where a drawing bigger than its box is scrolled to.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scroll {
    /// Leftmost visible card column.
    pub column: usize,
    /// Topmost visible row, in cells.
    pub row: i32,
}

/// Draw cards and edges as `at` says. `selected` highlights the card holding
/// one vertex.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    view: &PipelineView,
    cards: &CardView,
    at: Paint,
    selected: Option<&str>,
) {
    if cards.drawing.boxes().next().is_none() || area.height == 0 {
        return;
    }
    let (scroll, palette) = (at.scroll, at.palette);
    let (dx, dy) = (
        cards.col_x.get(scroll.column).copied().unwrap_or(0),
        scroll.row,
    );
    let mut canvas = Canvas::new(area.width, area.height, dx, dy);
    for r in cards.drawing.routes() {
        let colour = cards.colours.get(r.edge.index()).copied().flatten();
        canvas.path(r.edge.index(), r.points, colour);
        if let Some(&(x, y)) = r.points.last() {
            canvas.head(x, y, colour, r.heading);
        }
    }
    if palette.crossing() == CrossingStyle::Bridge {
        canvas.bridge(&cards.graph, &cards.colours);
    }
    frame.render_widget(Paragraph::new(canvas.lines(palette)), area);

    let selected_node = selected
        .and_then(|s| nfctl_core::model::VertexName::new(s).ok())
        .and_then(|n| cards.graph.node_of(&n));
    for b in cards.drawing.boxes() {
        let (x, y) = (b.rect.x - dx, b.rect.y - dy);
        if x < 0 || y < 0 {
            continue;
        }
        let rect = Rect {
            x: area.x + u16::try_from(x).unwrap_or(u16::MAX),
            y: area.y + u16::try_from(y).unwrap_or(u16::MAX),
            width: cards.card_w,
            height: u16::try_from(b.rect.h).unwrap_or(0),
        };
        if rect.right() > area.right() || rect.bottom() > area.bottom() {
            continue;
        }
        let Some(node) = cards.graph.nodes.get(b.node.index()) else {
            continue;
        };
        let id = NodeId(u32::try_from(b.node.index()).unwrap_or(u32::MAX));
        let badges = cards.badges.get(&id).map(Vec::as_slice).unwrap_or_default();
        card(
            frame,
            rect,
            node,
            &numbers(view, node),
            badges,
            selected_node == Some(id),
            at,
        );
    }

    // Overflow markers.
    let hidden_left = scroll.column;
    let hidden_right = cards
        .col_x
        .len()
        .saturating_sub(cards.last_visible(scroll.column, area.width) + 1);
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

/// Columns a ticker holds a line still for before moving it on. At the frame
/// rate this is about a third of a second a column, which is readable.
const TICK_HOLD: usize = 3;
/// Blank columns between the end of a ticker's text and its start coming round.
const TICK_GAP: usize = 4;

/// A name wider than the room it has, moved along a column at a time so all
/// of it can be read. Only the name: tags wrap onto another row instead,
/// which is quieter to read when several cards have them.
fn ticker(spans: Vec<Span<'static>>, width: usize, frame: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| s.content.width()).sum();
    if total <= width || width == 0 {
        return spans;
    }
    // One long line of (character, style), plus a gap, read from an offset
    // that walks it and comes round.
    let cells: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .chain((0..TICK_GAP).map(|_| (' ', Style::default())))
        .collect();
    let at = (frame / TICK_HOLD) % cells.len();
    let mut out: Vec<Span<'static>> = Vec::new();
    for i in 0..width {
        let (c, style) = cells[(at + i) % cells.len()];
        match out.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(c),
            _ => out.push(Span::styled(c.to_string(), style)),
        }
    }
    out
}

fn card(
    frame: &mut Frame,
    area: Rect,
    node: &ViewNode,
    (rate, pending): &(Option<f64>, Option<i64>),
    badges: &[Badge],
    selected: bool,
    at: Paint,
) {
    let (palette, spin) = (at.palette, at.spin);
    let border = if selected { style::key() } else { style::dim() };
    let block = Block::default().borders(Borders::ALL).border_style(border);
    let inner = block.inner(area);
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);
    let head = Line::from(ticker(
        vec![
            Span::styled(node.label.clone(), style::title()),
            Span::styled(format!("  {}", node.kind.as_str()), style::dim()),
            if node.partitions > 1 {
                Span::styled(format!(" x{}", node.partitions), style::dim())
            } else {
                Span::raw("")
            },
        ],
        usize::from(inner.width),
        spin,
    ));
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
    // Tags wrap onto as many rows as they take, and the card was made tall
    // enough for them; scrolling them was worse to read than a second row.
    let labels: Vec<String> = badges.iter().map(|b| b.label.clone()).collect();
    for row in badge_rows(&labels, usize::from(inner.width)) {
        let mut tags: Vec<Span<'static>> = Vec::new();
        for i in row {
            if !tags.is_empty() {
                tags.push(Span::raw(" "));
            }
            tags.push(Span::styled(
                badges[i].label.clone(),
                palette.edge(badges[i].colour),
            ));
        }
        lines.push(Line::from(tags));
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
