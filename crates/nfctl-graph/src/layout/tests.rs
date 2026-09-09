#![allow(clippy::unwrap_used)]

use std::fmt::Write as _;

use super::*;
use crate::from_mermaid;
use nfctl_core::model::Topology;

fn topo(body: &str) -> Topology {
    let src = format!("graph LR\n subgraph p[\"p\"]\n{body}\n end\n");
    from_mermaid(&src).unwrap().remove(0).1
}

const OPTS: LayoutOptions = LayoutOptions {
    card_w: 12,
    card_h: 5,
};

fn lay(body: &str) -> (ViewGraph, Layout) {
    let g = ViewGraph::collapsed(&topo(body));
    let l = layout(&g, OPTS);
    (g, l)
}

fn node(g: &ViewGraph, label: &str) -> NodeId {
    NodeId(u32::try_from(g.nodes.iter().position(|n| n.label == label).unwrap()).unwrap())
}

/// Every route starts at its source card's right edge and ends one cell left of its target.
fn check_endpoints(g: &ViewGraph, l: &Layout) {
    for r in &l.routes {
        let e = &g.edges[r.edge.0 as usize];
        let from = l.card(e.from).unwrap();
        let to = l.card(e.to).unwrap();
        let first = r.polyline[0];
        let last = *r.polyline.last().unwrap();
        let mid = |c: &CardPos| c.y + i32::from(c.h) / 2;
        assert_eq!(
            first,
            (from.x + i32::from(OPTS.card_w), mid(from)),
            "start of {:?}",
            r.edge
        );
        assert_eq!(last, (to.x - 1, mid(to)), "end of {:?}", r.edge);
        assert_eq!(r.head, last);
        for w in r.polyline.windows(2) {
            assert!(w[0].0 == w[1].0 || w[0].1 == w[1].1, "orthogonal");
        }
    }
}

#[test]
fn linear_chain_is_one_row() {
    let (g, l) = lay("a([a]) --> b[b]\n b --> c[[c]]");
    assert_eq!(l.columns.len(), 3);
    assert!(
        l.gaps.iter().all(|g| g.tracks.is_empty()),
        "straight edges take no track"
    );
    assert!(l.cards.iter().all(|c| c.y == 0));
    assert_eq!(l.height, 5);
    assert_eq!(l.lanes, 0);
    check_endpoints(&g, &l);
    assert_eq!(l.routes[0].polyline, vec![(12, 2), (16, 2)]);
}

#[test]
fn fan_out_shares_one_track() {
    let (g, l) = lay("s([s]) --> a[a]\n s --> b[b]\n s --> c[c]\n s --> d[d]");
    assert_eq!(l.gaps[0].tracks.len(), 1);
    assert_eq!(l.gaps[0].width, MIN_GAP);
    assert_eq!(l.height, 23);
    check_endpoints(&g, &l);
}

#[test]
fn fan_in_shares_one_track_and_one_head() {
    let (g, l) = lay("a([a]) --> k[[k]]\n b([b]) --> k\n c([c]) --> k");
    assert_eq!(l.gaps[0].tracks.len(), 1);
    let heads: std::collections::BTreeSet<_> = l.routes.iter().map(|r| r.head).collect();
    assert_eq!(heads.len(), 1);
    check_endpoints(&g, &l);
}

#[test]
fn unrelated_overlapping_runs_take_two_tracks() {
    // a→y and b→x cross: different sources, different targets, overlapping rows.
    let (g, l) = lay("a([a]) --> y[y]\n b([b]) --> x[x]\n a --> x\n b --> y");
    assert_eq!(l.gaps[0].tracks.len(), 2);
    check_endpoints(&g, &l);
}

#[test]
fn skip_edge_passes_through_a_slot() {
    let (g, l) = lay("s([s]) --> m[m]\n m --> t[[t]]\n s --> t");
    let pass = l.columns[1]
        .iter()
        .filter(|s| matches!(s, Slot::Pass(_)))
        .count();
    assert_eq!(pass, 1);
    // The skip edge runs straight across the middle column.
    let skip = &l.routes[2];
    let ys: std::collections::BTreeSet<_> = skip.polyline.iter().map(|p| p.1).collect();
    assert!(ys.len() <= 3, "{:?}", skip.polyline);
    check_endpoints(&g, &l);
}

#[test]
fn cycle_gets_a_lane_under_the_cards() {
    let (g, l) = lay("s([s]) --> a[a]\n a --> b[b]\n b --> a\n b --> t[[t]]");
    assert_eq!(l.lanes, 1);
    assert_eq!(l.columns.len(), 4);
    let back = &l.routes[2];
    let lane_y = back.polyline.iter().map(|p| p.1).max().unwrap();
    assert_eq!(lane_y, 5);
    check_endpoints(&g, &l);
}

#[test]
fn back_edge_into_column_zero_reserves_a_margin() {
    let (_, l) = lay("a[a] --> b[b]\n b --> a");
    assert_eq!(l.col_x[0], 3);
}

#[test]
fn shards_collapse_into_one_node() {
    let g = ViewGraph::collapsed(&topo(
        "s([s]) --> w-0[w-0]\n s --> w-1[w-1]\n s --> w-2[w-2]\n w-0 --> t[[t]]\n w-1 --> t\n w-2 --> t",
    ));
    assert_eq!(g.nodes.len(), 3);
    let w = &g.nodes[node(&g, "w ×3").0 as usize];
    assert_eq!(w.members.len(), 3);
    assert_eq!(g.edges.len(), 2);
    assert_eq!(g.edges[0].members.len(), 3);
    let expanded = ViewGraph::expanded(&topo("s([s]) --> w-0[w-0]\n s --> w-1[w-1]"));
    assert_eq!(expanded.nodes.len(), 3);
}

