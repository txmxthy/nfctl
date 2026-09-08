//! Pack the vertical runs in one gap onto tracks: two runs share a track when
//! they do not overlap, or when they share a source or a target *and* a
//! colour, so a tagged fan-out leaves as parallel coloured lines rather than
//! one grey trunk. Then order the
//! tracks left to right so horizontals cross as few verticals as possible.

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
    pub colour: Option<EdgeColour>,
    pub src: u32,
    pub dst: u32,
}

/// Returns, per span (same order as input), the track index; and the track count.
pub(crate) fn pack(spans: &[Span]) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].lo, spans[i].hi, spans[i].edge));
    let mut tracks: Vec<Vec<Span>> = Vec::new();
    let mut assign = vec![0usize; spans.len()];
    for i in order {
        let s = spans[i];
        let compatible = |t: &Vec<Span>| {
            t.iter().all(|o| {
                ((o.src == s.src || o.dst == s.dst) && o.colour == s.colour)
                    || s.hi + 1 < o.lo
                    || o.hi + 1 < s.lo
            })
        };
        let slot = tracks.iter().position(compatible).unwrap_or_else(|| {
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
    (assign.into_iter().map(|t| rank[t]).collect(), tracks.len())
}

/// Cost of drawing track `a` left of track `b`: one per horizontal crossing a
/// vertical, ten when a horizontal lands on another run's corner row (it would
/// read as one line).
fn cost(a: &[Span], b: &[Span]) -> u32 {
    let mut c = 0;
    for x in a {
        for y in b {
            // y's entry horizontal runs across x's track.
            if x.lo < y.y_in && y.y_in < x.hi {
                c += 1;
            }
            // x's exit horizontal runs across y's track.
            if y.lo < x.y_out && x.y_out < y.hi {
                c += 1;
            }
            if x.lo < x.hi && x.y_out == y.y_in {
                c += 10;
            }
        }
    }
    c
}

/// Adjacent-swap descent over the pairwise cost matrix (a linear ordering problem).
fn order_tracks(tracks: &[Vec<Span>]) -> Vec<usize> {
    let n = tracks.len();
    let mut perm: Vec<usize> = (0..n).collect();
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
