//! Coordinates: card and pass rows per column, gap widths from track counts,
//! one polyline per edge, lanes for back edges.

use std::collections::HashMap;

use super::order::{LNode, Layered, cards};
use super::rank::Ranked;
use super::score::{Score, score};
use super::tracks::{Span, pack};
use super::{
    Badge, CardPos, EdgeColour, EdgeId, GapPlan, Layout, LayoutOptions, MIN_GAP, NodeId, Route,
    Slot, Track, ViewGraph, i,
};

/// Blank rows between stacked slots in a column.
const SLOT_GAP: i32 = 1;
/// Placement sweeps tried after the centred layout: left-to-right, then
/// right-to-left, alternating.
const SWEEPS: usize = 4;
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

/// Card top rows per column, in column order.
type Placement = Vec<Vec<i32>>;

fn tallest(cards: &[Vec<NodeId>], card_h: i32) -> i32 {
    cards
        .iter()
        .map(|c| i(c.len()) * card_h + i(c.len().saturating_sub(1)) * SLOT_GAP)
        .max()
        .unwrap_or(0)
}

fn geometry(g: &ViewGraph, l: &Layered, ys: &Placement, opts: LayoutOptions) -> Geometry {
    let card_h = i32::from(opts.card_h);
    // Cards only: pass slots are placed afterwards, on a row the edge already
    // travels on wherever that row is free. One blank row between stacked
    // cards keeps every column an odd height, so a card placed on the median
    // of its neighbours sits exactly between two of them.
    let mut geo = Geometry {
        attach: HashMap::new(),
        cards: Vec::new(),
        columns: Vec::new(),
        cards_h: ys
            .iter()
            .flatten()
            .map(|&y| y + card_h)
            .max()
            .unwrap_or(0)
            .max(tallest(&cards(&l.columns), card_h)),
    };
    for (c, col) in l.columns.iter().enumerate() {
        let mut placed = 0;
        let slots = col
            .iter()
            .map(|&n| match n {
                LNode::Pass(e) => Slot::Pass(e),
                LNode::Real(id) => {
                    let y = ys[c][placed];
                    placed += 1;
                    geo.attach.insert(n, (c, y + card_h / 2));
                    geo.cards.push(CardPos {
                        node: id,
                        col: c,
                        x: 0,
                        y,
                        h: opts.card_h,
                    });
                    Slot::Card(id)
                }
            })
            .collect();
        geo.columns.push(slots);
    }
    pass_rows(g, l, &mut geo, card_h);
    geo
}

/// Candidate placements, the plain one first: every column centred on the
/// tallest; then cards placed by their neighbours. A left-to-right sweep puts
/// each card on the median row of its sources in the previous column, a
/// right-to-left sweep on the median of its targets in the next column; a
/// long edge counts as its real endpoint. Within a column cards stack in
/// order, the column then shifts as a whole by the median of what its cards
/// still want, and is pressed into the tallest column's height so the layout
/// never grows. `build` scores each candidate on the drawn geometry and keeps
/// the best.
fn placements(g: &ViewGraph, l: &Layered, card_h: i32) -> (Placement, Vec<Placement>) {
    let cards = cards(&l.columns);
    let tallest = tallest(&cards, card_h);
    let cols = cards.len();
    let mid = |y: i32| y + card_h / 2;
    // (column, index in column) per card.
    let mut at: HashMap<NodeId, (usize, usize)> = HashMap::new();
    for (c, col) in cards.iter().enumerate() {
        for (k, &id) in col.iter().enumerate() {
            at.insert(id, (c, k));
        }
    }
    // Forward edges as `(from, to)`; a long edge pulls on its real endpoints.
    let mut hops: Vec<(NodeId, NodeId)> = l
        .segments
        .iter()
        .map(|s| {
            (
                g.edges[s.edge.0 as usize].from,
                g.edges[s.edge.0 as usize].to,
            )
        })
        .collect();
    hops.dedup();
    let mut ys = centred(&cards, card_h);
    let plain = ys.clone();
    let mut out = Vec::new();
    let row = |ys: &Placement, id: NodeId| {
        let (c, k) = at[&id];
        mid(ys[c][k])
    };
    let sweep = |ys: &mut Placement, c: usize, down: bool| {
        let desired: Vec<Option<i32>> = cards[c]
            .iter()
            .map(|&id| {
                let mut rows: Vec<i32> = hops
                    .iter()
                    .filter(|&&(from, to)| if down { to == id } else { from == id })
                    .map(|&(from, to)| row(ys, if down { from } else { to }))
                    .collect();
                rows.sort_unstable();
                median(&rows)
            })
            .collect();
        let col = &mut ys[c];
        let mut bottom = -SLOT_GAP;
        for (k, want) in desired.iter().enumerate() {
            col[k] = want.map_or(bottom + SLOT_GAP, |w| {
                (w - card_h / 2).max(bottom + SLOT_GAP)
            });
            bottom = col[k] + card_h;
        }
        let mut residual: Vec<i32> = desired
            .iter()
            .zip(col.iter())
            .filter_map(|(w, &y)| w.map(|w| w - mid(y)))
            .collect();
        residual.sort_unstable();
        let shift = median(&residual).unwrap_or((tallest - bottom) / 2);
        for y in col.iter_mut() {
            *y += shift;
        }
        fit(col, card_h, tallest);
    };
    for pass in 0..SWEEPS {
        if pass % 2 == 0 {
            for c in 0..cols {
                sweep(&mut ys, c, true);
            }
        } else {
            for c in (0..cols.saturating_sub(1)).rev() {
                sweep(&mut ys, c, false);
            }
        }
        if ys != plain && !out.contains(&ys) {
            out.push(ys.clone());
        }
    }
    (plain, out)
}

