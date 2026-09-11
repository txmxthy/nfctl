//! Coordinates: card and pass rows per column, gap widths from track counts,
//! one polyline per edge, lanes for back edges.

use std::collections::{BTreeSet, HashMap};

use super::order::{LNode, Layered, cards};
use super::rank::Ranked;
use super::score::{Score, score};
use super::tracks::{Span, pack};
use super::view::ViewEdge;
use super::{
    Badge, Bundling, CardPos, EdgeColour, EdgeId, GapPlan, Layout, LayoutOptions, MIN_GAP, NodeId,
    Route, Slot, Track, ViewGraph, i,
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
/// Rows a column, or every column from one on, may shift either way.
const COLUMN_SHIFT: i32 = 4;
/// Passes of the column-shift search.
const COLUMN_PASSES: usize = 4;
/// Rounds of nudges then column shifts.
const REFINE_ROUNDS: usize = 3;
/// Geometries `refine` may measure, at most, as `REFINE_PROXY_EDGES` edges'
/// worth: a geometry costs in proportion to the edges placed, like a raster.
const REFINE_PROXY_EDGES: usize = 1600;
/// Rasters `refine` may spend, at most; a raster costs in proportion to the
/// edges drawn, and the largest corpus pipeline is near the debug-time budget
/// before it starts, so the budget is `REFINE_DRAW_EDGES` edges' worth.
const REFINE_DRAWS: usize = 4;
const REFINE_DRAW_EDGES: usize = 48;
/// Sweeps `build` may draw beyond the centred placement, on the same rule:
/// cards of different heights put every sweep somewhere different, so a big
/// graph would otherwise raster one for each. `SHIFT_DRAWS` is how many more
/// column shifts are tried once one is known to clear an overlap, so a big
/// graph settles for the first shift that works rather than the best.
const BUILD_DRAWS: usize = 4;
const BUILD_DRAW_EDGES: usize = 240;
const SHIFT_DRAWS: usize = 5;
/// Left margin when a back edge targets column 0.
const BACK_MARGIN: i32 = 3;
/// Pseudo node keys so back-edge verticals never share a track with forward
/// runs: a lane vertical passing through a join would read as part of it.
const LANE_OUT: u32 = u32::MAX - 1;
const LANE_IN: u32 = u32::MAX - 2;

/// What every drawing of one graph shares: the graph, its ranks, the colour
/// per edge, the badges per node and the card size.
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub g: &'a ViewGraph,
    pub ranked: &'a Ranked,
    pub edge_colour: &'a [Option<EdgeColour>],
    pub badges: &'a HashMap<NodeId, Vec<Badge>>,
    pub opts: LayoutOptions,
    /// Card height per node.
    pub heights: &'a [i32],
    /// Give each colour leaving a card its own track, so a bus never carries
    /// two colours. Dropped when the result would give an edge more than one
    /// junction at an end.
    pub by_colour: bool,
    /// Order a fan-in by the row each branch comes in on rather than by the
    /// card it comes from. Dropped on the same terms.
    pub aim_in: bool,
}

impl Ctx<'_> {
    fn card_h(&self) -> i32 {
        i32::from(self.opts.card_h)
    }
}

/// How tall each card is. A card gives one interior row to every colour that
/// leaves it and one to every colour that arrives, so no two colours have to
/// share an attach row and be drawn as one grey stem; a card with fewer edges
/// than that stays at `base`. Only under `Ribbon`, which is what gives each
/// colour a row of its own: nothing else would use the room.
pub(crate) fn heights(g: &ViewGraph, r: &Ranked, opts: LayoutOptions) -> Vec<i32> {
    let base = i32::from(opts.card_h);
    if opts.bundling != Bundling::Ribbon {
        return vec![base; g.nodes.len()];
    }
    let line = super::lines(g);
    let mut out: Vec<BTreeSet<Option<u32>>> = vec![BTreeSet::new(); g.nodes.len()];
    let mut into: Vec<BTreeSet<Option<u32>>> = vec![BTreeSet::new(); g.nodes.len()];
    for (ei, e) in g.edges.iter().enumerate() {
        if r.back[ei] {
            continue; // a back edge takes the middle row and its lane
        }
        let c = line.get(ei).copied().flatten();
        out[e.from.0 as usize].insert(c);
        into[e.to.0 as usize].insert(c);
    }
    // A card also has to hold what is written in it: a name, the numbers, and
    // its tags on as many rows as they take at this width. The painter wraps
    // them the same way, so the two agree.
    let colour = super::colours(g);
    let badges = super::badges_for(g, &colour);
    let room = usize::from(opts.card_w).saturating_sub(2);
    (0..g.nodes.len())
        .map(|n| {
            let id = NodeId(u32::try_from(n).unwrap_or(u32::MAX));
            let labels: Vec<String> = badges
                .get(&id)
                .map(|b| b.iter().map(|b| b.label.clone()).collect())
                .unwrap_or_default();
            let written = 2 + i(super::badge_rows(&labels, room).len());
            let edges = i(out[n].len().max(into[n].len()));
            base.max(edges.max(written) + 2)
        })
        .collect()
}

/// The heights of one column's cards, top to bottom.
fn col_h(h: &[i32], col: &[NodeId]) -> Vec<i32> {
    col.iter()
        .map(|n| h.get(n.0 as usize).copied().unwrap_or(0))
        .collect()
}

fn edge_id(index: usize) -> EdgeId {
    EdgeId(u32::try_from(index).unwrap_or(u32::MAX))
}

fn key(n: LNode) -> u32 {
    match n {
        LNode::Real(id) => id.0,
        LNode::Pass(e) => 0x8000_0000 | e.0,
    }
}

/// A row as an index into a per-column table; rows are never negative.
fn row_index(row: i32) -> usize {
    usize::try_from(row).unwrap_or(0)
}

/// Column and attach row per layered node, plus the card and slot lists.
struct Geometry {
    /// Column and attach row per layered node: nodes first, then an entry
    /// per edge for its pass slots.
    attach: Vec<Option<(usize, i32)>>,
    nodes: usize,
    cards: Vec<CardPos>,
    columns: Vec<Vec<Slot>>,
    cards_h: i32,
    /// Rows held open between two stacked cards, per column and row.
    reserved: Vec<Vec<bool>>,
    /// The held rows (`lo..=hi`) of the gap an edge's pass slot sits in, per
    /// column it passes.
    slot: HashMap<(usize, EdgeId), (i32, i32)>,
    /// The row an edge leaves its source on, when the style gives each edge a
    /// row of its own instead of one shared bus. Empty otherwise.
    exit: Vec<Option<i32>>,
    /// The row an edge meets its target on, likewise.
    entry: Vec<Option<i32>>,
}

impl Geometry {
    fn slot_of(&self, n: LNode) -> usize {
        match n {
            LNode::Real(id) => id.0 as usize,
            LNode::Pass(e) => self.nodes + e.0 as usize,
        }
    }

    /// Where a layered node attaches; every node is placed before it is asked for.
    fn at(&self, n: LNode) -> (usize, i32) {
        self.attach
            .get(self.slot_of(n))
            .copied()
            .flatten()
            .unwrap_or_default()
    }

