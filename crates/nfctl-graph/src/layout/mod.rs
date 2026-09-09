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
    let mut preview = |cols: &[Vec<order::LNode>]| {
        layered.columns = cols.to_vec();
        key(&route::preview(
            g,
            &ranked,
            &layered,
            &edge_colour,
            &badges,
            opts,
        ))
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
    let mut best: Option<(Key, Layout)> = None;
    for cols in candidates {
        layered.columns = cols;
        let (layout, s) = route::build(g, &ranked, &layered, &edge_colour, &badges, opts);
        let k = key(&s);
        if best.as_ref().is_none_or(|(bk, _)| k < *bk) {
            best = Some((k, layout));
        }
    }
    // `candidates` always holds the first pass, so the fallback never runs.
    best.map_or_else(
        || route::build(g, &ranked, &layered, &edge_colour, &badges, opts).0,
        |(_, l)| l,
    )
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
