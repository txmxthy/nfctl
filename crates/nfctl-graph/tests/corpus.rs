#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Every anonymised corpus file imports, re-emits, and renders within bounds.

mod common;

use nfctl_graph::layout::{ViewGraph, to_graph};
use nfctl_graph::{Direction, Format, from_mermaid, render, to_mermaid};
use orthodag::{Heading, Options};

#[test]
fn corpus_imports_and_renders() {
    let files = common::corpus();
    let mut pipelines = 0;
    let mut slowest = std::time::Duration::ZERO;
    for (name, src) in &files {
        let topologies = from_mermaid(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!topologies.is_empty(), "{name}: no pipelines");
        for (pl, t) in topologies {
            pipelines += 1;
            let mmd = to_mermaid(&t, Direction::LeftRight);
            let again = from_mermaid(&mmd).unwrap().remove(0).1;
            assert_eq!(again, t, "{name}/{pl}: round trip");
            for width in [120usize, 200] {
                let text = render(&t, Format::Ascii, Some(width));
                assert!(!text.trim().is_empty(), "{name}/{pl}: empty render");
                let widest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
                assert!(widest <= 220, "{name}/{pl}: {widest} cols at width {width}");
                assert!(text.lines().count() <= 400, "{name}/{pl}: too tall");
            }
            assert_eq!(
                render(&t, Format::Ascii, Some(120)),
                render(&t, Format::Ascii, Some(120)),
                "determinism"
            );
            for g in [ViewGraph::collapsed(&t), ViewGraph::expanded(&t)] {
                slowest = slowest.max(laid_out(&format!("{name}/{pl}"), &g));
            }
        }
    }
    if !files.is_empty() {
        eprintln!(
            "corpus: {} files, {pipelines} pipelines rendered, slowest layout {slowest:?}",
            files.len()
        );
    }
}

/// The orthodag drawing of one view graph, at the card view's geometry:
/// deterministic, a route per edge landing where the painter expects it,
/// coloured where the edge is tagged, and within the bounds a terminal has.
/// Returns what the layout cost, best of three.
fn laid_out(at: &str, g: &ViewGraph) -> std::time::Duration {
    let og = to_graph(g);
    let opts = Options::new().box_width(18).box_height(5);
    // Best of three: the fastest run is what the layout costs, the slower
    // ones are whatever else the machine was doing.
    let mut best = std::time::Duration::MAX;
    let mut d = orthodag::layout(&og, opts);
    for _ in 0..3 {
        let start = std::time::Instant::now();
        d = orthodag::layout(&og, opts);
        best = best.min(start.elapsed());
    }
    // 336 ms is the worst this corpus asks for in a debug build; twice that
    // leaves room for a slow machine without hiding a layout that has got
    // dramatically dearer.
    assert!(best.as_millis() < 700, "{at}: layout took {best:?}");

    let again = orthodag::layout(&og, opts);
    assert_eq!(
        d.boxes().collect::<Vec<_>>(),
        again.boxes().collect::<Vec<_>>(),
        "{at}: box determinism"
    );
    assert_eq!(
        d.routes().collect::<Vec<_>>(),
        again.routes().collect::<Vec<_>>(),
        "{at}: route determinism"
    );
    assert_eq!(d.routes().count(), g.edges.len(), "{at}: a route per edge");

    let box_of = |node: nfctl_graph::layout::NodeId| {
        d.boxes()
            .find(|b| b.node.index() == node.0 as usize)
            .unwrap()
            .rect
    };
    for r in d.routes() {
        let e = &g.edges[r.edge.index()];
        let (from, to) = (box_of(e.from), box_of(e.to));
        let first = r.points[0];
        let last = *r.points.last().unwrap();
        match r.heading {
            Heading::Right => {
                assert_eq!(
                    first.0,
                    from.x + from.w,
                    "{at}: leaves the right of its box"
                );
                assert!(
                    (from.y + 1..=from.y + from.h - 2).contains(&first.1),
                    "{at}: leaves between its box's borders"
                );
                assert_eq!(last.0, to.x - 1, "{at}: arrives a cell left of its target");
            }
            Heading::Up => {
                assert_eq!(
                    first.1,
                    from.y + from.h,
                    "{at}: a back edge leaves the bottom"
                );
                assert_eq!(last.1, to.y + to.h, "{at}: a back edge arrives underneath");
            }
        }
    }
    let colours = orthodag::colour::of(&og);
    for (i, e) in g.edges.iter().enumerate() {
        assert_eq!(
            colours[i].is_some(),
            !e.tags.is_empty(),
            "{at}: colour iff tagged"
        );
    }
    let (w, h) = d.size();
    assert!(w <= 600 && h <= 400, "{at}: {w}x{h}");
    best
}