    /// Where an edge leaves its source: its own row when the style gives it
    /// one, else the card's middle row.
    fn exit_at(&self, e: EdgeId, from: LNode) -> (usize, i32) {
        let (col, row) = self.at(from);
        (
            col,
            self.exit
                .get(e.0 as usize)
                .copied()
                .flatten()
                .unwrap_or(row),
        )
    }

    /// Where an edge meets its target, likewise.
    fn entry_at(&self, e: EdgeId, to: LNode) -> (usize, i32) {
        let (col, row) = self.at(to);
        (
            col,
            self.entry
                .get(e.0 as usize)
                .copied()
                .flatten()
                .unwrap_or(row),
        )
    }

    /// The row a segment end sits on: a card end takes the edge's own row
    /// when it has one, a pass slot its own.
    fn seg_at(&self, e: EdgeId, n: LNode, is_source: bool) -> (usize, i32) {
        match n {
            LNode::Real(_) if is_source => self.exit_at(e, n),
            LNode::Real(_) => self.entry_at(e, n),
            LNode::Pass(_) => self.at(n),
        }
    }

    fn place(&mut self, n: LNode, at: (usize, i32)) {
        let k = self.slot_of(n);
        if let Some(cell) = self.attach.get_mut(k) {
            *cell = Some(at);
        }
    }
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

fn height_of(hs: &[i32], gaps: &[i32]) -> i32 {
    hs.iter().sum::<i32>() + gaps.iter().sum::<i32>()
}

fn tallest(cards: &[Vec<NodeId>], gaps: &Gaps, h: &[i32]) -> i32 {
    cards
        .iter()
        .zip(gaps)
        .map(|(c, g)| height_of(&col_h(h, c), g))
        .max()
        .unwrap_or(0)
}

fn geometry(
    g: &ViewGraph,
    l: &Layered,
    ys: &Placement,
    gaps: &Gaps,
    opts: LayoutOptions,
    heights: &[i32],
    aim_in: bool,
) -> Geometry {
    let cards_in = cards(&l.columns);
    // Cards only: pass slots are placed afterwards, on a row the edge already
    // travels on wherever that row is free, or on the row held open for it
    // between two stacked cards.
    let height = ys
        .iter()
        .zip(&cards_in)
        .flat_map(|(col, ids)| col.iter().zip(col_h(heights, ids)).map(|(&y, k)| y + k))
        .max()
        .unwrap_or(0)
        .max(tallest(&cards_in, gaps, heights));
    let mut geo = Geometry {
        attach: vec![None; g.nodes.len() + g.edges.len()],
        nodes: g.nodes.len(),
        cards: Vec::new(),
        columns: Vec::new(),
        cards_h: height,
        reserved: vec![vec![false; row_index(height) + 1]; l.columns.len()],
        slot: HashMap::new(),
        exit: Vec::new(),
        entry: Vec::new(),
    };
    for (c, col) in l.columns.iter().enumerate() {
        let mut placed = 0;
        let mut prev: Option<(i32, i32)> = None;
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
                    let card_h = heights.get(id.0 as usize).copied().unwrap_or(0);
                    if let Some((py, ph)) = prev
                        && gaps[c][placed - 1] > SLOT_GAP
                    {
                        // The middle rows of the gap, past both margins.
                        let n = i(pending.len());
                        let lo = py + ph + 1 + (gaps[c][placed - 1] - 2 - n) / 2;
                        let hi = lo + n - 1;
                        for r in lo..=hi {
                            if let Some(held) = geo.reserved[c].get_mut(row_index(r)) {
                                *held = true;
                            }
                        }
                        for e in pending.drain(..) {
                            geo.slot.insert((c, e), (lo, hi));
                        }
                    }
                    pending.clear();
                    prev = Some((y, card_h));
                    placed += 1;
                    geo.place(n, (c, y + card_h / 2));
                    geo.cards.push(CardPos {
                        node: id,
                        col: c,
                        x: 0,
                        y,
                        h: u16::try_from(card_h).unwrap_or(0),
                    });
                    Slot::Card(id)
                }
            })
            .collect();
        geo.columns.push(slots);
    }
    if opts.bundling == Bundling::Ribbon {
        // Twice: the first pass has to order each fan by where its branches
        // end, the second by the row they actually leave on, which is only
        // known once the pass rows are out.
        own_rows(g, &mut geo, None, aim_in);
        pass_rows(g, l, &mut geo);
        let aim: Vec<Option<i32>> = (0..g.edges.len())
            .map(|ei| {
                let e = edge_id(ei);
                geo.attach[g.nodes.len() + ei].map(|_| geo.at(LNode::Pass(e)).1)
            })
            .collect();
        own_rows(g, &mut geo, Some(&aim), aim_in);
    }
    pass_rows(g, l, &mut geo);
    if opts.bundling == Bundling::Ribbon {
        ribbon(g, l, &mut geo);
    }
    geo
}

/// Give every edge at a card its own row, so a fan-out leaves as that many
/// coloured lines side by side instead of one shared stub into a bus, and a
/// fan-in arrives as that many arrows. A card has `card_h - 2` rows to give:
/// one edge takes the middle, two take the outer two so the pair stays
/// symmetric, three take all of them. A card with more edges than rows keeps
/// the single bus. Rows go to edges in the order of the cards they join, so
/// the lines do not cross each other on the way out.
fn own_rows(g: &ViewGraph, geo: &mut Geometry, aim: Option<&[Option<i32>]>, aim_in: bool) {
    geo.exit = vec![None; g.edges.len()];
    geo.entry = vec![None; g.edges.len()];
    let line = super::lines(g);
    let card_at = |geo: &Geometry, n: NodeId| geo.at(LNode::Real(n));
    // Edges at each card, as (other end, edge), source side then target side.
    let mut out: Vec<Vec<(i32, EdgeId)>> = vec![Vec::new(); g.nodes.len()];
    let mut into: Vec<Vec<(i32, EdgeId)>> = vec![Vec::new(); g.nodes.len()];
    for (ei, e) in g.edges.iter().enumerate() {
        let id = EdgeId(u32::try_from(ei).unwrap_or(u32::MAX));
        let (from_col, from_row) = card_at(geo, e.from);
        let (to_col, to_row) = card_at(geo, e.to);
        if to_col <= from_col {
            continue; // back edges keep the middle row and their lane
        }
        // The row the edge runs across on, which is where it turns towards on
        // leaving and where it comes in from on arriving. Ordering a fan by
        // where its branches' cards are, when they go somewhere else first,
        // is what makes a fan cross itself.
        let turn = aim.and_then(|a| a.get(ei).copied().flatten());
        out[e.from.0 as usize].push((turn.unwrap_or(to_row), id));
        let comes_from = if aim_in { turn } else { None };
        into[e.to.0 as usize].push((comes_from.unwrap_or(from_row), id));
    }
    // One edge takes the middle row, two the outer two so the pair stays
    // symmetric, three all of them. Past that the rows are shared out in
    // order, so only the edges on a shared row have a shared stub instead of
    // all of them.
    let spread = |n: usize, rows: i32| -> Vec<i32> {
        match (n, rows) {
            (0, _) => Vec::new(),
            (1, _) => vec![rows / 2 + 1],
            (2, r) if r >= 3 => vec![1, r],
            // Centred, so the fan stays symmetric about the card's middle.
            (k, r) if i(k) <= r => {
                let top = 1 + (r - i(k)) / 2;
                (top..top + i(k)).collect()
            }
            // More edges than rows: spread them evenly over the rows, so the
            // pair that has to share sits in the middle and the outer rows
            // stay one edge each.
            (k, r) => (0..k)
                .map(|x| 1 + (i(x) * (r - 1) * 2 + i(k) - 1) / (2 * (i(k) - 1)))
                .collect(),
        }
    };
    // Edges of one colour share a row, so they leave through one line and
    // arrive on one arrowhead; a different colour always gets a row of its
    // own while there are rows left, so nothing that shares a cell has to be
    // drawn grey. Groups take rows in the order of the cards they join.
    let assign = |ends: &mut Vec<(i32, EdgeId)>, node: usize, exit: bool, geo: &mut Geometry| {
        ends.sort_unstable();
        let mut groups: Vec<(Option<u32>, Vec<EdgeId>)> = Vec::new();
        for &(_, e) in ends.iter() {
            let c = line.get(e.0 as usize).copied().flatten();
            match groups.iter_mut().find(|(g, _)| *g == c) {
                Some((_, members)) => members.push(e),
                None => groups.push((c, vec![e])),
            }
        }
        // Groups take rows in the order of the middle of the cards they
        // join, so the lines cross each other as little as leaving together
        // allows.
        let mid = |members: &Vec<EdgeId>| {
            let rows: Vec<i32> = ends
                .iter()
                .filter(|(_, e)| members.contains(e))
                .map(|(r, _)| *r)
                .collect();
            rows.iter().sum::<i32>() * 2 / i(rows.len().max(1))
        };
        groups.sort_by_key(|(_, members)| mid(members));
        let Some((top, rows)) = geo
            .cards
            .iter()
            .find(|k| k.node.0 as usize == node)
            .map(|k| (k.y, i32::from(k.h) - 2))
        else {
            return;
        };
        let offsets = spread(groups.len(), rows);
        if offsets.is_empty() {
            return;
        }
        for ((_, members), offset) in groups.iter().zip(&offsets) {
            for e in members {
                let cell = if exit {
                    &mut geo.exit[e.0 as usize]
                } else {
                    &mut geo.entry[e.0 as usize]
                };
                *cell = Some(top + offset);
            }
        }
    };
    for node in 0..g.nodes.len() {
        let mut ends = std::mem::take(&mut out[node]);
        assign(&mut ends, node, true, geo);
        let mut ends = std::mem::take(&mut into[node]);
        assign(&mut ends, node, false, geo);
    }
}

