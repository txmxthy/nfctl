//! Numbers for what the eye judges: how many bends an edge has, where edges
//! share cells and whether that reads as a fork, a join, a crossing or an
//! accident, and how symmetric fan-outs and fan-ins are. Computed from the
//! layout alone by rasterising routes the way the painter does, so a test can
//! guard the drawing without a terminal.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::{EdgeId, Layout, NodeId, Route, ViewGraph};

const L: u8 = 1;
const R: u8 = 2;
const U: u8 = 4;
const D: u8 = 8;

/// Per-edge facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeScore {
    pub edge: EdgeId,
    pub back: bool,
    /// Columns crossed: 1 for neighbours, more for a skip.
    pub span: usize,
    /// Interior polyline points: 0 straight, 1 an L, 2 a Z or S.
    pub bends: usize,
    /// Bends this edge may have: 2 between neighbours, 4 for a skip (a Z at
    /// each end with a straight run across the columns it passes), 4 for a
    /// back edge.
    pub allowed: usize,
    /// Distinct runs of cells shared with another edge from the same source.
    pub forks: usize,
    /// Distinct runs of cells shared with another edge into the same target.
    pub joins: usize,
    /// Cells where this edge and an unrelated one pass straight through each other.
    pub crossings: usize,
    /// Cells shared with an unrelated edge that are not a clean crossing.
    pub overlaps: usize,
    /// Vertical cells travelled beyond the row difference (forward edges).
    pub detour: i32,
}

/// The layout's score. Lower is better everywhere; `total` is the weighted sum
/// the loop optimises, the rest are what a reader looks at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Score {
    pub edges: Vec<EdgeScore>,
    /// Forward edges between neighbouring columns with more than two bends.
    pub bends_over_fwd: usize,
    /// Skip edges with more than four bends.
    pub bends_over_skip: usize,
    /// Back edges with more than four bends.
    pub bends_over_back: usize,
    /// Edges with more than one fork run or more than one join run.
    pub junction_over: usize,
    pub overlaps: usize,
    pub crossings: usize,
    /// Σ over fan-outs and fan-ins of |rows above − rows below| the bus row.
    pub asymmetry: i32,
    pub detour: i32,
    /// Cells where two colours meet and the painter has to pick one.
    pub mixed_cells: usize,
    /// Widest gap minus narrowest gap.
    pub gap_spread: u16,
    /// Blank rows left between the runs that cross a column, summed over the
    /// columns. Zero when every set of parallel runs is one solid ribbon.
    #[serde(default)]
    pub scatter: usize,
    pub height: u16,
    pub width: u16,
    pub total: i64,
}

impl Score {
    /// The vocabulary tier: what must reach zero and stay there.
    #[must_use]
    pub fn vocabulary(&self) -> [usize; 5] {
        [
            self.bends_over_fwd,
            self.bends_over_skip,
            self.bends_over_back,
            self.junction_over,
            self.overlaps,
        ]
    }

    /// The soft tier: pushed down, never locked.
    #[must_use]
    pub fn soft(&self) -> i64 {
        i64::try_from(self.crossings).unwrap_or(i64::MAX / 4)
            + i64::from(self.asymmetry)
            + i64::from(self.detour)
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bends>2 {:>2}  skip>4 {:>2}  back>4 {:>2}  junc>1 {:>2}  overlap {:>3}  cross {:>3}  asym {:>3}  detour {:>3}  mixed {:>3}  gap± {:>2}  scatter {:>3}  {}x{}  total {:>4}",
            self.bends_over_fwd,
            self.bends_over_skip,
            self.bends_over_back,
            self.junction_over,
            self.overlaps,
            self.crossings,
            self.asymmetry,
            self.detour,
            self.mixed_cells,
            self.gap_spread,
            self.scatter,
            self.width,
            self.height,
            self.total
        )
    }
}

/// One rasterised run of an edge.
#[derive(Debug, Clone, Copy)]
struct Cell {
    edge: usize,
    bits: u8,
    /// Vertical run length through this cell (0 for horizontals): the painter's reach.
    reach: i32,
}

