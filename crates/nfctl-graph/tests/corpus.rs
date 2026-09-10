#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Every anonymised corpus file imports, re-emits, and renders within bounds.

mod common;

use nfctl_graph::layout::{Bundling, LayoutOptions, ViewGraph, layout};
use nfctl_graph::{Direction, Format, from_mermaid, render, to_mermaid};

#[test]
fn corpus_imports_and_renders() {
    let files = common::corpus();
    let mut pipelines = 0;
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
                // Ribbon gives a card a row for every colour at it, so its
                // cards are taller, its drawings bigger and its search wider.
                // Timed apart from the default, at what it costs today, so a
                // change that makes it worse is visible.
                let ribbon = LayoutOptions {
                    bundling: Bundling::Ribbon,
                    card_w: 18,
                    card_h: 5,
                };
                let mut best = std::time::Duration::MAX;
                for _ in 0..3 {
                    let start = std::time::Instant::now();
                    let l = layout(&g, ribbon);
                    best = best.min(start.elapsed());
                    assert!(
                        l.cards.iter().all(|c| c.h >= 5),
                        "{name}/{pl}: a card shrank"
                    );
                }
                assert!(best.as_millis() < 100, "{name}/{pl}: ribbon took {best:?}");
                let opts = LayoutOptions {
                    bundling: Bundling::Spread,
                    card_w: 18,
                    card_h: 5,
                };
                // Best of three: the fastest run is what the layout costs,
                // the slower ones are whatever else the machine was doing.
                let mut best = std::time::Duration::MAX;
                let mut l = layout(&g, opts);
                for _ in 0..3 {
                    let start = std::time::Instant::now();
                    l = layout(&g, opts);
                    best = best.min(start.elapsed());
                }
                assert!(best.as_millis() < 50, "{name}/{pl}: layout took {best:?}");
                assert_eq!(l, layout(&g, opts), "{name}/{pl}: layout determinism");
                assert_eq!(
                    l.routes.len(),
                    g.edges.len(),
                    "{name}/{pl}: a route per edge"
                );
                for r in &l.routes {
                    let e = &g.edges[r.edge.0 as usize];
                    let (from, to) = (l.card(e.from).unwrap(), l.card(e.to).unwrap());
                    assert_eq!(
                        r.polyline[0],
                        (from.x + 18, from.y + i32::from(from.h) / 2),
                        "{name}/{pl}: route start"
                    );
                    assert_eq!(
                        r.head,
                        (to.x - 1, to.y + i32::from(to.h) / 2),
                        "{name}/{pl}: route head"
                    );
                    assert_eq!(
                        r.colour.is_some(),
                        !e.tags.is_empty(),
                        "{name}/{pl}: colour iff tagged"
                    );
                }
                assert!(
                    l.width <= 600 && l.height <= 400,
                    "{name}/{pl}: {}x{}",
                    l.width,
                    l.height
                );
            }
        }
    }
    if !files.is_empty() {
        eprintln!(
            "corpus: {} files, {pipelines} pipelines rendered",
            files.len()
        );
    }
}
