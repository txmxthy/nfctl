//! Coordinates: card and pass rows per column, gap widths from track counts,
//! one polyline per edge, lanes for back edges.

use std::collections::HashMap;

use super::order::{LNode, Layered};
use super::rank::Ranked;
use super::tracks::{Span, pack};
use super::{
    Badge, CardPos, EdgeColour, EdgeId, GapPlan, Layout, LayoutOptions, MIN_GAP, NodeId, Route,
    Slot, Track, ViewGraph, i,
};

/// Left margin when a back edge targets column 0.
const BACK_MARGIN: i32 = 3;
/// Pseudo node keys so back-edge verticals never share a track with forward
/// runs: a lane vertical passing through a join would read as part of it.
const LANE_OUT: u32 = u32::MAX - 1;
const LANE_IN: u32 = u32::MAX - 2;

fn edge_id(index: usize) -> EdgeId {
    EdgeId(u32::try_from(index).unwrap_or(u32::MAX))
}

fn key(n: LNode) -> u32 {
    match n {
        LNode::Real(id) => id.0,
        LNode::Pass(e) => 0x8000_0000 | e.0,
    }
}

/// Column and attach row per layered node, plus the card and slot lists.
struct Geometry {
    attach: HashMap<LNode, (usize, i32)>,
    cards: Vec<CardPos>,
    columns: Vec<Vec<Slot>>,
    cards_h: i32,
}

fn geometry(l: &Layered, opts: LayoutOptions) -> Geometry {
    let slot_h = |n: &LNode| match n {
        LNode::Real(_) => i32::from(opts.card_h),
        LNode::Pass(_) => 1,
    };
    let heights: Vec<i32> = l
        .columns
        .iter()
        .map(|col| col.iter().map(slot_h).sum())
        .collect();
    let tallest = heights.iter().copied().max().unwrap_or(0);
    let mut geo = Geometry {
        attach: HashMap::new(),
        cards: Vec::new(),
        columns: Vec::new(),
        cards_h: tallest,
    };
    for (c, col) in l.columns.iter().enumerate() {
        // Shorter columns sit centred on the tallest one, so a chain that fans
        // out and back in reads as one horizontal line through the middle.
        let mut y = (tallest - heights[c]) / 2;
        let mut slots = Vec::new();
        for &n in col {
            match n {
                LNode::Real(id) => {
                    // Every card is the same (odd) height so edges meet a true
                    // middle row and siblings line up; the badge row is blank
                    // when nothing tagged arrives.
                    let h = opts.card_h;
                    // Edges meet the card on its middle row.
                    geo.attach.insert(n, (c, y + i32::from(h) / 2));
                    geo.cards.push(CardPos {
                        node: id,
                        col: c,
                        x: 0,
                        y,
                        h,
                    });
                    slots.push(Slot::Card(id));
                    y += i32::from(h);
                }
                LNode::Pass(e) => {
                    geo.attach.insert(n, (c, y));
                    slots.push(Slot::Pass(e));
                    y += 1;
                }
            }
        }
        geo.columns.push(slots);
    }
    geo
}

/// One lane row per back-edge source; shortest hops nearest the cards.
fn lanes(g: &ViewGraph, r: &Ranked, cards_h: i32) -> HashMap<NodeId, i32> {
    let mut sources: Vec<(usize, NodeId)> = g
        .edges
        .iter()
        .enumerate()
        .filter(|(ei, _)| r.back[*ei])
        .map(|(_, e)| {
            (
                r.rank[e.from.0 as usize].abs_diff(r.rank[e.to.0 as usize]),
                e.from,
            )
        })
        .collect();
    sources.sort_unstable();
    sources.dedup_by_key(|x| x.1);
    sources
        .into_iter()
        .enumerate()
        .map(|(k, (_, src))| (src, cards_h + i(k)))
        .collect()
}

/// `(gap, index into that gap's spans)`.
type SpanAt = (usize, usize);