fn raster(routes: &[Route]) -> HashMap<(i32, i32), Vec<Cell>> {
    let mut cells: HashMap<(i32, i32), Vec<Cell>> = HashMap::new();
    let mut put = |edge: usize, x: i32, y: i32, bits: u8, reach: i32| {
        let v = cells.entry((x, y)).or_default();
        if let Some(c) = v.iter_mut().find(|c| c.edge == edge) {
            c.bits |= bits;
            c.reach = c.reach.max(reach);
        } else {
            v.push(Cell { edge, bits, reach });
        }
    };
    for (ei, r) in routes.iter().enumerate() {
        for w in r.polyline.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if y0 == y1 {
                let (a, b) = (x0.min(x1), x0.max(x1));
                for x in a..=b {
                    let bits = if x > a { L } else { 0 } | if x < b { R } else { 0 };
                    put(ei, x, y0, bits, 0);
                }
            } else {
                let (a, b) = (y0.min(y1), y0.max(y1));
                for y in a..=b {
                    let bits = if y > a { U } else { 0 } | if y < b { D } else { 0 };
                    put(ei, x0, y, bits, b - a + 1);
                }
            }
        }
    }
    cells
}

fn is_straight_h(bits: u8) -> bool {
    bits == (L | R)
}

fn is_straight_v(bits: u8) -> bool {
    bits == (U | D)
}

/// Number of 4-connected components in a set of cells.
fn components(cells: &BTreeSet<(i32, i32)>) -> usize {
    let mut seen = BTreeSet::new();
    let mut n = 0;
    for &start in cells {
        if !seen.insert(start) {
            continue;
        }
        n += 1;
        let mut stack = vec![start];
        while let Some((x, y)) = stack.pop() {
            for next in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if cells.contains(&next) && seen.insert(next) {
                    stack.push(next);
                }
            }
        }
    }
    n
}

/// Blank rows between the runs crossing each column: how far the parallel
/// runs are from being one solid ribbon. Rows a card covers do not count.
/// Reporting only, so it is not on the path the layout search runs; use
/// [`score_full`] when the number is wanted.
#[must_use]
pub fn scatter(l: &Layout) -> usize {
    let Some(card_w) = l
        .gaps
        .first()
        .map(|g| g.x0 - l.col_x.first().copied().unwrap_or(0))
    else {
        return 0;
    };
    let mut total = 0;
    for (c, &x0) in l.col_x.iter().enumerate() {
        let x1 = x0 + card_w;
        let mut rows: BTreeSet<i32> = BTreeSet::new();
        for r in &l.routes {
            for w in r.polyline.windows(2) {
                let (a, b) = (w[0], w[1]);
                if a.1 == b.1 && a.0.min(b.0) <= x0 && a.0.max(b.0) >= x1 {
                    rows.insert(a.1);
                }
            }
        }
        let (Some(&lo), Some(&hi)) = (rows.iter().next(), rows.iter().next_back()) else {
            continue;
        };
        for row in lo..=hi {
            let blocked = l
                .cards
                .iter()
                .any(|k| k.col == c && row >= k.y - 1 && row <= k.y + i32::from(k.h));
            if !rows.contains(&row) && !blocked {
                total += 1;
            }
        }
    }
    total
}

/// Score a layout of `g`, reporting metrics included. The layout search calls
/// [`score`] instead, which leaves the reporting-only ones out.
#[must_use]
pub fn score_full(g: &ViewGraph, l: &Layout) -> Score {
    Score {
        scatter: scatter(l),
        ..score(g, l)
    }
}

