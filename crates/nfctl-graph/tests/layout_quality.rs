#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! The layout quality gate. Two tiers against a baseline the orchestrator
//! writes (`target/layout-lab/baseline.json`): the vocabulary tier (bends over
//! budget, junctions over one, overlaps) may never rise per pipeline, so a
//! pipeline that reaches zero stays there; the soft tier (crossings, asymmetry,
//! detour) may not rise in total and only a little per pipeline. Fixtures must
//! be at zero on the vocabulary tier outright. Without a baseline only the
//! fixture assertions run.

mod common;

use std::collections::BTreeMap;

use nfctl_graph::layout::Score;

const SOFT_SLACK: i64 = 3;
const HEIGHT_SLACK: u16 = 2;
const WIDTH_SLACK: u16 = 4;

fn baseline() -> Option<BTreeMap<String, Score>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/layout-lab/baseline.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[test]
fn fixtures_use_only_the_vocabulary() {
    let scores = common::scores();
    let mut lines = Vec::new();
    for (name, s) in scores.iter().filter(|(n, _)| n.starts_with("fixture/")) {
        assert_eq!(s.vocabulary(), [0, 0, 0, 0, 0], "{name}: {s}");
        lines.push(format!("{name}: {s}"));
    }
    insta::assert_snapshot!("fixture_scores", lines.join("\n"));
}

#[test]
fn nothing_regresses_against_the_baseline() {
    let Some(base) = baseline() else {
        eprintln!("layout_quality: no target/layout-lab/baseline.json, skipping the ratchet");
        return;
    };
    let now = common::scores();
    let mut failures = Vec::new();
    let mut soft_now = 0;
    let mut soft_base = 0;
    for (name, b) in &base {
        let Some(s) = now.get(name) else {
            failures.push(format!("{name}: missing now"));
            continue;
        };
        for (i, (n, o)) in s.vocabulary().iter().zip(b.vocabulary()).enumerate() {
            if *n > o {
                failures.push(format!("{name}: vocabulary[{i}] {o} -> {n}"));
            }
        }
        if s.soft() > b.soft() + SOFT_SLACK {
            failures.push(format!("{name}: soft {} -> {}", b.soft(), s.soft()));
        }
        if s.height > b.height + HEIGHT_SLACK {
            failures.push(format!("{name}: height {} -> {}", b.height, s.height));
        }
        if s.width > b.width + WIDTH_SLACK {
            failures.push(format!("{name}: width {} -> {}", b.width, s.width));
        }
        soft_now += s.soft();
        soft_base += b.soft();
    }
    if soft_now > soft_base {
        failures.push(format!("soft total {soft_base} -> {soft_now}"));
    }
    assert!(
        failures.is_empty(),
        "layout regressed:\n{}",
        failures.join("\n")
    );
    eprintln!(
        "layout_quality: {} pipelines, soft {soft_base} -> {soft_now}",
        base.len()
    );
}