/// Vertical runs per gap, and the (gap, index) of each run keyed by edge and role.
struct Spans {
    per_gap: Vec<Vec<Span>>,
    /// Forward segments in `Layered::segments` order.
    forward: Vec<SpanAt>,
    /// Back edges: (edge index, span leaving the source, span entering the target).
    back: HashMap<usize, (Option<SpanAt>, Option<SpanAt>)>,
}

fn spans(
    g: &ViewGraph,
    r: &Ranked,
    l: &Layered,
    geo: &Geometry,
    lane: &HashMap<NodeId, i32>,
    colour: &[Option<EdgeColour>],
) -> Spans {
    let cols = l.columns.len();
    let mut per_gap: Vec<Vec<Span>> = vec![Vec::new(); cols.saturating_sub(1)];
    let mut push = |gap: usize, s: Span| {
        per_gap[gap].push(s);
        (gap, per_gap[gap].len() - 1)
    };
    let forward = l
        .segments
        .iter()
        .map(|s| {
            let (_, y0) = geo.attach[&s.from];
            let (_, y1) = geo.attach[&s.to];
            push(
                s.col,
                Span {
                    edge: s.edge,
                    lo: y0.min(y1),
                    hi: y0.max(y1),
                    y_in: y0,
                    y_out: y1,
                    colour: colour[s.edge.0 as usize],
                    src: key(s.from),
                    dst: key(s.to),
                },
            )
        })
        .collect();
    let mut back = HashMap::new();
    for (ei, e) in g.edges.iter().enumerate() {
        if !r.back[ei] {
            continue;
        }
        let (cu, yu) = geo.attach[&LNode::Real(e.from)];
        let (cv, yv) = geo.attach[&LNode::Real(e.to)];
        let ly = lane[&e.from];
        let out = (cu + 1 < cols).then(|| {
            push(
                cu,
                Span {
                    edge: edge_id(ei),
                    lo: yu,
                    hi: ly,
                    y_in: yu,
                    y_out: ly,
                    colour: colour[ei],
                    src: LANE_OUT,
                    dst: e.from.0,
                },
            )
        });
        let inn = (cv >= 1).then(|| {
            push(
                cv - 1,
                Span {
                    edge: edge_id(ei),
                    lo: yv,
                    hi: ly,
                    y_in: ly,
                    y_out: yv,
                    colour: colour[ei],
                    src: e.to.0,
                    dst: LANE_IN,
                },
            )
        });
        back.insert(ei, (out, inn));
    }
    Spans {
        per_gap,
        forward,
        back,
    }
}

struct Columns {
    col_x: Vec<i32>,
    gaps: Vec<GapPlan>,
    /// Track index per span, per gap.
    assign: Vec<Vec<usize>>,
    width: i32,
}

fn columns(sp: &Spans, cols: usize, card_w: i32, margin: i32) -> Columns {
    let mut out = Columns {
        col_x: vec![0; cols],
        gaps: Vec::new(),
        assign: Vec::new(),
        width: margin,
    };
    for c in 0..cols {
        out.col_x[c] = out.width;
        out.width += card_w;
        if c + 1 < cols {
            let (assign, n) = pack(&sp.per_gap[c]);
            let width = u16::try_from(n)
                .unwrap_or(u16::MAX)
                .saturating_add(2)
                .max(MIN_GAP);
            let pad = (i32::from(width) - 2 - i(n)) / 2;
            let x0 = out.width;
            let tracks = (0..n)
                .map(|t| Track {
                    x: x0 + 1 + pad + i(t),
                    edges: sp.per_gap[c]
                        .iter()
                        .zip(&assign)
                        .filter(|(_, a)| **a == t)
                        .map(|(s, _)| s.edge)
                        .collect(),
                })
                .collect();
            out.gaps.push(GapPlan { x0, width, tracks });
            out.assign.push(assign);
            out.width += i32::from(width);
        }
    }
    out
}

impl Columns {
    fn track_x(&self, at: SpanAt) -> i32 {
        self.gaps[at.0].tracks[self.assign[at.0][at.1]].x
    }
}