/// Score a layout of `g`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn score(g: &ViewGraph, l: &Layout) -> Score {
    let n_edges = g.edges.len();
    let cells = raster(&l.routes);
    let same_src = |a: usize, b: usize| g.edges[a].from == g.edges[b].from;
    let same_dst = |a: usize, b: usize| g.edges[a].to == g.edges[b].to;
    let span_of = |e: usize| -> (bool, usize) {
        let (Some(from), Some(to)) = (l.card(g.edges[e].from), l.card(g.edges[e].to)) else {
            return (false, 1);
        };
        (to.col <= from.col, to.col.abs_diff(from.col).max(1))
    };

    let mut fork_cells: Vec<BTreeSet<(i32, i32)>> = vec![BTreeSet::new(); n_edges];
    let mut join_cells: Vec<BTreeSet<(i32, i32)>> = vec![BTreeSet::new(); n_edges];
    let mut crossings = vec![0usize; n_edges];
    let mut overlaps = vec![0usize; n_edges];
    let mut mixed_cells = 0usize;
    let mut sorted: Vec<(&(i32, i32), &Vec<Cell>)> = cells.iter().collect();
    sorted.sort_by_key(|(k, _)| **k);
    for (&pos, v) in sorted {
        if v.len() < 2 {
            continue;
        }
        let colours: BTreeSet<_> = v.iter().map(|c| l.routes[c.edge].colour).collect();
        if colours.len() > 1 {
            mixed_cells += 1;
        }
        for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                let (a, b) = (v[i], v[j]);
                let crossing = (is_straight_h(a.bits) && is_straight_v(b.bits))
                    || (is_straight_v(a.bits) && is_straight_h(b.bits));
                if crossing {
                    crossings[a.edge] += 1;
                    crossings[b.edge] += 1;
                } else if same_src(a.edge, b.edge) {
                    fork_cells[a.edge].insert(pos);
                    fork_cells[b.edge].insert(pos);
                } else if same_dst(a.edge, b.edge) {
                    join_cells[a.edge].insert(pos);
                    join_cells[b.edge].insert(pos);
                } else {
                    overlaps[a.edge] += 1;
                    overlaps[b.edge] += 1;
                }
            }
        }
    }

    let mut edges = Vec::with_capacity(n_edges);
    for (ei, r) in l.routes.iter().enumerate() {
        let (back, span) = span_of(ei);
        let bends = r.polyline.len().saturating_sub(2);
        let allowed = if back || span > 1 { 4 } else { 2 };
        let travelled: i32 = r
            .polyline
            .windows(2)
            .filter(|w| w[0].0 == w[1].0)
            .map(|w| (w[1].1 - w[0].1).abs())
            .sum();
        let (y0, y1) = (
            r.polyline.first().map_or(0, |p| p.1),
            r.polyline.last().map_or(0, |p| p.1),
        );
        let detour = if back { 0 } else { travelled - (y1 - y0).abs() };
        edges.push(EdgeScore {
            edge: EdgeId(u32::try_from(ei).unwrap_or(u32::MAX)),
            back,
            span,
            bends,
            allowed,
            forks: components(&fork_cells[ei]),
            joins: components(&join_cells[ei]),
            crossings: crossings[ei],
            overlaps: overlaps[ei],
            detour,
        });
    }

    // Symmetry of every fan-out and fan-in over forward edges.
    let mut outs: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
    let mut ins: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
    for (ei, e) in g.edges.iter().enumerate() {
        if edges[ei].back {
            continue;
        }
        outs.entry(e.from).or_default().push(ei);
        ins.entry(e.to).or_default().push(ei);
    }
    let after_first_vertical = |r: &Route| {
        r.polyline
            .windows(2)
            .find(|w| w[0].0 == w[1].0)
            .map_or_else(|| r.polyline.first().map_or(0, |p| p.1), |w| w[1].1)
    };
    let before_last_vertical = |r: &Route| {
        r.polyline
            .windows(2)
            .rev()
            .find(|w| w[0].0 == w[1].0)
            .map_or_else(|| r.polyline.last().map_or(0, |p| p.1), |w| w[0].1)
    };
    let fan = |members: &Vec<usize>, bus_row: i32, side: &dyn Fn(&Route) -> i32| -> i32 {
        if members.len() < 2 {
            return 0;
        }
        let rows: Vec<i32> = members.iter().map(|&e| side(&l.routes[e])).collect();
        let above = bus_row - rows.iter().copied().min().unwrap_or(bus_row);
        let below = rows.iter().copied().max().unwrap_or(bus_row) - bus_row;
        (above - below).abs()
    };
    let mut asymmetry = 0;
    for (node, members) in &outs {
        let Some(card) = l.card(*node) else { continue };
        let bus_row = card.y + i32::from(card.h) / 2;
        asymmetry += fan(members, bus_row, &after_first_vertical);
    }
    for (node, members) in &ins {
        let Some(card) = l.card(*node) else { continue };
        let bus_row = card.y + i32::from(card.h) / 2;
        asymmetry += fan(members, bus_row, &before_last_vertical);
    }

    let over = |e: &EdgeScore| e.bends > e.allowed;
    let bends_over_fwd = edges
        .iter()
        .filter(|e| !e.back && e.span == 1 && over(e))
        .count();
    let bends_over_skip = edges
        .iter()
        .filter(|e| !e.back && e.span > 1 && over(e))
        .count();
    let bends_over_back = edges.iter().filter(|e| e.back && over(e)).count();
    let junction_over = edges.iter().filter(|e| e.forks > 1 || e.joins > 1).count();
    let overlaps: usize = edges.iter().map(|e| e.overlaps).sum::<usize>() / 2;
    let crossings: usize = edges.iter().map(|e| e.crossings).sum::<usize>() / 2;
    let detour: i32 = edges.iter().map(|e| e.detour).sum();
    let gap_spread = l
        .gaps
        .iter()
        .map(|g| g.width)
        .max()
        .unwrap_or(0)
        .saturating_sub(l.gaps.iter().map(|g| g.width).min().unwrap_or(0));
    let total = 10 * i64::try_from(bends_over_fwd + bends_over_skip + bends_over_back).unwrap_or(0)
        + 10 * i64::try_from(junction_over).unwrap_or(0)
        + 5 * i64::try_from(overlaps).unwrap_or(0)
        + 3 * i64::try_from(crossings).unwrap_or(0)
        + 2 * i64::from(asymmetry)
        + i64::from(detour);
    Score {
        edges,
        bends_over_fwd,
        bends_over_skip,
        bends_over_back,
        junction_over,
        overlaps,
        crossings,
        asymmetry,
        detour,
        mixed_cells,
        gap_spread,
        scatter: 0,
        height: l.height,
        width: l.width,
        total,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::from_mermaid;
    use crate::layout::{LayoutOptions, layout};

    const OPTS: LayoutOptions = LayoutOptions {
        bundling: crate::layout::Bundling::Spread,
        card_w: 18,
        card_h: 5,
    };

    fn scored(body: &str) -> Score {
        let src = format!("graph LR\n subgraph p[\"p\"]\n{body}\n end\n");
        let t = from_mermaid(&src).unwrap().remove(0).1;
        let g = ViewGraph::expanded(&t);
        score(&g, &layout(&g, OPTS))
    }

    #[test]
    fn linear_chain_is_perfect() {
        let s = scored("a([a]) --> b[b]\n b --> c[[c]]");
        assert_eq!(s.vocabulary(), [0, 0, 0, 0, 0]);
        assert_eq!(
            (s.crossings, s.asymmetry, s.detour, s.mixed_cells),
            (0, 0, 0, 0)
        );
        assert!(s.edges.iter().all(|e| e.bends == 0));
        assert_eq!(s.total, 0);
    }

    #[test]
    fn symmetric_fan_out_scores_zero_asymmetry() {
        let s = scored("s([s]) -->|a| x[x]\n s -->|b| y[y]\n s -->|c| z[z]");
        assert_eq!(s.asymmetry, 0);
        // A branch is a Z: out, along the bus, in.
        assert!(s.edges.iter().all(|e| e.bends <= 2));
        assert_eq!(s.junction_over, 0);
        assert!(s.edges.iter().all(|e| e.forks <= 1));
        assert_eq!(s.overlaps, 0);
        // The bus carries three colours through the shared stub and junction.
        assert!(s.mixed_cells > 0);
    }

    #[test]
    fn fan_in_is_one_join_per_edge() {
        let s = scored("a([a]) --> k[[k]]\n b([b]) --> k\n c([c]) --> k");
        assert!(s.edges.iter().all(|e| e.joins <= 1 && e.forks == 0));
        assert_eq!(s.overlaps, 0);
    }

    #[test]
    fn skip_edge_is_a_z_at_worst_and_back_edge_is_counted_apart() {
        let s = scored("s([s]) --> m[m]\n m --> t[[t]]\n s --> t");
        let skip = s.edges.iter().find(|e| e.span == 2).unwrap();
        assert_eq!(skip.allowed, 4);
        assert!(skip.bends <= 4, "{skip:?}");
        assert_eq!(s.bends_over_fwd + s.bends_over_skip, 0);
        let s = scored("s([s]) --> a[a]\n a --> b[b]\n b --> a\n b --> t[[t]]");
        let back: Vec<_> = s.edges.iter().filter(|e| e.back).collect();
        assert_eq!(back.len(), 1);
        assert!(back[0].bends <= 4);
        assert_eq!(s.bends_over_back, 0);
    }

    #[test]
    fn deterministic() {
        let body = "a([a]) --> y[y]\n b([b]) --> x[x]\n a --> x\n b --> y\n x --> e[[e]]\n y --> e";
        assert_eq!(scored(body), scored(body));
    }
}
