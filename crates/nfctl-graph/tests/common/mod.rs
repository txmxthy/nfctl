#![allow(clippy::print_stderr, clippy::unwrap_used, dead_code)]
//! The private, git-ignored corpus under `testdata/private`. Absent in CI.

use std::path::PathBuf;

pub fn corpus_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/private");
    dir.is_dir().then_some(dir)
}

/// `(file stem, source)` for every `.mmd`, sorted by name.
pub fn corpus() -> Vec<(String, String)> {
    let Some(dir) = corpus_dir() else {
        eprintln!("corpus: testdata/private absent, skipping");
        return Vec::new();
    };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mmd"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            std::fs::read_to_string(&p).ok().map(|s| (stem, s))
        })
        .collect()
}

/// The gallery's 220-column geometry.
pub const SCORE_OPTS: nfctl_graph::layout::LayoutOptions = nfctl_graph::layout::LayoutOptions {
    bundling: nfctl_graph::layout::Bundling::Spread,
    card_w: 18,
    card_h: 5,
};

/// Expanded-layout scores for every fixture pipeline and, when present, every
/// corpus pipeline, keyed `fixture/<name>` and `corpus/<file>/<pipeline>`.
pub fn scores() -> std::collections::BTreeMap<String, nfctl_graph::layout::Score> {
    scores_with(SCORE_OPTS)
}

/// The same, at a chosen geometry.
pub fn scores_with(
    opts: nfctl_graph::layout::LayoutOptions,
) -> std::collections::BTreeMap<String, nfctl_graph::layout::Score> {
    use nfctl_graph::layout::{ViewGraph, layout, score_full};
    let mut out = std::collections::BTreeMap::new();
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/fixtures/demo.yaml"),
    )
    .unwrap();
    for p in nfctl_core::fake::Fixture::parse(&text).unwrap().pipelines {
        let g = ViewGraph::expanded(&p.spec.topology);
        out.insert(
            format!("fixture/{}", p.key.name),
            score_full(&g, &layout(&g, opts)),
        );
    }
    for (file, src) in corpus() {
        for (pl, t) in nfctl_graph::from_mermaid(&src).unwrap() {
            let g = ViewGraph::expanded(&t);
            out.insert(
                format!("corpus/{file}/{pl}"),
                score_full(&g, &layout(&g, opts)),
            );
        }
    }
    out
}
