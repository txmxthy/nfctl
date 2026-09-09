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

/// Blank rows between stacked slots in a column.
const SLOT_GAP: i32 = 1;
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

fn geometry(g: &ViewGraph, l: &Layered, opts: LayoutOptions) -> Geometry {
    let card_h = i32::from(opts.card_h);
    // Cards only: pass slots are placed afterwards, on a row the edge already
    // travels on wherever that row is free. One blank row between stacked
    // cards keeps every column an odd height, so centring lands on a row and
    // a source sits exactly between the two targets it fans out to.
    let cards_in = |col: &Vec<LNode>| col.iter().filter(|n| matches!(n, LNode::Real(_))).count();
    let heights: Vec<i32> = l
        .columns
        .iter()
        .map(|col| {
            let n = cards_in(col);
            i(n) * card_h + i(n.saturating_sub(1)) * SLOT_GAP
        })
        .collect();
    let tallest = heights.iter().copied().max().unwrap_or(0);
    let mut geo = Geometry {
        attach: HashMap::new(),
        cards: Vec::new(),
        columns: Vec::new(),
        cards_h: tallest,
    };
    for (c, col) in l.columns.iter().enumerate() {
        let mut y = (tallest - heights[c]) / 2;
        let mut slots = Vec::new();
        let mut placed = 0;
        for &n in col {
            let LNode::Real(id) = n else {
                slots.push(match n {
                    LNode::Pass(e) => Slot::Pass(e),
                    LNode::Real(id) => Slot::Card(id),
                });
                continue;
            };
            if placed > 0 {
                y += SLOT_GAP;
            }
            placed += 1;
            geo.attach.insert(n, (c, y + card_h / 2));
            geo.cards.push(CardPos {
                node: id,
                col: c,
                x: 0,
                y,
                h: opts.card_h,
            });
            slots.push(Slot::Card(id));
            y += card_h;
        }
        geo.columns.push(slots);
    }
    pass_rows(g, l, &mut geo, card_h);
    geo
}

/// Give every pass slot a row. First choice: the row the edge leaves its
/// source on, so a long edge runs straight; then the row it enters its
/// target on; then the free row nearest the source row. A row is free in a
/// column when no card there covers it or the row beside it, and no other
/// pass in that column has it.
fn pass_rows(g: &ViewGraph, l: &Layered, geo: &mut Geometry, card_h: i32) {
    let blocked = |geo: &Geometry, c: usize, row: i32| {
        geo.cards
            .iter()
            .filter(|k| k.col == c)
            .any(|k| row >= k.y - 1 && row <= k.y + card_h)
            || geo
                .attach
                .iter()
                .any(|(n, &(pc, py))| matches!(n, LNode::Pass(_)) && pc == c && py == row)
    };
    let mut passes: Vec<(usize, EdgeId)> = l
        .columns
        .iter()
        .enumerate()
        .flat_map(|(c, col)| {
            col.iter().filter_map(move |n| match n {
                LNode::Pass(e) => Some((c, *e)),
                LNode::Real(_) => None,
            })
        })
        .collect();
    // Longer edges first: they have the least freedom.
    passes.sort_by_key(|&(c, e)| {
        let edge = &g.edges[e.0 as usize];
        let (sc, _) = geo.attach[&LNode::Real(edge.from)];
        let (tc, _) = geo.attach[&LNode::Real(edge.to)];
        (std::cmp::Reverse(tc - sc), c, e)
    });
    for (c, e) in passes {
        let edge = &g.edges[e.0 as usize];
        let (_, src_row) = geo.attach[&LNode::Real(edge.from)];
        let (_, dst_row) = geo.attach[&LNode::Real(edge.to)];
        let mut row = None;
        for candidate in [src_row, dst_row] {
            if !blocked(geo, c, candidate) {
                row = Some(candidate);
                break;
            }
        }
        let row = row.unwrap_or_else(|| {
            // Every row past the tallest column is free, so the search ends.
            let limit = geo.cards_h + 2;
            (1..=limit + src_row)
                .flat_map(|d| [src_row - d, src_row + d])
                .find(|&r| r >= 0 && !blocked(geo, c, r))
                .unwrap_or(src_row)
        });
        geo.attach.insert(LNode::Pass(e), (c, row));
        geo.cards_h = geo.cards_h.max(row + 1);
    }
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
                        .filter(|(s, a)| **a == t && s.lo != s.hi)
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
        if k == 0 {
            pts.push((cx.col_x[c] + card_w, y0));
        }
        if y0 == y1 {
            // Straight across: no track needed.
            pts.push((cx.col_x[c + 1] - 1, y1));
        } else {
            let tx = cx.track_x(sp.forward[si]);
            pts.extend([(tx, y0), (tx, y1), (cx.col_x[c + 1] - 1, y1)]);
        }
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
    let geo = geometry(g, layered, opts);
    let lane = lanes(g, ranked, geo.cards_h);
    let sp = spans(g, ranked, layered, &geo, &lane);
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