/// Every column centred on the tallest.
fn centred(cards: &[Vec<NodeId>], card_h: i32) -> Placement {
    let tallest = tallest(cards, card_h);
    cards
        .iter()
        .map(|col| {
            let height = i(col.len()) * card_h + i(col.len().saturating_sub(1)) * SLOT_GAP;
            (0..col.len())
                .map(|k| (tallest - height) / 2 + i(k) * (card_h + SLOT_GAP))
                .collect()
        })
        .collect()
}

/// Middle value of a sorted list; the mean of the two middle ones when even.
fn median(sorted: &[i32]) -> Option<i32> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    Some(if n % 2 == 1 {
        sorted[n / 2]
    } else {
        sorted[n / 2 - 1].midpoint(sorted[n / 2])
    })
}

/// Press a stacked column into rows `0..height`, keeping order and moving
/// each card as little as possible.
fn fit(col: &mut [i32], card_h: i32, height: i32) {
    let step = card_h + SLOT_GAP;
    let n = col.len();
    for k in (0..n).rev() {
        let limit = if k + 1 < n {
            col[k + 1] - step
        } else {
            height - card_h
        };
        col[k] = col[k].min(limit);
    }
    for k in 0..n {
        let limit = if k > 0 { col[k - 1] + step } else { 0 };
        col[k] = col[k].max(limit);
    }
}

