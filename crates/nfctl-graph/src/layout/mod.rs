//! Layered layout for the card view: columns by rank, one-row pass slots for
//! long edges, column order chosen by drawing each barycenter sweep, bundled
//! tracks in the gaps, lanes for back edges. Pure: cell coordinates in, no
//! terminal types. The TUI paints it.

mod order;
mod rank;
mod route;
pub mod score;
#[cfg(test)]
mod tests;
mod tracks;
mod view;

pub use score::{EdgeScore, Score, score};
pub use view::{ViewEdge, ViewGraph, ViewNode};

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct EdgeId(pub u32);

/// Palette slot for an edge's tag combination; the renderer owns the hues.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct EdgeColour(pub u8);

pub const PALETTE_SIZE: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Card(NodeId),
    /// A long edge passing through this column on one row.
    Pass(EdgeId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub x: i32,
    pub edges: Vec<EdgeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapPlan {
    pub x0: i32,
    pub width: u16,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub edge: EdgeId,
    /// Orthogonal polyline; collinear points collapsed.
    pub polyline: Vec<(i32, i32)>,
    /// Cell for the arrowhead (shared by every edge into the same target).
    pub head: (i32, i32),
    pub colour: Option<EdgeColour>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPos {
    pub node: NodeId,
    pub col: usize,
    pub x: i32,
    pub y: i32,
    pub h: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub label: String,
    pub colour: Option<EdgeColour>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub columns: Vec<Vec<Slot>>,
    pub col_x: Vec<i32>,
    pub cards: Vec<CardPos>,
    pub gaps: Vec<GapPlan>,
    pub routes: Vec<Route>,
    /// Back-edge lane rows under the cards.
    pub lanes: u16,
    /// Tag badges per node: one per distinct combination arriving there.
    pub badges: HashMap<NodeId, Vec<Badge>>,
    pub width: u16,
    pub height: u16,
}

impl Layout {
    #[must_use]
    pub fn card(&self, node: NodeId) -> Option<&CardPos> {
        self.cards.iter().find(|c| c.node == node)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    pub card_w: u16,
    /// Rows per card, borders included. Odd, so there is a middle row; the
    /// last content row holds tag badges.
    pub card_h: u16,
}

pub(crate) const MIN_GAP: u16 = 5;
pub(crate) const MAX_SWEEPS: usize = 8;
/// Column orders drawn in full, after ranking by the preview.
const FULL_BUILDS: usize = 2;
/// The adjacent-swap search previews every pair of neighbouring cards; past
/// this many segments a preview costs too much for the 50 ms budget.
const SWAP_SEARCH_MAX_SEGMENTS: usize = 64;
/// Passes of the adjacent-swap search.
const SWAP_PASSES: usize = 2;

pub(crate) fn i(x: usize) -> i32 {
    i32::try_from(x).unwrap_or(i32::MAX)
}

/// Colour per edge: sorted tag combination, first-encounter order.
#[must_use]
pub fn colours(g: &ViewGraph) -> Vec<Option<EdgeColour>> {
    let mut combos: HashMap<String, u8> = HashMap::new();
    g.edges
        .iter()
        .map(|e| {
            if e.tags.is_empty() {
                return None;
            }
            let mut key: Vec<&str> = e.tags.iter().map(String::as_str).collect();
            key.sort_unstable();
            let key = key.join(",");
            let n = combos.len();
            let idx = *combos
                .entry(key)
                .or_insert_with(|| u8::try_from(n % usize::from(PALETTE_SIZE)).unwrap_or(0));
            Some(EdgeColour(idx))
        })
        .collect()
}

/// What one column order is worth: the vocabulary tier, then the total,
/// then crossings. Ties fall to whichever came first.
type Key = ([usize; 5], i64, usize);

fn key(s: &Score) -> Key {
    (s.vocabulary(), s.total, s.crossings)
}

/// Lay the graph out. Deterministic for a given `(g, opts)`.
///
/// Every distinct card order the barycenter sweeps propose is drawn with the
/// centred placement and scored; the best is refined by swapping neighbouring
/// cards while the preview improves; then that order and the best sweeps are
/// built in full, placement search included, and the lowest full score wins.
#[must_use]
pub fn layout(g: &ViewGraph, opts: LayoutOptions) -> Layout {
    let edge_colour = colours(g);
    let ranked = rank::rank(g);
    let badges = badges_for(g, &edge_colour);
    let mut layered = order::layer(g, &ranked);
    let orderings = order::orderings(&mut layered);
    let swap_search = layered.segments.len() <= SWAP_SEARCH_MAX_SEGMENTS;
    // Every preview drawn, so a candidate's first placement is not drawn twice.
    let mut previews: Vec<(Vec<Vec<order::LNode>>, Layout, Score)> = Vec::new();
    let mut preview = |cols: &[Vec<order::LNode>]| {
        layered.columns = cols.to_vec();
        let (l, s) = route::preview(g, &ranked, &layered, &edge_colour, &badges, opts);
        let k = key(&s);
        previews.push((cols.to_vec(), l, s));
        k
    };
    let candidates: Vec<Vec<Vec<order::LNode>>> = if orderings.len() == 1 && !swap_search {
        orderings
    } else {
        let mut keyed: Vec<(Key, usize)> = orderings
            .iter()
            .enumerate()
            .map(|(i, cols)| (preview(cols), i))
            .collect();
        keyed.sort_unstable();
        let mut out: Vec<Vec<Vec<order::LNode>>> = keyed
            .iter()
            .take(FULL_BUILDS)
            .map(|&(_, i)| orderings[i].clone())
            .collect();
        if swap_search {
            let (mut best_key, i) = keyed[0];
            let mut cols = orderings[i].clone();
            for _ in 0..SWAP_PASSES {
                let mut improved = false;
                for (c, a, b) in order::card_pairs(&cols) {
                    cols[c].swap(a, b);
                    let k = preview(&cols);
                    if k < best_key {
                        best_key = k;
                        improved = true;
                    } else {
                        cols[c].swap(a, b);
                    }
                }
                if !improved {
                    break;
                }
            }
            // Last, so it only replaces a sweep when the full score says so.
            if !out.contains(&cols) {
                out.push(cols);
            }
        }
        out
    };
    let mut best: Option<(Score, Layout, Vec<Vec<order::LNode>>)> = None;
    for cols in candidates {
        let drawn = previews.iter().position(|(c, _, _)| *c == cols).map(|i| {
            let (_, l, s) = previews.swap_remove(i);
            (l, s)
        });
        layered.columns = cols;
        let (layout, s) = route::build(
            g,
            &ranked,
            &layered,
            &edge_colour,
            &badges,
            opts,
            false,
            drawn,
        );
        if best.as_ref().is_none_or(|(bs, _, _)| key(&s) < key(bs)) {
            best = Some((s, layout, layered.columns.clone()));
        }
    }
    // `candidates` always holds the first pass, so the fallback never runs.
    let Some((s, layout, cols)) = best else {
        return route::build(
            g,
            &ranked,
            &layered,
            &edge_colour,
            &badges,
            opts,
            false,
            None,
        )
        .0;
    };
    polish(
        g,
        &ranked,
        &mut layered,
        &edge_colour,
        &badges,
        opts,
        (s, layout, cols),
    )
}

/// The winner once more with pass slots held open between stacked cards:
/// where its order puts them, then where the drawn rows say they belong.
/// Kept when that lowers the total without raising either tier, or the
/// height past the slots' allowance. Then the winner with each fan centred
/// on its extremes.
fn polish(
    g: &ViewGraph,
    ranked: &rank::Ranked,
    layered: &mut order::Layered,
    edge_colour: &[Option<EdgeColour>],
    badges: &HashMap<NodeId, Vec<Badge>>,
    opts: LayoutOptions,
    best: (Score, Layout, Vec<Vec<order::LNode>>),
) -> Layout {
    let (s, layout, cols) = best;
    layered.columns.clone_from(&cols);
    let reslotted = route::reslot(g, layered, &layout);
    let mut best = (s, layout, cols, false);
    let mut tried: Vec<Vec<Vec<order::LNode>>> = Vec::new();
    for cols in [layered.columns.clone(), reslotted] {
        if tried.contains(&cols) || !route::has_slots(&cols) {
            continue;
        }
        tried.push(cols.clone());
        layered.columns = cols;
        let (slotted, s2) = route::build(g, ranked, layered, edge_colour, badges, opts, true, None);
        if route::no_worse(&s2, &best.0)
            && s2.total < best.0.total
            && i32::from(s2.height) <= i32::from(best.0.height) + route::SLOT_RISE
        {
            best = (s2, slotted, layered.columns.clone(), true);
        }
    }
    let (s, layout, cols, slots) = best;
    layered.columns = cols;
    route::refine(
        g,
        ranked,
        layered,
        edge_colour,
        badges,
        opts,
        slots,
        (layout, s),
    )
    .0
}

fn badges_for(g: &ViewGraph, edge_colour: &[Option<EdgeColour>]) -> HashMap<NodeId, Vec<Badge>> {
    let mut tagged_in: HashMap<NodeId, Vec<(String, Option<EdgeColour>)>> = HashMap::new();
    for (i, e) in g.edges.iter().enumerate() {
        if !e.tags.is_empty() {
            tagged_in
                .entry(e.to)
                .or_default()
                .push((e.tags.join(", "), edge_colour[i]));
        }
    }
    tagged_in
        .into_iter()
        .map(|(node, v)| {
            let mut seen = Vec::<Badge>::new();
            for (label, colour) in v {
                if !seen.iter().any(|b| b.label == label) {
                    seen.push(Badge { label, colour });
                }
            }
            (node, seen)
        })
        .collect()
}