fn forward_route(
    l: &Layered,
    geo: &Geometry,
    cx: &Columns,
    sp: &Spans,
    card_w: i32,
    id: EdgeId,
) -> Vec<(i32, i32)> {
    let mut segs: Vec<usize> = (0..l.segments.len())
        .filter(|&si| l.segments[si].edge == id)
        .collect();
    segs.sort_by_key(|&si| l.segments[si].col);
    let mut pts = Vec::new();
    for (k, &si) in segs.iter().enumerate() {
        let s = &l.segments[si];
        let (c, y0) = geo.attach[&s.from];
        let (_, y1) = geo.attach[&s.to];
        let tx = cx.track_x(sp.forward[si]);
        if k == 0 {
            pts.push((cx.col_x[c] + card_w, y0));
        }
        pts.extend([(tx, y0), (tx, y1), (cx.col_x[c + 1] - 1, y1)]);
        if k + 1 < segs.len() {
            pts.push((cx.col_x[c + 1] + card_w, y1));
        }
    }
    pts
}

fn back_route(
    g: &ViewGraph,
    geo: &Geometry,
    cx: &Columns,
    sp: &Spans,
    lane: &HashMap<NodeId, i32>,
    card_w: i32,
    ei: usize,
) -> Vec<(i32, i32)> {
    let e = &g.edges[ei];
    let (cu, yu) = geo.attach[&LNode::Real(e.from)];
    let (cv, yv) = geo.attach[&LNode::Real(e.to)];
    let ly = lane[&e.from];
    let (out, inn) = sp.back.get(&ei).copied().unwrap_or((None, None));
    let x_out = cx.col_x[cu] + card_w;
    let xd = out.map_or(x_out + 1, |at| cx.track_x(at));
    let xu = inn.map_or(BACK_MARGIN / 2, |at| cx.track_x(at));
    vec![
        (x_out, yu),
        (xd, yu),
        (xd, ly),
        (xu, ly),
        (xu, yv),
        (cx.col_x[cv] - 1, yv),
    ]
}

fn collapse(pts: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for p in pts {
        if out.last() == Some(&p) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            if (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1) {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

pub(crate) fn build(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
) -> Layout {
    let cols = layered.columns.len();
    let card_w = i32::from(opts.card_w);
    let geo = geometry(layered, opts);
    let lane = lanes(g, ranked, geo.cards_h);
    let sp = spans(g, ranked, layered, &geo, &lane, edge_colour);
    let margin = if g
        .edges
        .iter()
        .enumerate()
        .any(|(ei, e)| ranked.back[ei] && ranked.rank[e.to.0 as usize] == 0)
    {
        BACK_MARGIN
    } else {
        0
    };
    let cx = columns(&sp, cols, card_w, margin);
    let routes: Vec<Route> = (0..g.edges.len())
        .map(|ei| {
            let pts = if ranked.back[ei] {
                back_route(g, &geo, &cx, &sp, &lane, card_w, ei)
            } else {
                forward_route(layered, &geo, &cx, &sp, card_w, edge_id(ei))
            };
            let polyline = collapse(pts);
            let head = polyline.last().copied().unwrap_or_default();
            Route {
                edge: edge_id(ei),
                polyline,
                head,
                colour: edge_colour[ei],
            }
        })
        .collect();
    let mut cards = geo.cards;
    for card in &mut cards {
        card.x = cx.col_x[card.col];
    }
    let lanes = u16::try_from(lane.len()).unwrap_or(u16::MAX);
    Layout {
        columns: geo.columns,
        col_x: cx.col_x,
        cards,
        gaps: cx.gaps,
        routes,
        lanes,
        badges: badges.clone(),
        width: u16::try_from(cx.width).unwrap_or(u16::MAX),
        height: u16::try_from(geo.cards_h)
            .unwrap_or(u16::MAX)
            .saturating_add(lanes),
    }
}