/// How far a long edge may be pulled off the row that costs it least, to sit
/// against the run above it.
const RIBBON_BUDGET: i32 = 3;

/// Pull the runs that cross the same columns against each other, so long
/// edges read like a ribbon cable instead of scattered lines. Each run moves
/// up to sit directly under the last one placed in every column it crosses,
/// but only while that costs it no more than `RIBBON_BUDGET` rows of detour,
/// so nothing is dragged far from where it belongs. Rows are taken in order,
/// so runs keep their order and none crosses another inside the ribbon.
fn ribbon(g: &ViewGraph, l: &Layered, geo: &mut Geometry) {
    let one_line = same_line(g, true);
    let cols = l.columns.len();
    if cols == 0 {
        return;
    }
    let mut covered: Vec<Vec<bool>> = vec![vec![false; row_index(geo.cards_h) + 1]; cols];
    for k in &geo.cards {
        for r in (k.y - 1).max(0)..=(k.y + i32::from(k.h)) {
            if let Some(hit) = covered[k.col].get_mut(row_index(r)) {
                *hit = true;
            }
        }
    }
    // Every pass edge with the row it holds, nearest the top first.
    let mut placed: Vec<(EdgeId, i32, &Vec<usize>)> = l
        .passes
        .iter()
        .map(|(e, cols)| (*e, geo.at(LNode::Pass(*e)).1, cols))
        .collect();
    placed.sort_by_key(|(e, row, _)| (*row, *e));
    // The rows given out so far, per column, and by which edge.
    let mut taken: Vec<Vec<Vec<EdgeId>>> = vec![Vec::new(); cols];
    // The lowest row used so far in each column.
    let mut last: Vec<Option<i32>> = vec![None; cols];
    // Runs that are the same line: packing puts every run on a row of its
    // own, which would draw a merge group as a stack of parallel lines all
    // saying the same thing. They take their group's row instead.
    let merge = merging(g);
    let mut group_row: HashMap<u32, i32> = HashMap::new();
    for (e, row, pass_cols) in placed {
        let edge = &g.edges[e.0 as usize];
        let (_, src_row) = geo.at(LNode::Real(edge.from));
        let (_, dst_row) = geo.at(LNode::Real(edge.to));
        let (lo, hi) = (src_row.min(dst_row), src_row.max(dst_row));
        let detour = |r: i32| (lo - r).max(0) + (r - hi).max(0);
        let free = |taken: &[Vec<Vec<EdgeId>>], r: i32| {
            r >= 0
                && pass_cols.iter().all(|&col| {
                    let at = row_index(r);
                    !covered[col].get(at).copied().unwrap_or(false)
                        && taken[col]
                            .get(at)
                            .is_none_or(|v| v.iter().all(|&o| one_line(o, e)))
                        && !geo.reserved[col].get(at).copied().unwrap_or(false)
                })
        };
        // Directly under the last run placed in every column this one crosses.
        let against = pass_cols
            .iter()
            .filter_map(|&col| last[col].map(|r| r + 1))
            .max()
            .unwrap_or(0)
            .max(0);
        let group = merge.get(e.0 as usize).copied().flatten();
        let joined = group
            .and_then(|gid| group_row.get(&gid).copied())
            .filter(|&r| free(&taken, r) && detour(r) <= detour(row) + RIBBON_BUDGET);
        let row = joined.unwrap_or_else(|| {
            (against..row)
                .find(|&r| free(&taken, r) && detour(r) <= detour(row) + RIBBON_BUDGET)
                .unwrap_or(row)
        });
        if let Some(gid) = group {
            group_row.entry(gid).or_insert(row);
        }
        for &col in pass_cols {
            geo.place(LNode::Pass(e), (col, row));
            let at = row_index(row);
            if taken[col].len() <= at {
                taken[col].resize(at + 1, Vec::new());
            }
            taken[col][at].push(e);
            last[col] = Some(last[col].map_or(row, |r| r.max(row)));
        }
        geo.cards_h = geo.cards_h.max(row + 1);
    }
}

