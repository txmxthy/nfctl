//! Coordinates: card and pass rows per column, gap widths from track counts,
//! one polyline per edge, lanes for back edges.

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
/// Rows a layout may grow past its tallest column for pass slots between
/// stacked cards.
pub(crate) const SLOT_RISE: i32 = 2;
/// Placement sweeps tried after the centred layout: left-to-right, then
/// right-to-left, alternating.
const SWEEPS: usize = 4;
/// Rows a card may be nudged either way to centre its fans on their extremes.
const NUDGE: i32 = 2;
/// Passes of the nudge search.
const NUDGE_PASSES: usize = 2;
/// Rasters `refine` may spend; the largest corpus pipeline is near the
/// debug-time budget before it starts.
const REFINE_DRAWS: usize = 4;
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
    /// Edges on each pass row given out so far, per column.
    taken: HashMap<(usize, i32), Vec<EdgeId>>,
    /// Rows held open between two stacked cards, per column.
    reserved: BTreeSet<(usize, i32)>,
    /// The held rows (`lo..=hi`) of the gap an edge's pass slot sits in, per
    /// column it passes.
    slot: HashMap<(usize, EdgeId), (i32, i32)>,
}

/// Card top rows per column, in column order.
type Placement = Vec<Vec<i32>>;

/// Blank rows between each pair of stacked cards, per column.
type Gaps = Vec<Vec<i32>>;

/// Rows between a pair of stacked cards holding `n` pass slots: a margin, a
/// row per pass, a margin, and one more when `n` is even so the column stays
/// an odd height.
fn slot_gap(n: usize) -> i32 {
    if n == 0 {
        SLOT_GAP
    } else {
        SLOT_GAP + 1 + i(n) + i32::from(n.is_multiple_of(2))
    }
}

/// One blank row between stacked cards keeps every column an odd height, so
/// a card placed on the median of its neighbours sits exactly between two of
/// them. With `slots`, a pair the column order puts pass slots between gets
/// `slot_gap` of them, which keeps the parity.
fn gaps(columns: &[Vec<LNode>], slots: bool) -> Gaps {
    columns
        .iter()
        .map(|col| {
            let mut out = Vec::new();
            let mut cards = 0;
            let mut pass = 0;
            for n in col {
                match n {
                    LNode::Pass(_) => pass += 1,
                    LNode::Real(_) => {
                        if cards > 0 {
                            out.push(if slots { slot_gap(pass) } else { SLOT_GAP });
                        }
                        cards += 1;
                        pass = 0;
                    }
                }
            }
            out
        })
        .collect()
}

fn height_of(cards: usize, gaps: &[i32], card_h: i32) -> i32 {
    i(cards) * card_h + gaps.iter().sum::<i32>()
}

fn tallest(cards: &[Vec<NodeId>], gaps: &Gaps, card_h: i32) -> i32 {
    cards
        .iter()
        .zip(gaps)
        .map(|(c, g)| height_of(c.len(), g, card_h))
        .max()
        .unwrap_or(0)
}

