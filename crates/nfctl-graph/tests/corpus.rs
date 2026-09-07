#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Every anonymised corpus file imports, re-emits, and renders within bounds.

mod common;

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
        }
    }
    if !files.is_empty() {
        eprintln!(
            "corpus: {} files, {pipelines} pipelines rendered",
            files.len()
        );
    }
}