#[test]
fn shard_tags_carrying_the_index_normalise() {
    let g = ViewGraph::collapsed(&topo(
        "s([s]) -->|route-0| w-0[w-0]\n s -->|route-1| w-1[w-1]\n o([o]) -->|other| w-1",
    ));
    assert_eq!(g.nodes.len(), 4, "different neighbourhoods, no group");
    let g = ViewGraph::collapsed(&topo(
        "s([s]) -->|route-0| w-0[w-0]\n s -->|route-1| w-1[w-1]",
    ));
    assert_eq!(g.nodes.len(), 2);
    assert_eq!(g.edges.len(), 1);
    assert_eq!(g.edges[0].tags, ["route-*"]);
    assert_eq!(g.edges[0].members.len(), 2);
}

#[test]
fn shards_with_different_neighbours_stay_apart() {
    let g = ViewGraph::collapsed(&topo(
        "s([s]) --> w-0[w-0]\n s --> w-1[w-1]\n w-0 --> t[[t]]",
    ));
    assert_eq!(g.nodes.len(), 4);
    let g = ViewGraph::collapsed(&topo("s([s]) -->|x| w-0[w-0]\n s -->|y| w-1[w-1]"));
    assert_eq!(g.nodes.len(), 3);
    let g = ViewGraph::collapsed(&topo("s([s]) --> w-0[w-0]\n s --> w-1{{w-1}}"));
    assert_eq!(g.nodes.len(), 3);
}

#[test]
fn ordering_removes_crossings() {
    // Spec order puts a above b, but a feeds y and b feeds x.
    let (_, l) = lay("a([a]) --> y[y]\n b([b]) --> x[x]\n x --> e[[e]]\n y --> f[[f]]");
    let g = ViewGraph::collapsed(&topo(
        "a([a]) --> y[y]\n b([b]) --> x[x]\n x --> e[[e]]\n y --> f[[f]]",
    ));
    let ya = l.card(node(&g, "a")).unwrap().y;
    let yy = l.card(node(&g, "y")).unwrap().y;
    let yb = l.card(node(&g, "b")).unwrap().y;
    let yx = l.card(node(&g, "x")).unwrap().y;
    assert_eq!(ya < yb, yy < yx);
}

#[test]
fn colours_key_on_tag_combination() {
    let (g, l) =
        lay("s([s]) -->|a| x[x]\n s -->|b| y[y]\n s -->|a| z[z]\n s --> w[w]\n s -->|b, a| v[v]");
    let c = colours(&g);
    assert_eq!(c[0], c[2]);
    assert_ne!(c[0], c[1]);
    assert_eq!(c[3], None);
    assert_ne!(c[4], c[0]);
    assert!(l.routes.iter().zip(&c).all(|(r, c)| r.colour == *c));
}

#[test]
fn badges_merge_at_the_destination() {
    let (g, l) =
        lay("a([a]) -->|t1| k[[k]]\n b([b]) -->|t1| k\n c([c]) -->|t2| k\n d([d]) -->|t3| m[[m]]");
    let k = node(&g, "k");
    let badges = &l.badges[&k];
    assert_eq!(
        badges.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["t1", "t2"]
    );
    assert_eq!(l.card(k).unwrap().h, 5);
    assert_eq!(l.badges[&node(&g, "m")].len(), 1);
    assert!(!l.badges.contains_key(&node(&g, "a")));
}

#[test]
fn deterministic_and_fast() {
    let mut body = String::new();
    for i in 0..20 {
        writeln!(body, "s{i}([s{i}]) --> m{i}[m{i}]").unwrap();
        for j in 0..5 {
            writeln!(body, "m{i} -->|t{j}| k{j}[[k{j}]]").unwrap();
        }
    }
    let g = ViewGraph::collapsed(&topo(&body));
    assert_eq!(g.edges.len(), 120);
    let start = std::time::Instant::now();
    let a = layout(&g, OPTS);
    let took = start.elapsed();
    let b = layout(&g, OPTS);
    assert_eq!(a, b);
    assert!(took.as_millis() < 50, "{took:?}");
    check_endpoints(&g, &a);
}

#[test]
fn skip_edge_runs_between_stacked_cards() {
    // `s` sits beside a fan-out; its skip edge would wrap the whole stack
    // in the middle column unless a slot opens between two of its cards.
    let (g, l) = lay(
        "a([a]) --> b[b]\n a --> c[c]\n a --> d[d]\n a --> e[e]\n b --> t[[t]]\n c --> t\n d --> t\n e --> t\n s([s]) --> t",
    );
    let skip = l
        .routes
        .iter()
        .find(|r| g.edges[r.edge.0 as usize].from == node(&g, "s"))
        .unwrap();
    let ys: std::collections::BTreeSet<_> = skip.polyline.iter().map(|p| p.1).collect();
    let (top, bottom) = l
        .cards
        .iter()
        .filter(|c| c.col == 1)
        .fold((i32::MAX, 0), |(t, b), c| {
            (t.min(c.y), b.max(c.y + i32::from(c.h)))
        });
    assert!(
        ys.iter().all(|&y| y > top && y < bottom),
        "runs inside the stack: {:?}",
        skip.polyline
    );
    check_endpoints(&g, &l);
}
