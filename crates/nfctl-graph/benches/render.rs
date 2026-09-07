#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Import and render the five largest corpus files. Skips silently without the corpus.

#[path = "../tests/common/mod.rs"]
mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use nfctl_graph::layout::{LayoutOptions, ViewGraph, layout};
use nfctl_graph::{Direction, Format, from_mermaid, render, to_mermaid};

fn bench(c: &mut Criterion) {
    let mut files = common::corpus();
    files.sort_by_key(|(_, s)| std::cmp::Reverse(s.len()));
    for (name, src) in files.into_iter().take(5) {
        c.bench_function(&format!("import/{name}"), |b| {
            b.iter(|| from_mermaid(&src).unwrap());
        });
        let topologies = from_mermaid(&src).unwrap();
        for (pl, t) in topologies.into_iter().take(1) {
            let mmd = to_mermaid(&t, Direction::LeftRight);
            c.bench_function(&format!("emit/{name}/{pl}"), |b| {
                b.iter(|| to_mermaid(&t, Direction::LeftRight));
            });
            c.bench_function(&format!("ascii/{name}/{pl}"), |b| {
                b.iter(|| render(&t, Format::Ascii, Some(160)));
            });
            let g = ViewGraph::collapsed(&t);
            c.bench_function(&format!("layout/{name}/{pl}"), |b| {
                b.iter(|| {
                    layout(
                        &g,
                        LayoutOptions {
                            card_w: 18,
                            card_h: 4,
                        },
                    )
                });
            });
            std::hint::black_box(mmd);
        }
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
