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