/// Candidate placements, the plain one first: every column centred on the
/// tallest; then cards placed by their neighbours, once per `centre`. A
/// left-to-right sweep puts each card on the centre row of its sources in the
/// previous column, a right-to-left sweep on the centre of its targets in the
/// next column; a long edge counts as its real endpoint. Within a column
/// cards stack in order, the column then shifts as a whole by the centre of
/// what its cards still want, and is pressed into the tallest column's
/// height plus `rise` so the layout never grows past that. The caller scores
/// each candidate on the drawn geometry and keeps the best.
fn placements(
    g: &ViewGraph,
    l: &Layered,
    gaps: &Gaps,
    heights: &[i32],
    centres: &[Centre],
    rise: i32,
) -> (Placement, Vec<Placement>) {
    let cards = cards(&l.columns);
    let tallest = tallest(&cards, gaps, heights) + rise;
    let cols = cards.len();
    let hs: Vec<Vec<i32>> = cards.iter().map(|c| col_h(heights, c)).collect();
    let mid = |y: i32, k: i32| y + k / 2;
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
    let plain = centred(&cards, gaps, heights);
    let mut out = Vec::new();
    let row = |ys: &Placement, id: NodeId| {
        let (c, k) = at[&id];
        mid(ys[c][k], hs[c][k])
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
            col[k] = want.map_or(floor, |w| (w - hs[c][k] / 2).max(floor));
            bottom = col[k] + hs[c][k];
        }
        let mut residual: Vec<i32> = desired
            .iter()
            .zip(col.iter())
            .zip(&hs[c])
            .filter_map(|((w, &y), &k)| w.map(|w| w - mid(y, k)))
            .collect();
        residual.sort_unstable();
        let shift = centre(&residual).unwrap_or((tallest - bottom) / 2);
        for y in col.iter_mut() {
            *y += shift;
        }
        fit(col, &gaps[c], &hs[c], tallest);
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
fn centred(cards: &[Vec<NodeId>], gaps: &Gaps, h: &[i32]) -> Placement {
    let tallest = tallest(cards, gaps, h);
    cards
        .iter()
        .zip(gaps)
        .map(|(col, g)| {
            let hs = col_h(h, col);
            let mut y = (tallest - height_of(&hs, g)) / 2;
            hs.iter()
                .enumerate()
                .map(|(k, &card_h)| {
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
fn fit(col: &mut [i32], gaps: &[i32], hs: &[i32], height: i32) {
    let n = col.len();
    for k in (0..n).rev() {
        let limit = if k + 1 < n {
            col[k + 1] - hs[k] - gaps[k]
        } else {
            height - hs[k]
        };
        col[k] = col[k].min(limit);
    }
    for k in 0..n {
        let limit = if k > 0 {
            col[k - 1] + hs[k - 1] + gaps[k - 1]
        } else {
            0
        };
        col[k] = col[k].max(limit);
    }
}

/// The row an edge's pass slots want. Staying on the row it left its card on
/// keeps the line where the eye picked it up and puts its one turn at the far
/// end; the alternative, taking the target's row, turns immediately and runs
/// the length of the drawing somewhere else. Only when the branches of a fan
/// still share their exit row is the early turn worth it, because then they
/// have to separate before they can be told apart.
fn wanted(outs: usize, ins: usize, src_row: i32, dst_row: i32, own_row: bool, merge: bool) -> i32 {
    // An edge with company takes the row it will arrive on, which its
    // fellows also take, so they run as one line from as early as they can
    // and only the branches into it show how many there are. Alone, it keeps
    // the row it left on and turns once at the end.
    if merge || (outs >= 2 && ins == 1 && !own_row) {
        dst_row
    } else {
        src_row
    }
}

/// Edges that end at the same card carrying the same tags: they can share one
/// line, so they are given the row they arrive on and merge on to it. Tags,
/// not the colour they are drawn in: the palette has fewer hues than a
/// pipeline can have tag sets, so two unrelated ones can share a hue and must
/// not be drawn as one line.
fn merging(g: &ViewGraph) -> Vec<Option<u32>> {
    let key = |e: &ViewEdge| (e.to, e.tags.join("\u{1f}"));
    let mut count: HashMap<(NodeId, String), usize> = HashMap::new();
    for e in &g.edges {
        *count.entry(key(e)).or_default() += 1;
    }
    let mut group: HashMap<(NodeId, String), u32> = HashMap::new();
    g.edges
        .iter()
        .map(|e| {
            let k = key(e);
            if count.get(&k).copied().unwrap_or(0) < 2 {
                return None;
            }
            let next = u32::try_from(group.len()).unwrap_or(u32::MAX);
            Some(*group.entry(k).or_insert(next))
        })
        .collect()
}

/// Whether two edges may be drawn on one row: they must meet at an end, and
/// under `by_colour` they must be the same colour, since a cell holds one
/// colour and the other would simply be lost.
fn same_line(g: &ViewGraph, by_colour: bool) -> impl Fn(EdgeId, EdgeId) -> bool {
    let colour = super::colours(g);
    let ends: Vec<(NodeId, NodeId)> = g.edges.iter().map(|e| (e.from, e.to)).collect();
    move |a: EdgeId, b: EdgeId| {
        let (i, j) = (a.0 as usize, b.0 as usize);
        let (Some(&(af, at)), Some(&(bf, bt))) = (ends.get(i), ends.get(j)) else {
            return false;
        };
        (af == bf || at == bt)
            && (!by_colour || colour.get(i).copied().flatten() == colour.get(j).copied().flatten())
    }
}

/// The edge every merge group answers to: the lowest id in the group, or the
/// edge itself when it has no company. Empty when rows were not shared out by
/// group, so nothing is treated as merged.
fn merged_rep(g: &ViewGraph, on: bool) -> Vec<EdgeId> {
    let ids: Vec<EdgeId> = (0..g.edges.len()).map(edge_id).collect();
    if !on {
        return ids;
    }
    let mut first: HashMap<u32, EdgeId> = HashMap::new();
    let merge = merging(g);
    ids.iter()
        .enumerate()
        .map(|(i, &id)| match merge[i] {
            Some(gid) => *first.entry(gid).or_insert(id),
            None => id,
        })
        .collect()
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
fn pass_rows(g: &ViewGraph, l: &Layered, geo: &mut Geometry) {
    let cols = l.columns.len();
    let own_row = !geo.exit.is_empty();
    let merge = if own_row {
        merging(g)
    } else {
        vec![None; g.edges.len()]
    };
    let one_line = same_line(g, own_row);
    // The row the first edge of each merge group settled on: its fellows join
    // it there rather than each picking the row that suits it alone, which is
    // what left them drawn as a stack of parallel lines saying one thing.
    let mut group_row: HashMap<u32, i32> = HashMap::new();
    // Rows a card covers, margins included, per column and row.
    let mut covered: Vec<Vec<bool>> = vec![vec![false; row_index(geo.cards_h) + 1]; cols];
    for k in &geo.cards {
        for r in (k.y - 1).max(0)..=(k.y + i32::from(k.h)) {
            if let Some(hit) = covered[k.col].get_mut(row_index(r)) {
                *hit = true;
            }
        }
    }
    // Edges on each pass row given out so far, per column and row.
    let mut taken: Vec<Vec<Vec<EdgeId>>> = vec![Vec::new(); cols];
    for (e, cols) in &l.passes {
        let (e, edge) = (*e, &g.edges[e.0 as usize]);
        // The rows this edge actually leaves and meets its cards on, which
        // are its own when each edge at a card has one. Aiming at the card's
        // middle instead would turn the edge off its row and back again.
        let (_, src_row) = geo.exit_at(e, LNode::Real(edge.from));
        let (_, dst_row) = geo.entry_at(e, LNode::Real(edge.to));
        let want = wanted(
            l.out_deg[edge.from.0 as usize],
            l.in_deg[edge.to.0 as usize],
            src_row,
            dst_row,
            !geo.exit.is_empty(),
            merge.get(e.0 as usize).copied().flatten().is_some(),
        );
        // The rows held open for this edge's slot, per pass column.
        let held_rows: Vec<Option<(i32, i32)>> = cols
            .iter()
            .map(|&c| geo.slot.get(&(c, e)).copied())
            .collect();
        let held_in =
            |k: usize, row: i32| held_rows[k].is_some_and(|(lo, hi)| (lo..=hi).contains(&row));
        let blocked = |taken: &[Vec<Vec<EdgeId>>], k: usize, row: i32, share: bool| {
            let (col, at) = (cols[k], row_index(row));
            row < 0
                || covered[col].get(at).copied().unwrap_or(false)
                || taken[col]
                    .get(at)
                    .is_some_and(|v| v.iter().any(|&o| !share || !one_line(o, e)))
                || (geo.reserved[col].get(at).copied().unwrap_or(false) && !held_in(k, row))
        };
        let free = |taken: &[Vec<Vec<EdgeId>>], row: i32, share: bool| {
            (0..cols.len()).all(|k| !blocked(taken, k, row, share))
        };
        // An edge with company wants the row its fellows are on, so it takes
        // one already given out; alone it wants a row to itself.
        let group = merge.get(e.0 as usize).copied().flatten();
        let with_company = group.is_some();
        let joined = group
            .and_then(|gid| group_row.get(&gid).copied())
            .filter(|&r| free(&taken, r, true));
        let row = if let Some(r) = joined {
            r
        } else if free(&taken, want, with_company) {
            want
        } else {
            // Any row between the two ends adds no detour; among those (or
            // failing that, the rest) a row of its own before one shared
            // with a related edge, a row held open for this edge, then the
            // one nearest the wanted row. Every row past the tallest column
            // is free, so the range suffices.
            let (lo, hi) = (src_row.min(dst_row), src_row.max(dst_row));
            let detour = |r: i32| (lo - r).max(0) + (r - hi).max(0);
            let held = |r: i32| (0..cols.len()).any(|k| held_in(k, r));
            (0..=geo.cards_h + 2)
                .filter(|&r| free(&taken, r, true))
                .min_by_key(|&r| {
                    (
                        detour(r),
                        // Shared where it has company to join, private otherwise.
                        free(&taken, r, false) == with_company,
                        !held(r),
                        (r - want).abs(),
                        r,
                    )
                })
                .unwrap_or(want)
        };
        if let Some(gid) = group {
            group_row.entry(gid).or_insert(row);
        }
        for &col in cols {
            geo.place(LNode::Pass(e), (col, row));
            let at = row_index(row);
            if taken[col].len() <= at {
                taken[col].resize(at + 1, Vec::new());
            }
            taken[col][at].push(e);
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
    colour: &[Option<EdgeColour>],
) -> Spans {
    let cols = l.columns.len();
    let mut per_gap: Vec<Vec<Span>> = vec![Vec::new(); cols.saturating_sub(1)];
    let mut push = |gap: usize, s: Span| {
        per_gap[gap].push(s);
        (gap, per_gap[gap].len() - 1)
    };
    // The pass slots of a merge group are one line, so they answer to one
    // key: the tracks then hold them as a single trunk with a junction per
    // branch, instead of a bundle of parallel verticals a row apart. Only
    // when the rows were shared out that way in the first place.
    let rep = merged_rep(g, !geo.exit.is_empty());
    let node_key = |n: LNode| match n {
        LNode::Pass(e) => key(LNode::Pass(rep[e.0 as usize])),
        LNode::Real(_) => key(n),
    };
    let forward = l
        .segments
        .iter()
        .map(|s| {
            let (_, y0) = geo.seg_at(s.edge, s.from, true);
            let (_, y1) = geo.seg_at(s.edge, s.to, false);
            push(
                s.col,
                Span {
                    edge: s.edge,
                    lo: y0.min(y1),
                    hi: y0.max(y1),
                    y_in: y0,
                    y_out: y1,
                    src: node_key(s.from),
                    dst: node_key(s.to),
                    colour: colour.get(s.edge.0 as usize).copied().flatten(),
                },
            )
        })
        .collect();
    let mut back = HashMap::new();
    for (ei, e) in g.edges.iter().enumerate() {
        if !r.back[ei] {
            continue;
        }
        let (cu, yu) = geo.at(LNode::Real(e.from));
        let (cv, yv) = geo.at(LNode::Real(e.to));
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
                    colour: colour.get(ei).copied().flatten(),
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
                    colour: colour.get(ei).copied().flatten(),
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

/// Where the tracks of each gap sit. Normally they are centred, which leaves
/// a couple of cells between a card and the bus its edges share; those cells
/// carry every branch at once so they have to be drawn grey. `hug` puts the
/// tracks against the card instead, so the shared part is the junction alone.
fn columns(
    sp: &Spans,
    cols: usize,
    card_w: i32,
    margin: i32,
    hug: bool,
    by_colour: bool,
) -> Columns {
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
            let (assign, n) = pack(&sp.per_gap[c], by_colour);
            let width = u16::try_from(n)
                .unwrap_or(u16::MAX)
                .saturating_add(2)
                .max(MIN_GAP);
            let pad = if hug {
                0
            } else {
                (i32::from(width) - 2 - i(n)) / 2
            };
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
        let (c, y0) = geo.seg_at(id, s.from, true);
        let (_, y1) = geo.seg_at(id, s.to, false);
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
    let (cu, yu) = geo.at(LNode::Real(e.from));
    let (cv, yv) = geo.at(LNode::Real(e.to));
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
pub(crate) fn preview(ctx: &Ctx, layered: &Layered) -> (Layout, Score) {
    let gaps = gaps(&layered.columns, false);
    let plain = centred(&cards(&layered.columns), &gaps, ctx.heights);
    let layout = build_with(ctx, layered, &plain, &gaps);
    let s = score(ctx.g, &layout);
    (layout, s)
}

/// What one column order is worth: the vocabulary tier, then the total,
/// then crossings. Ties fall to whichever came first.
pub(crate) type Key = ([usize; 5], i64, usize);

pub(crate) fn order_key(s: &Score) -> Key {
    (s.vocabulary(), s.total, s.crossings)
}

/// `s` no worse than `base` on either tier.
fn no_worse(s: &Score, base: &Score) -> bool {
    s.vocabulary()
        .iter()
        .zip(base.vocabulary())
        .all(|(n, o)| *n <= o)
        && s.soft() <= base.soft()
}

/// The height a layout `height` rows tall may grow to for pass slots.
pub(crate) fn slot_ceiling(height: u16) -> u16 {
    height.saturating_add(u16::try_from(SLOT_RISE).unwrap_or(0))
}

/// The rule every placement loop accepts a candidate by: `s` lowers the
/// incumbent's total without rising on either tier past `tiers` (the
/// incumbent, or the layout a loop holds its tiers to) or in height past
/// `ceiling`.
pub(crate) fn accept(s: &Score, incumbent: &Score, tiers: &Score, ceiling: u16) -> bool {
    no_worse(s, tiers) && s.total < incumbent.total && s.height <= ceiling
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
pub(crate) fn reslot(
    g: &ViewGraph,
    l: &Layered,
    layout: &Layout,
    own_row: bool,
) -> Vec<Vec<LNode>> {
    let merge = if own_row {
        merging(g)
    } else {
        vec![None; g.edges.len()]
    };
    let card = |id: NodeId| layout.card(id).map_or((0, 0), |k| (k.y, i32::from(k.h)));
    let row = |id: NodeId| {
        let (y, h) = card(id);
        y + h / 2
    };
    // Rows the cards and pass rows reach, without the lanes.
    let cards_h = i32::from(layout.height) - i32::from(layout.lanes);
    let cols_of: HashMap<EdgeId, &Vec<usize>> = l.passes.iter().map(|(e, c)| (*e, c)).collect();
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
                let want = wanted(
                    l.out_deg[edge.from.0 as usize],
                    l.in_deg[edge.to.0 as usize],
                    src,
                    dst,
                    own_row,
                    merge.get(e.0 as usize).copied().flatten().is_some(),
                );
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
pub(crate) fn build(
    ctx: &Ctx,
    layered: &Layered,
    slots: bool,
    centred: Option<(Layout, Score)>,
) -> (Layout, Score) {
    let (g, ranked) = (ctx.g, ctx.ranked);
    let gaps = gaps(&layered.columns, slots);
    let card_h = ctx.card_h();
    let (plain, sweeps) = placements(g, layered, &gaps, ctx.heights, &[median], 0);
    let draws = (BUILD_DRAW_EDGES / g.edges.len().max(1)).clamp(1, BUILD_DRAWS);
    let (mut best, base) = match centred {
        Some(built) if !slots => built,
        _ => {
            let l = build_with(ctx, layered, &plain, &gaps);
            let s = score(g, &l);
            (l, s)
        }
    };
    let mut best_score = base.clone();
    let mut best_ys = plain;
    for ys in sweeps.into_iter().take(draws) {
        let layout = build_with(ctx, layered, &ys, &gaps);
        let s = score(g, &layout);
        // Never worse than the centred layout on either tier, then lowest
        // total, whatever the height.
        if accept(&s, &best_score, &base, u16::MAX) {
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
    let mut shifts = (SHIFT_DRAWS * REFINE_DRAW_EDGES / g.edges.len().max(1)).max(1);
    while best_score.overlaps > 0 {
        let mut found: Option<(Placement, Layout, Score)> = None;
        let rank = |s: &Score| (s.vocabulary(), s.total, s.height);
        let columns = conflict_columns(g, ranked, &best, &best_score);
        'shift: for d in (1..=half).flat_map(|d| [d, -d]) {
            for &c in &columns {
                if best_ys[c].iter().any(|&y| y + d < 0) {
                    continue;
                }
                let mut ys = best_ys.clone();
                for y in &mut ys[c] {
                    *y += d;
                }
                let layout = build_with(ctx, layered, &ys, &gaps);
                let s = score(g, &layout);
                if s.vocabulary() < best_score.vocabulary()
                    && found.as_ref().is_none_or(|(_, _, f)| rank(&s) < rank(f))
                {
                    found = Some((ys, layout, s));
                }
                shifts = shifts.saturating_sub(1);
                if found.is_some() && shifts == 0 {
                    break 'shift;
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
/// on) nudged there, as far as the cards stacked with it allow, then whole
/// columns shifted, alone or with every column after them, while that
/// brings rows together. The columns may grow the layout by `SLOT_RISE`
/// from the start; when a nudge is refused by the layout's height, the
/// sweeps and nudges run once more with that allowance too.
/// The search steps on the geometry alone (`proxy`), within a budget of
/// geometries; every state it accepts is then drawn in proxy order, as many
/// as the draw budget allows, and the lowest drawn total is kept when it
/// falls without raising either tier or the height past the allowance.
pub(crate) fn refine(
    ctx: &Ctx,
    layered: &Layered,
    slots: bool,
    best: (Layout, Score),
) -> (Layout, Score) {
    let g = ctx.g;
    let gaps = gaps(&layered.columns, slots);
    let (mut best, mut best_score) = best;
    let columns = cards(&layered.columns);
    let ys: Placement = columns
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
    let edges = g.edges.len().max(1);
    let draws = (REFINE_DRAW_EDGES / edges).clamp(1, REFINE_DRAWS);
    let geometries = (REFINE_PROXY_EDGES / edges).max(1);
    // The starting placement is measured out of the same budget.
    let cur = proxy(
        g,
        ctx.ranked,
        layered,
        &geometry(g, layered, &ys, &gaps, ctx.opts, ctx.heights, ctx.aim_in),
    );
    let mut search = Search {
        ctx: *ctx,
        layered,
        gaps: &gaps,
        left: geometries - 1,
        ys,
        cur,
        seen: Vec::new(),
        hs: cards(&layered.columns)
            .iter()
            .map(|col| col_h(ctx.heights, col))
            .collect(),
    };
    // Rows the cards and pass rows reach, without the lanes.
    let reach = i32::from(best_score.height) - i32::from(best.lanes);
    let ceiling = slot_ceiling(best_score.height);
    'search: for rise in [0, SLOT_RISE] {
        if search.sweeps(rise).is_none() {
            break;
        }
        // A nudge the height refused: what the rise is for.
        let mut held = false;
        for _ in 0..REFINE_ROUNDS {
            let Some(nudged) = search.nudges(&at, reach, rise, &mut held) else {
                break 'search;
            };
            let Some(shifted) = search.columns(reach + SLOT_RISE) else {
                break 'search;
            };
            if !nudged && !shifted {
                break;
            }
        }
        if !held {
            break;
        }
    }
    // Every accepted measure is below the one before it, so the order is
    // total: best measure first.
    let mut seen = search.seen;
    seen.sort_by_key(|(p, _)| *p);
    for (_, ys) in seen.into_iter().take(draws) {
        let layout = build_with(ctx, layered, &ys, &gaps);
        let s = score(g, &layout);
        if accept(&s, &best_score, &best_score, ceiling) {
            best = layout;
            best_score = s;
        }
    }
    (best, best_score)
}

/// The refinement's state: the placement it is at, its measure, and every
/// placement it accepted on the way, with its measure, in order.
struct Search<'a> {
    ctx: Ctx<'a>,
    layered: &'a Layered,
    gaps: &'a Gaps,
    /// Geometries it may still measure.
    left: usize,
    ys: Placement,
    cur: Proxy,
    seen: Vec<(i32, Placement)>,
    /// Card heights per column, top to bottom.
    hs: Vec<Vec<i32>>,
}

impl Search<'_> {
    /// The measure of `ys`, or `None` once the budget is spent.
    fn measure(&mut self, ys: &Placement) -> Option<Proxy> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let geo = geometry(
            self.ctx.g,
            self.layered,
            ys,
            self.gaps,
            self.ctx.opts,
            self.ctx.heights,
            self.ctx.aim_in,
        );
        Some(proxy(self.ctx.g, self.ctx.ranked, self.layered, &geo))
    }

    /// Move to `ys` when it measures less than where the search is.
    fn step(&mut self, ys: Placement, p: Proxy) -> bool {
        if p.soft() >= self.cur.soft() {
            return false;
        }
        self.seen.push((p.soft(), ys.clone()));
        self.ys = ys;
        self.cur = p;
        true
    }

    /// The sweeps with the extremes' midpoint as the centre, the one that
    /// measures least taken. `None` once the budget is spent.
    fn sweeps(&mut self, rise: i32) -> Option<()> {
        let mut found: Option<(Proxy, Placement)> = None;
        for ys in placements(
            self.ctx.g,
            self.layered,
            self.gaps,
            self.ctx.heights,
            &EXTREMES,
            rise,
        )
        .1
        {
            let Some(p) = self.measure(&ys) else { break };
            if found.as_ref().is_none_or(|(f, _)| p.soft() < f.soft()) {
                found = Some((p, ys));
            }
        }
        if let Some((p, ys)) = found {
            self.step(ys, p);
        }
        (self.left > 0).then_some(())
    }

    /// Each card whose fan is off centre nudged towards its midpoint, the
    /// cards stacked beyond it moving along, the first that measures less
    /// taken, for `NUDGE_PASSES`. A nudge the height refuses sets `held`.
    /// Whether one was taken; `None` once the budget is spent.
    fn nudges(
        &mut self,
        at: &HashMap<NodeId, (usize, usize)>,
        reach: i32,
        rise: i32,
        held: &mut bool,
    ) -> Option<bool> {
        let mut moved = false;
        for _ in 0..NUDGE_PASSES {
            let mut improved = false;
            for (node, moves) in self.cur.wants.clone() {
                let (c, k) = at[&node];
                let hs = self.hs[c].clone();
                for d in moves {
                    let mut ys = self.ys.clone();
                    let col = &mut ys[c];
                    col[k] += d;
                    if d > 0 {
                        for j in k + 1..col.len() {
                            col[j] = col[j].max(col[j - 1] + hs[j - 1] + self.gaps[c][j - 1]);
                        }
                    } else {
                        for j in (0..k).rev() {
                            col[j] = col[j].min(col[j + 1] - hs[j] - self.gaps[c][j]);
                        }
                    }
                    let bottom = col.last().map_or(0, |&y| y + hs[col.len() - 1]);
                    if col[0] < 0 {
                        continue;
                    }
                    if bottom > reach + rise {
                        *held = true;
                        continue;
                    }
                    let p = self.measure(&ys)?;
                    if self.step(ys, p) {
                        improved = true;
                        moved = true;
                        break;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        Some(moved)
    }

    /// Whole columns shifted, within rows `0..limit`: the shifts in the
    /// order the cached paths rate them, each measured until one measures
    /// less, for `COLUMN_PASSES`. Whether one was taken; `None` once the
    /// budget is spent.
    fn columns(&mut self, limit: i32) -> Option<bool> {
        // The bottom of each column is its last card's height below its top.
        let last: Vec<i32> = self
            .hs
            .iter()
            .map(|h| h.last().copied().unwrap_or(0))
            .collect();
        let mut moved = false;
        for _ in 0..COLUMN_PASSES {
            let cols = self.ys.len();
            let mut ranked_shifts: Vec<(i32, usize, usize, bool, i32)> = column_shifts(cols)
                .enumerate()
                .map(|(i, (k, suffix, d))| {
                    let mut delta = vec![0; cols];
                    let to = if suffix { cols } else { k + 1 };
                    for x in &mut delta[k..to] {
                        *x = d;
                    }
                    (self.cur.paths.measure(&delta).soft(), i, k, suffix, d)
                })
                .collect();
            ranked_shifts.sort_unstable();
            let mut improved = false;
            'shifts: for (_, _, k, suffix, d) in ranked_shifts {
                for t in [0, 1] {
                    let Some(ys) = column_shift(&self.ys, k, suffix, d, t, &last, limit) else {
                        continue;
                    };
                    if ys == self.ys {
                        continue;
                    }
                    let p = self.measure(&ys)?;
                    // A slot between stacked cards was held for an edge
                    // running straight into it; a column may not move its
                    // card off that row.
                    if p.paths.leaves_slot(&self.cur.paths) {
                        continue;
                    }
                    if self.step(ys, p) {
                        improved = true;
                        moved = true;
                        break 'shifts;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        Some(moved)
    }
}

/// The column shifts: `(k, suffix, d)` moves column `k` by `d` rows, alone
/// or with every column after it. The last column alone is the last suffix.
fn column_shifts(cols: usize) -> impl Iterator<Item = (usize, bool, i32)> {
    (1..=COLUMN_SHIFT).flat_map(|d| [d, -d]).flat_map(move |d| {
        (0..cols).flat_map(move |k| {
            [(k, true, d), (k, false, d)]
                .into_iter()
                .filter(move |&(k, suffix, _)| (!suffix || k > 0) && (suffix || k + 1 < cols))
        })
    })
}

/// `ys` with column `k` moved `d` rows, alone or with every column after
/// it (`suffix`), then the whole layout moved back into rows `0..limit` as
/// little as possible plus `t` rows down, when that fits.
fn column_shift(
    ys: &Placement,
    k: usize,
    suffix: bool,
    d: i32,
    t: i32,
    last: &[i32],
    limit: i32,
) -> Option<Placement> {
    let cols = ys.len();
    let mut shifted = ys.clone();
    let to = if suffix { cols } else { k + 1 };
    for col in &mut shifted[k..to] {
        for y in col.iter_mut() {
            *y += d;
        }
    }
    let top = *shifted.iter().filter_map(|c| c.first()).min()?;
    let bottom = shifted
        .iter()
        .zip(last)
        .filter_map(|(c, &h)| c.last().map(|&y| y + h))
        .max()?;
    let t = t + if top < 0 {
        -top
    } else if bottom > limit {
        limit - bottom
    } else {
        0
    };
    if top + t < 0 || bottom + t > limit {
        return None;
    }
    for col in &mut shifted {
        for y in col.iter_mut() {
            *y += t;
        }
    }
    Some(shifted)
}

/// What the scorer's soft tier sees in a geometry before any raster: the
/// asymmetry and detour of the forward edges (crossings need the drawing),
/// and per card with a fan of two or more forward edges whose bus row is not
/// the midpoint of the fan's outermost rows, the rows to move it by (both
/// roundings when the midpoint falls between rows, at most `NUDGE` either
/// way), in card order. `paths` is the geometry it was measured on, kept so
/// a column shift can be estimated without placing the pass rows again.
struct Proxy {
    asym: i32,
    detour: i32,
    wants: Vec<(NodeId, Vec<i32>)>,
    paths: Paths,
}

impl Proxy {
    fn soft(&self) -> i32 {
        2 * self.asym + self.detour
    }
}

/// `(row, column)` at an edge's source, each pass slot, and its target, the
/// column being the one whose shift moves the point.
type Path = Vec<(i32, usize)>;

/// The rows every forward edge travels, column by column, and every card's
/// bus row, each with the column whose shift moves it: a card's own, and
/// for a pass row the column of the endpoint whose row it wants, since one
/// row serves every column an edge passes.
struct Paths {
    /// Per forward edge: source, target, its path.
    edges: Vec<(NodeId, NodeId, Path)>,
    /// Per node index: `(column, bus row)`.
    bus: Vec<Option<(usize, i32)>>,
    /// Per forward edge (as `edges`): whether a slot between stacked cards
    /// is held open for it.
    slotted: Vec<bool>,
}

/// Asymmetry and detour of a geometry, and how far each fan's card is from
/// its midpoint.
struct Measure {
    asym: i32,
    detour: i32,
    /// Twice the ideal bus row less twice the actual one, per node index.
    off: Vec<Option<i32>>,
}

impl Measure {
    fn soft(&self) -> i32 {
        2 * self.asym + self.detour
    }
}

impl Paths {
    fn from_geometry(g: &ViewGraph, ranked: &Ranked, l: &Layered, geo: &Geometry) -> Self {
        let merge = if geo.exit.is_empty() {
            vec![None; g.edges.len()]
        } else {
            merging(g)
        };
        // Rows each forward edge travels, column by column: its source's, each
        // pass row, its target's. Segments come in column order per edge.
        let mut rows: Vec<Path> = vec![Vec::new(); g.edges.len()];
        // The column a pass row follows, per edge, once known.
        let mut anchor: Vec<Option<usize>> = vec![None; g.edges.len()];
        for s in &l.segments {
            let ei = s.edge.0 as usize;
            let (c0, y0) = geo.seg_at(s.edge, s.from, true);
            let (c1, y1) = geo.seg_at(s.edge, s.to, false);
            if rows[ei].is_empty() {
                rows[ei].push((y0, c0));
            }
            let col = match s.to {
                LNode::Real(_) => c1,
                LNode::Pass(_) => *anchor[ei].get_or_insert_with(|| {
                    let e = &g.edges[ei];
                    let (cs, ys) = geo.at(LNode::Real(e.from));
                    let (ct, yt) = geo.at(LNode::Real(e.to));
                    let outs = l.out_deg[e.from.0 as usize];
                    let ins = l.in_deg[e.to.0 as usize];
                    if wanted(outs, ins, ys, yt, !geo.exit.is_empty(), merge[ei].is_some()) == yt
                        && ys != yt
                    {
                        ct
                    } else {
                        cs
                    }
                }),
            };
            rows[ei].push((y1, col));
        }
        let mut slotted = Vec::new();
        let edges = g
            .edges
            .iter()
            .zip(rows)
            .enumerate()
            .filter(|&(ei, (_, ref pts))| !ranked.back[ei] && !pts.is_empty())
            .map(|(ei, (e, pts))| {
                slotted.push(geo.slot.keys().any(|&(_, se)| se.0 as usize == ei));
                (e.from, e.to, pts)
            })
            .collect();
        let bus = geo.attach[..geo.nodes].to_vec();
        Paths {
            edges,
            bus,
            slotted,
        }
    }

    /// Whether an edge with a pass slot held open for it ran straight from
    /// its source or into its target in `before` and no longer does here.
    fn leaves_slot(&self, before: &Paths) -> bool {
        let straight_end = |pts: &[(i32, usize)]| {
            pts.len() >= 3 && (pts[0].0 == pts[1].0 || pts[pts.len() - 2].0 == pts[pts.len() - 1].0)
        };
        self.edges
            .iter()
            .zip(&before.edges)
            .zip(&before.slotted)
            .any(|(((_, _, now), (_, _, was)), &slotted)| {
                slotted && straight_end(was) && !straight_end(now)
            })
    }

    /// The measure with every column moved by `delta` rows, pass rows
    /// following the column they are anchored to.
    fn measure(&self, delta: &[i32]) -> Measure {
        let at = |&(row, col): &(i32, usize)| row + delta.get(col).copied().unwrap_or(0);
        let n = self.bus.len();
        let mut outs: Vec<Option<(i32, i32)>> = vec![None; n];
        let mut ins: Vec<Option<(i32, i32)>> = vec![None; n];
        let mut fan = vec![0usize; 2 * n];
        let mut detour = 0;
        let extend = |slot: &mut Option<(i32, i32)>, row: i32| {
            *slot = Some(slot.map_or((row, row), |(lo, hi)| (lo.min(row), hi.max(row))));
        };
        for (from, to, pts) in &self.edges {
            let rows: Vec<i32> = pts.iter().map(at).collect();
            let (Some(&first), Some(&last)) = (rows.first(), rows.last()) else {
                continue;
            };
            // The row after the first vertical run, and before the last:
            // where the branch leaves its source's bus and joins its target's.
            let leaves = rows
                .windows(2)
                .find(|w| w[0] != w[1])
                .map_or(first, |w| w[1]);
            let joins = rows
                .windows(2)
                .rev()
                .find(|w| w[0] != w[1])
                .map_or(last, |w| w[0]);
            detour +=
                rows.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<i32>() - (last - first).abs();
            let (f, t) = (from.0 as usize, to.0 as usize);
            extend(&mut outs[f], leaves);
            fan[f] += 1;
            extend(&mut ins[t], joins);
            fan[n + t] += 1;
        }
        let mut asym = 0;
        let mut off = vec![None; n];
        for (k, rows) in outs.iter().chain(&ins).enumerate() {
            let node = k % n;
            let (Some(&(lo, hi)), true) = (rows.as_ref(), fan[k] >= 2) else {
                continue;
            };
            let Some((col, bus)) = self.bus[node] else {
                continue;
            };
            let bus = bus + delta.get(col).copied().unwrap_or(0);
            asym += ((bus - lo) - (hi - bus)).abs();
            // Twice the ideal bus row less twice the actual one; a card
            // both fanning out and in keeps whichever is further.
            let o = lo + hi - 2 * bus;
            let slot: &mut Option<i32> = &mut off[node];
            if slot.is_none_or(|cur: i32| o.abs() > cur.abs()) {
                *slot = Some(o);
            }
        }
        Measure { asym, detour, off }
    }
}

fn proxy(g: &ViewGraph, ranked: &Ranked, l: &Layered, geo: &Geometry) -> Proxy {
    let paths = Paths::from_geometry(g, ranked, l, geo);
    let m = paths.measure(&vec![0; l.columns.len()]);
    let wants = m
        .off
        .iter()
        .enumerate()
        .filter_map(|(n, off)| {
            let off = (*off)?;
            let mut moves: Vec<i32> = [off.div_euclid(2), (off + 1).div_euclid(2)]
                .into_iter()
                .map(|d| d.clamp(-NUDGE, NUDGE))
                .filter(|&d| d != 0)
                .collect();
            moves.sort_unstable();
            moves.dedup();
            (!moves.is_empty()).then(|| (NodeId(u32::try_from(n).unwrap_or(u32::MAX)), moves))
        })
        .collect();
    Proxy {
        asym: m.asym,
        detour: m.detour,
        wants,
        paths,
    }
}

fn build_with(ctx: &Ctx, layered: &Layered, ys: &Placement, gaps: &Gaps) -> Layout {
    let (g, ranked) = (ctx.g, ctx.ranked);
    let cols = layered.columns.len();
    let card_w = i32::from(ctx.opts.card_w);
    let geo = geometry(g, layered, ys, gaps, ctx.opts, ctx.heights, ctx.aim_in);
    let lane = lanes(g, ranked, geo.cards_h);
    let sp = spans(g, ranked, layered, &geo, &lane, ctx.edge_colour);
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
    let cx = columns(
        &sp,
        cols,
        card_w,
        margin,
        ctx.opts.bundling == Bundling::Ribbon,
        ctx.by_colour,
    );
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
                colour: ctx.edge_colour[ei],
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
        badges: ctx.badges.clone(),
        width: u16::try_from(cx.width).unwrap_or(u16::MAX),
        height: u16::try_from(geo.cards_h)
            .unwrap_or(u16::MAX)
            .saturating_add(lanes),
    }
}