/// Give every pass slot a row, one row per edge across all the columns it
/// passes so a long edge runs straight. The row wanted: the target's row
/// when the edge leaves a fork for a target with one in-edge, the source's
/// row when it leaves a card with one out-edge for a join, else the source's
/// row. The row must be free in every pass column, otherwise the free row
/// nearest the wanted one. A row is free in a column when no card there
/// covers it or the row beside it, and no other pass in that column has it.
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
    // Forward degrees: every forward edge has one segment out of its real
    // source and one into its real target.
    let mut out_deg: HashMap<NodeId, usize> = HashMap::new();
    let mut in_deg: HashMap<NodeId, usize> = HashMap::new();
    for s in &l.segments {
        if let LNode::Real(id) = s.from {
            *out_deg.entry(id).or_default() += 1;
        }
        if let LNode::Real(id) = s.to {
            *in_deg.entry(id).or_default() += 1;
        }
    }
    // Pass columns per edge, in column order.
    let mut cols_of: HashMap<EdgeId, Vec<usize>> = HashMap::new();
    for (c, col) in l.columns.iter().enumerate() {
        for n in col {
            if let LNode::Pass(e) = n {
                cols_of.entry(*e).or_default().push(c);
            }
        }
    }
    let mut edges: Vec<(EdgeId, Vec<usize>)> = cols_of.into_iter().collect();
    // Longer edges first: they have the least freedom.
    edges.sort_by_key(|(e, cols)| (std::cmp::Reverse(cols.len()), *e));
    for (e, cols) in edges {
        let edge = &g.edges[e.0 as usize];
        let (_, src_row) = geo.attach[&LNode::Real(edge.from)];
        let (_, dst_row) = geo.attach[&LNode::Real(edge.to)];
        let outs = out_deg.get(&edge.from).copied().unwrap_or(0);
        let ins = in_deg.get(&edge.to).copied().unwrap_or(0);
        let want = if outs >= 2 && ins == 1 {
            dst_row
        } else {
            src_row
        };
        let free = |geo: &Geometry, row: i32| cols.iter().all(|&c| !blocked(geo, c, row));
        let row = if free(geo, want) {
            want
        } else {
            // Any row between the two ends adds no detour; among those (or
            // failing that, the rest) the one nearest the wanted row. Every
            // row past the tallest column is free, so the range suffices.
            let (lo, hi) = (src_row.min(dst_row), src_row.max(dst_row));
            let detour = |r: i32| (lo - r).max(0) + (r - hi).max(0);
            (0..=geo.cards_h + 2)
                .filter(|&r| free(geo, r))
                .min_by_key(|&r| (detour(r), (r - want).abs(), r))
                .unwrap_or(want)
        };
        for &c in &cols {
            geo.attach.insert(LNode::Pass(e), (c, row));
        }
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

/// The score of the centred placement alone: a cheap stand-in for `build`
/// when comparing column orders.
pub(crate) fn preview(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
) -> Score {
    let plain = centred(&cards(&layered.columns), i32::from(opts.card_h));
    score(
        g,
        &build_with(g, ranked, layered, &plain, edge_colour, badges, opts),
    )
}

pub(crate) fn build(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
) -> (Layout, Score) {
    let card_h = i32::from(opts.card_h);
    let (plain, sweeps) = placements(g, layered, card_h);
    let mut best = build_with(g, ranked, layered, &plain, edge_colour, badges, opts);
    let base = score(g, &best);
    let mut best_score = base.clone();
    let mut best_ys = plain;
    for ys in sweeps {
        let layout = build_with(g, ranked, layered, &ys, edge_colour, badges, opts);
        let s = score(g, &layout);
        // Never worse than the centred layout on either tier, then lowest total.
        let no_worse = s
            .vocabulary()
            .iter()
            .zip(base.vocabulary())
            .all(|(n, o)| *n <= o)
            && s.soft() <= base.soft();
        if no_worse && s.total < best_score.total {
            best = layout;
            best_score = s;
            best_ys = ys;
        }
    }
    // Two cards on the rows of two in the next column, joined crosswise,
    // cannot be drawn: whichever track is left, its exit lands on the
    // other's corner. Shift a whole column off its neighbours' rows, up to
    // half a step, while that removes an overlap; the lowest total, then
    // the lowest layout, wins.
    let half = card_h.midpoint(SLOT_GAP);
    while best_score.overlaps > 0 {
        let mut found: Option<(Placement, Layout, Score)> = None;
        let rank = |s: &Score| (s.vocabulary(), s.total, s.height);
        for c in 0..best_ys.len() {
            for d in (1..=half).flat_map(|d| [d, -d]) {
                if best_ys[c].iter().any(|&y| y + d < 0) {
                    continue;
                }
                let mut ys = best_ys.clone();
                for y in &mut ys[c] {
                    *y += d;
                }
                let layout = build_with(g, ranked, layered, &ys, edge_colour, badges, opts);
                let s = score(g, &layout);
                if s.vocabulary() < best_score.vocabulary()
                    && found.as_ref().is_none_or(|(_, _, f)| rank(&s) < rank(f))
                {
                    found = Some((ys, layout, s));
                }
            }
        }
        let Some((ys, layout, s)) = found else { break };
        best_ys = ys;
        best = layout;
        best_score = s;
    }
    (best, best_score)
}

fn build_with(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    ys: &Placement,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
) -> Layout {
    let cols = layered.columns.len();
    let card_w = i32::from(opts.card_w);
    let geo = geometry(g, layered, ys, opts);
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
