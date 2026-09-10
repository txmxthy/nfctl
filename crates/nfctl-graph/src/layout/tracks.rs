//! Pack the vertical runs in one gap onto tracks: two runs share a track when
//! they do not overlap, or when they share a source or a target, so a fan-out
//! is one bus with a junction per branch. Then order the
//! tracks left to right so no horizontal lands on another run's corner and
//! horizontals cross as few verticals as possible.

use super::{EdgeColour, EdgeId};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Span {
    pub edge: EdgeId,
    /// Rows covered by the vertical run.
    pub lo: i32,
    pub hi: i32,
    /// Row of the horizontal arriving from the left, and leaving to the right.
    pub y_in: i32,
    pub y_out: i32,
    pub src: u32,
    pub dst: u32,
    /// The edge's colour, so a bus can be split by colour when asked.
    pub colour: Option<EdgeColour>,
}

/// Returns, per span (same order as input), the track index; and the track
/// count. A straight span (no vertical run) takes no track and reports 0.
pub(crate) fn pack(spans: &[Span], by_colour: bool) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..spans.len())
        .filter(|&i| spans[i].lo != spans[i].hi)
        .collect();
    order.sort_by_key(|&i| (spans[i].lo, spans[i].hi, spans[i].edge));
    let mut tracks: Vec<Vec<Span>> = Vec::new();
    let mut assign = vec![0usize; spans.len()];
    for i in order {
        let s = spans[i];
        let compatible = |t: &Vec<Span>| {
            t.iter().all(|o| {
                // One bus per source (and per target): every branch leaves
                // the same column through its own junction. With `by_colour`
                // a bus is split so each colour runs on its own, since a cell
                // can only hold one colour and a shared bus would lose them.
                let related =
                    (o.src == s.src || o.dst == s.dst) && (!by_colour || o.colour == s.colour);
                let disjoint = s.hi + 1 < o.lo || o.hi + 1 < s.lo;
                related || disjoint
            })
        };
        // A track already carrying this run's bus before any other: first fit
        // alone would open a new track for a long run and then drop a short
        // one of the same colour and target into an older track, drawing one
        // line as two a column apart.
        let joins = |t: &Vec<Span>| {
            compatible(t)
                && t.iter()
                    .any(|o| (o.src == s.src || o.dst == s.dst) && o.colour == s.colour)
        };
        let slot = tracks
            .iter()
            .position(joins)
            .or_else(|| tracks.iter().position(compatible))
            .unwrap_or_else(|| {
                tracks.push(Vec::new());
                tracks.len() - 1
            });
        tracks[slot].push(s);
        assign[i] = slot;
    }
    let perm = order_tracks(&tracks);
    let rank: Vec<usize> = {
        let mut r = vec![0; perm.len()];
        for (pos, &t) in perm.iter().enumerate() {
            r[t] = pos;
        }
        r
    };
    (
        assign
            .into_iter()
            .map(|t| rank.get(t).copied().unwrap_or(0))
            .collect(),
        tracks.len(),
    )
}

/// Cost of drawing track `a` left of track `b`: horizontals landing on
/// another run's corner row (an overlap: it would read as one line), then
/// horizontals crossing a vertical.
fn cost(a: &[Span], b: &[Span]) -> (u32, u32) {
    let (mut landings, mut crossings) = (0, 0);
    for x in a {
        for y in b {
            // y's entry horizontal runs across x's track.
            if x.lo < y.y_in && y.y_in < x.hi {
                crossings += 1;
            }
            // x's exit horizontal runs across y's track.
            if y.lo < x.y_out && x.y_out < y.hi {
                crossings += 1;
            }
            if x.lo < x.hi && x.y_out == y.y_in {
                landings += 1;
            }
        }
    }
    (landings, crossings)
}

fn total(tracks: &[Vec<Span>], perm: &[usize]) -> (u32, u32) {
    let mut sum = (0, 0);
    for (i, &a) in perm.iter().enumerate() {
        for &b in &perm[i + 1..] {
            let c = cost(&tracks[a], &tracks[b]);
            sum = (sum.0 + c.0, sum.1 + c.1);
        }
    }
    sum
}

/// Gaps with this many tracks or fewer try every order.
const EXACT: usize = 6;

/// Order the tracks left to right: no landing on another run's corner where
/// any order avoids it, then the fewest crossings (a linear ordering
/// problem). Every permutation for small gaps, an adjacent-swap descent
/// beyond; ties keep the earlier order.
fn order_tracks(tracks: &[Vec<Span>]) -> Vec<usize> {
    let n = tracks.len();
    let mut perm: Vec<usize> = (0..n).collect();
    if n <= EXACT {
        let mut best = (total(tracks, &perm), perm.clone());
        while next_permutation(&mut perm) {
            let c = total(tracks, &perm);
            if c < best.0 {
                best = (c, perm.clone());
            }
        }
        return best.1;
    }
    let mut improved = true;
    while improved {
        improved = false;
        for i in 0..n.saturating_sub(1) {
            let (a, b) = (perm[i], perm[i + 1]);
            if cost(&tracks[b], &tracks[a]) < cost(&tracks[a], &tracks[b]) {
                perm.swap(i, i + 1);
                improved = true;
            }
        }
    }
    perm
}

/// Advance to the next permutation in lexicographic order; false at the last.
fn next_permutation(p: &mut [usize]) -> bool {
    let Some(i) = (1..p.len()).rev().find(|&i| p[i - 1] < p[i]) else {
        return false;
    };
    let Some(j) = (i..p.len()).rev().find(|&j| p[j] > p[i - 1]) else {
        return false;
    };
    p.swap(i - 1, j);
    p[i..].reverse();
    true
}
