//! Columns by longest path from the sources, with back edges taken out first.

use super::ViewGraph;

pub(crate) struct Ranked {
    pub rank: Vec<usize>,
    /// Edge indices that point backwards (cycles, self-loops).
    pub back: Vec<bool>,
}

pub(crate) fn rank(g: &ViewGraph) -> Ranked {
    let n = g.nodes.len();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for (i, e) in g.edges.iter().enumerate() {
        out[e.from.0 as usize].push(i);
        indeg[e.to.0 as usize] += 1;
    }
    // DFS colouring from the roots, in node order; grey targets mark back edges.
    let mut colour = vec![0u8; n]; // 0 white, 1 grey, 2 black
    let mut back = vec![false; g.edges.len()];
    let roots: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).chain(0..n).collect();
    for r in roots {
        if colour[r] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(r, 0)];
        colour[r] = 1;
        while let Some(&mut (u, ref mut next)) = stack.last_mut() {
            if *next < out[u].len() {
                let ei = out[u][*next];
                *next += 1;
                let v = g.edges[ei].to.0 as usize;
                match colour[v] {
                    1 => back[ei] = true,
                    0 => {
                        colour[v] = 1;
                        stack.push((v, 0));
                    }
                    _ => {}
                }
            } else {
                colour[u] = 2;
                stack.pop();
            }
        }
    }
    // Longest path over forward edges (Kahn order).
    let mut indeg_f = vec![0usize; n];
    for (i, e) in g.edges.iter().enumerate() {
        if !back[i] {
            indeg_f[e.to.0 as usize] += 1;
        }
    }
    let mut rank = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| indeg_f[v] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &ei in &out[u] {
            if back[ei] {
                continue;
            }
            let v = g.edges[ei].to.0 as usize;
            rank[v] = rank[v].max(rank[u] + 1);
            indeg_f[v] -= 1;
            if indeg_f[v] == 0 {
                queue.push(v);
            }
        }
    }
    Ranked { rank, back }
}

impl Ranked {
    pub(crate) fn columns(&self) -> usize {
        self.rank.iter().copied().max().map_or(0, |m| m + 1)
    }
}