fn geometry(
    g: &ViewGraph,
    l: &Layered,
    ys: &Placement,
    gaps: &Gaps,
    opts: LayoutOptions,
) -> Geometry {
    let card_h = i32::from(opts.card_h);
    // Cards only: pass slots are placed afterwards, on a row the edge already
    // travels on wherever that row is free, or on the row held open for it
    // between two stacked cards.
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
            .max(tallest(&cards(&l.columns), gaps, card_h)),
        taken: HashMap::new(),
        reserved: BTreeSet::new(),
        slot: HashMap::new(),
    };
    for (c, col) in l.columns.iter().enumerate() {
        let mut placed = 0;
        let mut prev: Option<i32> = None;
        let mut pending: Vec<EdgeId> = Vec::new();
        let slots = col
            .iter()
            .map(|&n| match n {
                LNode::Pass(e) => {
                    pending.push(e);
                    Slot::Pass(e)
                }
                LNode::Real(id) => {
                    let y = ys[c][placed];
                    if let Some(py) = prev
                        && gaps[c][placed - 1] > SLOT_GAP
                    {
                        // The middle rows of the gap, past both margins.
                        let n = i(pending.len());
                        let lo = py + card_h + 1 + (gaps[c][placed - 1] - 2 - n) / 2;
                        let hi = lo + n - 1;
                        geo.reserved.extend((lo..=hi).map(|r| (c, r)));
                        for e in pending.drain(..) {
                            geo.slot.insert((c, e), (lo, hi));
                        }
                    }
                    pending.clear();
                    prev = Some(y);
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
/// tallest; then cards placed by their neighbours, once per `centre`. A
/// left-to-right sweep puts each card on the centre row of its sources in the
/// previous column, a right-to-left sweep on the centre of its targets in the
/// next column; a long edge counts as its real endpoint. Within a column
/// cards stack in order, the column then shifts as a whole by the centre of
/// what its cards still want, and is pressed into the tallest column's
/// height so the layout never grows. The caller scores each candidate on the
/// drawn geometry and keeps the best.
fn placements(
    g: &ViewGraph,
    l: &Layered,
    gaps: &Gaps,
    card_h: i32,
    centres: &[Centre],
) -> (Placement, Vec<Placement>) {
    let cards = cards(&l.columns);
    let tallest = tallest(&cards, gaps, card_h);
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
    let plain = centred(&cards, gaps, card_h);
    let mut out = Vec::new();
    let row = |ys: &Placement, id: NodeId| {
        let (c, k) = at[&id];
        mid(ys[c][k])
    };
    let sweep = |ys: &mut Placement, c: usize, down: bool, centre: Centre| {
        let desired: Vec<Option<i32>> = cards[c]
            .iter()
            .map(|&id| {
                let mut rows: Vec<i32> = hops
                    .iter()
                    .filter(|&&(from, to)| if down { to == id } else { from == id })
                    .map(|&(from, to)| row(ys, if down { from } else { to }))
                    .collect();
                rows.sort_unstable();
                centre(&rows)
            })
            .collect();
        let col = &mut ys[c];
        let mut bottom = 0;
        for (k, want) in desired.iter().enumerate() {
            let floor = if k == 0 { 0 } else { bottom + gaps[c][k - 1] };
            col[k] = want.map_or(floor, |w| (w - card_h / 2).max(floor));
            bottom = col[k] + card_h;
        }
        let mut residual: Vec<i32> = desired
            .iter()
            .zip(col.iter())
            .filter_map(|(w, &y)| w.map(|w| w - mid(y)))
            .collect();
        residual.sort_unstable();
        let shift = centre(&residual).unwrap_or((tallest - bottom) / 2);
        for y in col.iter_mut() {
            *y += shift;
        }
        fit(col, &gaps[c], card_h, tallest);
    };
    for &centre in centres {
        let mut ys = plain.clone();
        for pass in 0..SWEEPS {
            if pass % 2 == 0 {
                for c in 0..cols {
                    sweep(&mut ys, c, true, centre);
                }
            } else {
                for c in (0..cols.saturating_sub(1)).rev() {
                    sweep(&mut ys, c, false, centre);
                }
            }
            if ys != plain && !out.contains(&ys) {
                out.push(ys.clone());
            }
        }
    }
    (plain, out)
}

/// How a sweep centres a card on a sorted list of neighbour rows.
type Centre = fn(&[i32]) -> Option<i32>;

/// Midpoint of the first and last of a sorted list, rounded down or `up`.
/// A fan is symmetric when its bus row is the midpoint of its outermost
/// rows, whatever lies between; the median is what the eye expects of a
/// stack of neighbours.
fn extremes(sorted: &[i32], up: bool) -> Option<i32> {
    let (lo, hi) = (*sorted.first()?, *sorted.last()?);
    Some((lo + hi + i32::from(up)).div_euclid(2))
}

/// The two roundings of the extremes' midpoint.
const EXTREMES: [Centre; 2] = [|s| extremes(s, false), |s| extremes(s, true)];

/// Every column centred on the tallest.
fn centred(cards: &[Vec<NodeId>], gaps: &Gaps, card_h: i32) -> Placement {
    let tallest = tallest(cards, gaps, card_h);
    cards
        .iter()
        .zip(gaps)
        .map(|(col, g)| {
            let mut y = (tallest - height_of(col.len(), g, card_h)) / 2;
            (0..col.len())
                .map(|k| {
                    let top = y;
                    y += card_h + g.get(k).copied().unwrap_or(0);
                    top
                })
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
fn fit(col: &mut [i32], gaps: &[i32], card_h: i32, height: i32) {
    let n = col.len();
    for k in (0..n).rev() {
        let limit = if k + 1 < n {
            col[k + 1] - card_h - gaps[k]
        } else {
            height - card_h
        };
        col[k] = col[k].min(limit);
    }
    for k in 0..n {
        let limit = if k > 0 {
            col[k - 1] + card_h + gaps[k - 1]
        } else {
            0
        };
        col[k] = col[k].max(limit);
    }
}

/// Pass columns per edge, in column order.
fn pass_columns(l: &Layered) -> HashMap<EdgeId, Vec<usize>> {
    let mut cols_of: HashMap<EdgeId, Vec<usize>> = HashMap::new();
    for (c, col) in l.columns.iter().enumerate() {
        for n in col {
            if let LNode::Pass(e) = n {
                cols_of.entry(*e).or_default().push(c);
            }
        }
    }
    cols_of
}

/// Forward degrees: every forward edge has one segment out of its real
/// source and one into its real target.
fn degrees(l: &Layered) -> (HashMap<NodeId, usize>, HashMap<NodeId, usize>) {
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
    (out_deg, in_deg)
}

/// The row an edge's pass slots want: the target's when the edge leaves a
/// fork for a target with one in-edge, else the source's (which is also the
/// row of a card with one out-edge feeding a join).
fn wanted(outs: usize, ins: usize, src_row: i32, dst_row: i32) -> i32 {
    if outs >= 2 && ins == 1 {
        dst_row
    } else {
        src_row
    }
}

/// Give every pass slot a row, one row per edge across all the columns it
/// passes so a long edge runs straight. The row wanted is `wanted`. It must
/// be free in every pass column, otherwise the free row nearest the wanted
/// one, a row held open for this edge's slot between two stacked cards
/// before any other. A row is free in a column when no card there covers it
/// or the row beside it, no pass of an unrelated edge (another source and
/// another target) in that column has it, and it is not held open for other
/// edges. Related edges may share a row: the shared run is their fork or
/// join.
fn pass_rows(g: &ViewGraph, l: &Layered, geo: &mut Geometry, card_h: i32) {
    let held_for = |geo: &Geometry, c: usize, row: i32, e: EdgeId| {
        geo.slot
            .get(&(c, e))
            .is_some_and(|&(lo, hi)| (lo..=hi).contains(&row))
    };
    let blocked = |geo: &Geometry, c: usize, row: i32, e: EdgeId, share: bool| {
        let edge = &g.edges[e.0 as usize];
        geo.cards
            .iter()
            .filter(|k| k.col == c)
            .any(|k| row >= k.y - 1 && row <= k.y + card_h)
            || geo.taken.get(&(c, row)).is_some_and(|v| {
                v.iter().any(|o| {
                    let o = &g.edges[o.0 as usize];
                    !share || (o.from != edge.from && o.to != edge.to)
                })
            })
            || (geo.reserved.contains(&(c, row)) && !held_for(geo, c, row, e))
    };
    let (out_deg, in_deg) = degrees(l);
    let mut edges: Vec<(EdgeId, Vec<usize>)> = pass_columns(l).into_iter().collect();
    // Longer edges first: they have the least freedom.
    edges.sort_by_key(|(e, cols)| (std::cmp::Reverse(cols.len()), *e));
    for (e, cols) in edges {
        let edge = &g.edges[e.0 as usize];
        let (_, src_row) = geo.attach[&LNode::Real(edge.from)];
        let (_, dst_row) = geo.attach[&LNode::Real(edge.to)];
        let outs = out_deg.get(&edge.from).copied().unwrap_or(0);
        let ins = in_deg.get(&edge.to).copied().unwrap_or(0);
        let want = wanted(outs, ins, src_row, dst_row);
        let free = |geo: &Geometry, row: i32, share: bool| {
            cols.iter().all(|&c| !blocked(geo, c, row, e, share))
        };
        let row = if free(geo, want, false) {
            want
        } else {
            // Any row between the two ends adds no detour; among those (or
            // failing that, the rest) a row of its own before one shared
            // with a related edge, a row held open for this edge, then the
            // one nearest the wanted row. Every row past the tallest column
            // is free, so the range suffices.
            let (lo, hi) = (src_row.min(dst_row), src_row.max(dst_row));
            let detour = |r: i32| (lo - r).max(0) + (r - hi).max(0);
            let held = |r: i32| cols.iter().any(|&c| held_for(geo, c, r, e));
            (0..=geo.cards_h + 2)
                .filter(|&r| free(geo, r, true))
                .min_by_key(|&r| {
                    (
                        detour(r),
                        !free(geo, r, false),
                        !held(r),
                        (r - want).abs(),
                        r,
                    )
                })
                .unwrap_or(want)
        };
        for &c in &cols {
            geo.attach.insert(LNode::Pass(e), (c, row));
            geo.taken.entry((c, row)).or_default().push(e);
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

/// The centred placement alone, drawn and scored: a cheap stand-in for
/// `build` when comparing column orders, and its first candidate.
pub(crate) fn preview(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
) -> (Layout, Score) {
    let gaps = gaps(&layered.columns, false);
    let plain = centred(&cards(&layered.columns), &gaps, i32::from(opts.card_h));
    let layout = build_with(g, ranked, layered, &plain, &gaps, edge_colour, badges, opts);
    let s = score(g, &layout);
    (layout, s)
}

/// `s` no worse than `base` on either tier.
pub(crate) fn no_worse(s: &Score, base: &Score) -> bool {
    s.vocabulary()
        .iter()
        .zip(base.vocabulary())
        .all(|(n, o)| *n <= o)
        && s.soft() <= base.soft()
}

/// Whether the column order puts a pass slot between two stacked cards
/// anywhere, so that `build` with `slots` differs from without.
pub(crate) fn has_slots(columns: &[Vec<LNode>]) -> bool {
    gaps(columns, true) != gaps(columns, false)
}

/// The column order with every pass slot in the gap the drawn rows say it
/// needs, `layout` being the order drawn without slots. A pass whose wanted
/// row falls inside a column's stack of cards goes between the pair whose
/// gap adds the least detour, nearest the wanted row; one above or below the
/// stack goes above or below it. Each gap widens by a row per pass, so a
/// column takes them in that order only while it stays within `SLOT_RISE`
/// of the tallest column; the rest go above or below, whichever is nearer.
pub(crate) fn reslot(g: &ViewGraph, l: &Layered, layout: &Layout) -> Vec<Vec<LNode>> {
    let (out_deg, in_deg) = degrees(l);
    let card = |id: NodeId| layout.card(id).map_or((0, 0), |k| (k.y, i32::from(k.h)));
    let row = |id: NodeId| {
        let (y, h) = card(id);
        y + h / 2
    };
    // Rows the cards and pass rows reach, without the lanes.
    let cards_h = i32::from(layout.height) - i32::from(layout.lanes);
    let cols_of = pass_columns(l);
    // A row a card in column `c` covers, margins included.
    let covered = |c: usize, r: i32| {
        layout
            .cards
            .iter()
            .any(|k| k.col == c && r >= k.y - 1 && r <= k.y + i32::from(k.h))
    };
    l.columns
        .iter()
        .enumerate()
        .map(|(here, col)| {
            let cards: Vec<NodeId> = col
                .iter()
                .filter_map(|n| match n {
                    LNode::Real(id) => Some(*id),
                    LNode::Pass(_) => None,
                })
                .collect();
            if cards.len() < 2 {
                return col.clone();
            }
            let (top, _) = card(cards[0]);
            let (ly, lh) = card(cards[cards.len() - 1]);
            let bottom = ly + lh;
            let room = cards_h + SLOT_RISE - (bottom - top);
            // Above or below the stack, whichever is nearer.
            let outside = |want: i32| {
                if want - top > bottom - want {
                    cards.len()
                } else {
                    0
                }
            };
            // Blank row under each card but the last: the gap it heads.
            let gap_row: Vec<i32> = cards[..cards.len() - 1]
                .iter()
                .map(|&id| {
                    let (y, h) = card(id);
                    y + h
                })
                .collect();
            // (key, edge, region): region 0 is above the first card, k is
            // the gap under card k - 1, `cards.len()` is below the last.
            let mut inside = Vec::new();
            let mut region: Vec<(usize, EdgeId)> = Vec::new();
            for n in col {
                let LNode::Pass(e) = *n else { continue };
                let edge = &g.edges[e.0 as usize];
                let (src, dst) = (row(edge.from), row(edge.to));
                let outs = out_deg.get(&edge.from).copied().unwrap_or(0);
                let ins = in_deg.get(&edge.to).copied().unwrap_or(0);
                let want = wanted(outs, ins, src, dst);
                let (lo, hi) = (src.min(dst), src.max(dst));
                let detour = |r: i32| (lo - r).max(0) + (r - hi).max(0);
                // A gap this edge can use: its row is clear of cards in
                // every other column the edge passes, as one row serves
                // them all.
                let usable = gap_row
                    .iter()
                    .enumerate()
                    .filter(|&(_, &r)| cols_of[&e].iter().all(|&c| c == here || !covered(c, r)))
                    .map(|(k, &r)| ((detour(r), (r - want).abs()), k + 1))
                    .min();
                match usable {
                    Some((key, k)) if want >= top && want < bottom => {
                        inside.push((key, e, k, want));
                    }
                    _ => region.push((outside(want), e)),
                }
            }
            inside.sort_unstable();
            let mut count = vec![0usize; cards.len()];
            let mut extra = 0;
            for (_, e, k, want) in inside {
                let grown = extra - slot_gap(count[k]) + slot_gap(count[k] + 1);
                if grown <= room {
                    count[k] += 1;
                    extra = grown;
                    region.push((k, e));
                } else {
                    region.push((outside(want), e));
                }
            }
            interleave(&cards, region)
        })
        .collect()
}

/// The column with each pass slot in its region: 0 above the first card,
/// `k` under card `k - 1`, `cards.len()` below the last; by edge id within
/// a region.
fn interleave(cards: &[NodeId], mut region: Vec<(usize, EdgeId)>) -> Vec<LNode> {
    region.sort_unstable();
    let mut out = Vec::new();
    let mut passes = region.into_iter().peekable();
    for k in 0..=cards.len() {
        while let Some((_, e)) = passes.next_if(|&(r, _)| r == k) {
            out.push(LNode::Pass(e));
        }
        if let Some(&id) = cards.get(k) {
            out.push(LNode::Real(id));
        }
    }
    out
}

/// The best placement for this column order. With `slots`, a pass slot is
/// held open between every pair of stacked cards the order puts one between.
/// `centred` is the order's `preview`, when the caller has it: the same
/// drawing as the first candidate, so it is not drawn twice.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
    slots: bool,
    centred: Option<(Layout, Score)>,
) -> (Layout, Score) {
    let gaps = gaps(&layered.columns, slots);
    let card_h = i32::from(opts.card_h);
    let (plain, sweeps) = placements(g, layered, &gaps, card_h, &[median]);
    let (mut best, base) = match centred {
        Some(drawn) if !slots => drawn,
        _ => {
            let l = build_with(g, ranked, layered, &plain, &gaps, edge_colour, badges, opts);
            let s = score(g, &l);
            (l, s)
        }
    };
    let mut best_score = base.clone();
    let mut best_ys = plain;
    for ys in sweeps {
        let layout = build_with(g, ranked, layered, &ys, &gaps, edge_colour, badges, opts);
        let s = score(g, &layout);
        // Never worse than the centred layout on either tier, then lowest total.
        if no_worse(&s, &base) && s.total < best_score.total {
            best = layout;
            best_score = s;
            best_ys = ys;
        }
    }
    // Two cards on the rows of two in the next column, joined crosswise,
    // cannot be drawn: whichever track is left, its exit lands on the
    // other's corner. Shift a whole column off its neighbours' rows, up to
    // half a step, while that removes an overlap; the lowest total, then
    // the lowest layout, wins. Only the columns beside the gap the
    // overlapping edges meet in are tried: every score costs a raster.
    let half = card_h.midpoint(SLOT_GAP);
    while best_score.overlaps > 0 {
        let mut found: Option<(Placement, Layout, Score)> = None;
        let rank = |s: &Score| (s.vocabulary(), s.total, s.height);
        let columns = conflict_columns(g, ranked, &best, &best_score);
        for d in (1..=half).flat_map(|d| [d, -d]) {
            for &c in &columns {
                if best_ys[c].iter().any(|&y| y + d < 0) {
                    continue;
                }
                let mut ys = best_ys.clone();
                for y in &mut ys[c] {
                    *y += d;
                }
                let layout = build_with(g, ranked, layered, &ys, &gaps, edge_colour, badges, opts);
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

/// The columns beside every gap where two or more overlapping edges have a
/// vertical run; failing that, the columns the overlapping edges start or
/// end in.
fn conflict_columns(g: &ViewGraph, ranked: &Ranked, l: &Layout, s: &Score) -> BTreeSet<usize> {
    let overlapping: Vec<usize> = s
        .edges
        .iter()
        .filter(|es| es.overlaps > 0)
        .map(|es| es.edge.0 as usize)
        .collect();
    let gap_at = |x: i32| {
        l.gaps
            .iter()
            .position(|gp| x >= gp.x0 && x < gp.x0 + i32::from(gp.width))
    };
    let mut per_gap: HashMap<usize, usize> = HashMap::new();
    for &ei in &overlapping {
        let mut seen = BTreeSet::new();
        for w in l.routes[ei].polyline.windows(2) {
            if w[0].0 == w[1].0
                && let Some(k) = gap_at(w[0].0)
            {
                seen.insert(k);
            }
        }
        for k in seen {
            *per_gap.entry(k).or_default() += 1;
        }
    }
    let beside: BTreeSet<usize> = per_gap
        .iter()
        .filter(|&(_, &n)| n >= 2)
        .flat_map(|(&k, _)| [k, k + 1])
        .collect();
    if !beside.is_empty() {
        return beside;
    }
    overlapping
        .iter()
        .flat_map(|&ei| {
            let e = &g.edges[ei];
            [ranked.rank[e.from.0 as usize], ranked.rank[e.to.0 as usize]]
        })
        .collect()
}

/// The winner once more with each fan centred on its extremes: the sweeps
/// with the extremes' midpoint as the centre, then each card whose bus row
/// is not the midpoint of the outermost rows its branches leave on (or join
/// on) nudged there, as far as the cards stacked with it allow. A candidate
/// is drawn only when its geometry alone (`proxy`) promises a lower soft
/// tier, and kept when the drawn total falls without raising either tier or
/// the height. Every raster costs, so the draws are budgeted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn refine(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
    slots: bool,
    best: (Layout, Score),
) -> (Layout, Score) {
    let gaps = gaps(&layered.columns, slots);
    let card_h = i32::from(opts.card_h);
    let (mut best, mut best_score) = best;
    let columns = cards(&layered.columns);
    let mut best_ys: Placement = columns
        .iter()
        .map(|col| {
            col.iter()
                .map(|&id| best.card(id).map_or(0, |k| k.y))
                .collect()
        })
        .collect();
    let at: HashMap<NodeId, (usize, usize)> = columns
        .iter()
        .enumerate()
        .flat_map(|(c, col)| col.iter().enumerate().map(move |(k, &id)| (id, (c, k))))
        .collect();
    let proxy_of =
        |ys: &Placement| proxy(g, ranked, layered, &geometry(g, layered, ys, &gaps, opts));
    let mut cur = proxy_of(&best_ys);
    // Rows the cards and pass rows reach, without the lanes.
    let reach = i32::from(best_score.height) - i32::from(best.lanes);
    let mut draws = 0;
    // Draw `ys` when its geometry promises less; keep it when the score agrees.
    let mut try_ys = |ys: Placement,
                      best_ys: &mut Placement,
                      best: &mut Layout,
                      best_score: &mut Score,
                      cur: &mut Proxy|
     -> bool {
        if draws == REFINE_DRAWS {
            return false;
        }
        let p = proxy_of(&ys);
        if p.soft() >= cur.soft() {
            return false;
        }
        draws += 1;
        let layout = build_with(g, ranked, layered, &ys, &gaps, edge_colour, badges, opts);
        let s = score(g, &layout);
        if !(no_worse(&s, best_score)
            && s.total < best_score.total
            && s.height <= best_score.height)
        {
            return false;
        }
        *best_ys = ys;
        *best = layout;
        *best_score = s;
        *cur = p;
        true
    };
    for ys in placements(g, layered, &gaps, card_h, &EXTREMES).1 {
        try_ys(ys, &mut best_ys, &mut best, &mut best_score, &mut cur);
    }
    for _ in 0..NUDGE_PASSES {
        let mut improved = false;
        for (node, moves) in cur.wants.clone() {
            let (c, k) = at[&node];
            for d in moves {
                // The cards stacked beyond it move along as far as needed.
                let mut ys = best_ys.clone();
                let col = &mut ys[c];
                col[k] += d;
                if d > 0 {
                    for j in k + 1..col.len() {
                        col[j] = col[j].max(col[j - 1] + card_h + gaps[c][j - 1]);
                    }
                } else {
                    for j in (0..k).rev() {
                        col[j] = col[j].min(col[j + 1] - card_h - gaps[c][j]);
                    }
                }
                let bottom = col.last().map_or(0, |&y| y + card_h);
                if col[0] < 0 || bottom > reach {
                    continue;
                }
                if try_ys(ys, &mut best_ys, &mut best, &mut best_score, &mut cur) {
                    improved = true;
                    break;
                }
            }
        }
        if !improved {
            break;
        }
    }
    (best, best_score)
}

/// What the scorer's soft tier sees in a geometry before any raster: the
/// asymmetry and detour of the forward edges (crossings need the drawing),
/// and per card with a fan of two or more forward edges whose bus row is not
/// the midpoint of the fan's outermost rows, the rows to move it by (both
/// roundings when the midpoint falls between rows, at most `NUDGE` either
/// way), in card order.
struct Proxy {
    asym: i32,
    detour: i32,
    wants: Vec<(NodeId, Vec<i32>)>,
}

impl Proxy {
    fn soft(&self) -> i32 {
        2 * self.asym + self.detour
    }
}

fn proxy(g: &ViewGraph, ranked: &Ranked, l: &Layered, geo: &Geometry) -> Proxy {
    // Rows each forward edge travels, column by column: its source's, each
    // pass row, its target's. Segments come in column order per edge.
    let mut rows: HashMap<EdgeId, Vec<i32>> = HashMap::new();
    for s in &l.segments {
        let (_, y0) = geo.attach[&s.from];
        let (_, y1) = geo.attach[&s.to];
        rows.entry(s.edge).or_insert_with(|| vec![y0]).push(y1);
    }
    let mut outs: BTreeMap<NodeId, Vec<i32>> = BTreeMap::new();
    let mut ins: BTreeMap<NodeId, Vec<i32>> = BTreeMap::new();
    let mut detour = 0;
    for (ei, e) in g.edges.iter().enumerate() {
        if ranked.back[ei] {
            continue;
        }
        let Some(ys) = rows.get(&edge_id(ei)) else {
            continue;
        };
        let (Some(&first), Some(&last)) = (ys.first(), ys.last()) else {
            continue;
        };
        let steps: Vec<(i32, i32)> = ys.windows(2).map(|w| (w[0], w[1])).collect();
        // The row after the first vertical run, and before the last: where
        // the branch leaves its source's bus and joins its target's.
        let leaves = steps
            .iter()
            .find(|(a, b)| a != b)
            .map_or(first, |&(_, b)| b);
        let joins = steps
            .iter()
            .rev()
            .find(|(a, b)| a != b)
            .map_or(last, |&(a, _)| a);
        detour += steps.iter().map(|(a, b)| (b - a).abs()).sum::<i32>() - (last - first).abs();
        outs.entry(e.from).or_default().push(leaves);
        ins.entry(e.to).or_default().push(joins);
    }
    let mut asym = 0;
    let mut wants: BTreeMap<NodeId, BTreeSet<i32>> = BTreeMap::new();
    for (node, rows) in outs.iter().chain(&ins) {
        if rows.len() < 2 {
            continue;
        }
        let Some(&(_, bus)) = geo.attach.get(&LNode::Real(*node)) else {
            continue;
        };
        let (Some(&lo), Some(&hi)) = (rows.iter().min(), rows.iter().max()) else {
            continue;
        };
        asym += ((bus - lo) - (hi - bus)).abs();
        // Twice the ideal bus row less twice the actual one.
        let off = lo + hi - 2 * bus;
        let moves = wants.entry(*node).or_default();
        for d in [off.div_euclid(2), (off + 1).div_euclid(2)] {
            let d = d.clamp(-NUDGE, NUDGE);
            if d != 0 {
                moves.insert(d);
            }
        }
    }
    Proxy {
        asym,
        detour,
        wants: wants
            .into_iter()
            .map(|(n, m)| (n, m.into_iter().collect()))
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_with(
    g: &ViewGraph,
    ranked: &Ranked,
    layered: &Layered,
    ys: &Placement,
    gaps: &Gaps,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
) -> Layout {
    let cols = layered.columns.len();
    let card_w = i32::from(opts.card_w);
    let geo = geometry(g, layered, ys, gaps, opts);
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
