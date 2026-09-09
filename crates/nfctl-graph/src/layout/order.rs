//! Layered graph with dummy pass nodes; barycenter sweeps propose column
//! orders, `layout` picks one by the drawn result.

use super::rank::Ranked;
use super::{EdgeId, MAX_SWEEPS, NodeId, ViewGraph, i};

/// A node in the layered graph: a real node or a pass slot for a long edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LNode {
    Real(NodeId),
    Pass(EdgeId),
}

/// One column hop of a forward edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Segment {
    pub edge: EdgeId,
    pub col: usize, // column of `from`; `to` is in col + 1
    pub from: LNode,
    pub to: LNode,
}

pub(crate) struct Layered {
    pub columns: Vec<Vec<LNode>>,
    pub segments: Vec<Segment>,
}

pub(crate) fn layer(g: &ViewGraph, r: &Ranked) -> Layered {
    let cols = r.columns();
    let mut columns: Vec<Vec<LNode>> = vec![Vec::new(); cols];
    for (i, _) in g.nodes.iter().enumerate() {
        columns[r.rank[i]].push(LNode::Real(NodeId(u32::try_from(i).unwrap_or(u32::MAX))));
    }
    let mut segments = Vec::new();
    for (ei, e) in g.edges.iter().enumerate() {
        if r.back[ei] {
            continue;
        }
        let (a, b) = (r.rank[e.from.0 as usize], r.rank[e.to.0 as usize]);
        let id = EdgeId(u32::try_from(ei).unwrap_or(u32::MAX));
        let mut prev = LNode::Real(e.from);
        for c in a..b {
            let next = if c + 1 == b {
                LNode::Real(e.to)
            } else {
                LNode::Pass(id)
            };
            if let LNode::Pass(_) = next {
                columns[c + 1].push(next);
            }
            segments.push(Segment {
                edge: id,
                col: c,
                from: prev,
                to: next,
            });
            prev = next;
        }
    }
    Layered { columns, segments }
}

fn position(columns: &[Vec<LNode>], col: usize, n: LNode) -> Option<usize> {
    columns[col].iter().position(|&x| x == n)
}

/// Sort column `col` by the mean position of its neighbours in `other`
/// (previous column when `down`, next when not). Nodes without neighbours keep
/// their index; a pass slot ties after a card.
fn sweep_column(l: &mut Layered, col: usize, down: bool) {
    let other = if down { col - 1 } else { col + 1 };
    let mut keyed: Vec<(Bary, usize, LNode)> = l.columns[col]
        .iter()
        .enumerate()
        .map(|(idx, &n)| {
            let neigh: Vec<usize> = l
                .segments
                .iter()
                .filter(|s| {
                    if down {
                        s.col == other && s.to == n
                    } else {
                        s.col == col && s.from == n
                    }
                })
                .filter_map(|s| position(&l.columns, other, if down { s.from } else { s.to }))
                .collect();
            // Mean neighbour position scaled by the LCM-free trick: keep a rational.
            let key = if neigh.is_empty() {
                Bary {
                    num: i(idx),
                    den: 1,
                }
            } else {
                Bary {
                    num: i(neigh.iter().sum::<usize>()),
                    den: i(neigh.len()),
                }
            };
            (key, idx, n)
        })
        .collect();
    keyed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| matches!(a.2, LNode::Pass(_)).cmp(&matches!(b.2, LNode::Pass(_))))
            .then_with(|| a.1.cmp(&b.1))
    });
    l.columns[col] = keyed.into_iter().map(|(_, _, n)| n).collect();
}

/// The cards of every column, in order: what the geometry depends on. Pass
/// slots take their rows from the edge, wherever they sit in the column.
pub(crate) fn cards(columns: &[Vec<LNode>]) -> Vec<Vec<NodeId>> {
    columns
        .iter()
        .map(|col| {
            col.iter()
                .filter_map(|n| match n {
                    LNode::Real(id) => Some(*id),
                    LNode::Pass(_) => None,
                })
                .collect()
        })
        .collect()
}

/// Candidate column orders: the first downward pass, then every alternating
/// barycenter sweep that puts the cards in an order not seen before. The
/// caller draws each and keeps the best; the order here is the tie-break.
pub(crate) fn orderings(l: &mut Layered) -> Vec<Vec<Vec<LNode>>> {
    let cols = l.columns.len();
    if cols < 2 {
        return vec![l.columns.clone()];
    }
    let mut out: Vec<Vec<Vec<LNode>>> = Vec::new();
    let mut seen: Vec<Vec<Vec<NodeId>>> = Vec::new();
    let mut keep = |l: &Layered, out: &mut Vec<Vec<Vec<LNode>>>| {
        let key = cards(&l.columns);
        if !seen.contains(&key) {
            seen.push(key);
            out.push(l.columns.clone());
        }
    };
    for c in 1..cols {
        sweep_column(l, c, true);
    }
    keep(l, &mut out);
    for sweep in 0..MAX_SWEEPS {
        if sweep % 2 == 0 {
            for c in 1..cols {
                sweep_column(l, c, true);
            }
        } else {
            for c in (0..cols - 1).rev() {
                sweep_column(l, c, false);
            }
        }
        keep(l, &mut out);
    }
    out
}

/// `(column, index, index)` of every pair of cards adjacent in a column,
/// pass slots between them ignored.
pub(crate) fn card_pairs(columns: &[Vec<LNode>]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (c, col) in columns.iter().enumerate() {
        let at: Vec<usize> = (0..col.len())
            .filter(|&k| matches!(col[k], LNode::Real(_)))
            .collect();
        out.extend(at.windows(2).map(|w| (c, w[0], w[1])));
    }
    out
}

/// Barycenter as a rational so ordering is exact and deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bary {
    num: i32,
    den: i32,
}

impl Ord for Bary {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (i64::from(self.num) * i64::from(o.den)).cmp(&(i64::from(o.num) * i64::from(self.den)))
    }
}

impl PartialOrd for Bary {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
