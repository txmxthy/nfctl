#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used
)]
//! Score every fixture and corpus pipeline's expanded layout at the gallery's
//! 220-column geometry, worst first.
//!
//!   `cargo run -q -p nfctl-graph --example layout_score -- [--json PATH]`

#[path = "../tests/common/mod.rs"]
mod common;

use nfctl_graph::layout::Score;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = if args.iter().any(|a| a == "--ribbon") {
        nfctl_graph::layout::LayoutOptions {
            bundling: nfctl_graph::layout::Bundling::Ribbon,
            ..common::SCORE_OPTS
        }
    } else {
        common::SCORE_OPTS
    };
    let scores = common::scores_with(opts);
    if let Some(i) = args.iter().position(|a| a == "--json") {
        let path = args.get(i + 1).expect("--json PATH");
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(path, serde_json::to_string_pretty(&scores).unwrap()).unwrap();
        eprintln!("wrote {path}");
    }
    let mut rows: Vec<(&String, &Score)> = scores.iter().collect();
    rows.sort_by_key(|(n, s)| (std::cmp::Reverse(s.total), (*n).clone()));
    let w = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(8);
    for (name, s) in &rows {
        println!("{name:<w$}  {s}");
    }
    let sum = |f: &dyn Fn(&Score) -> i64| rows.iter().map(|(_, s)| f(s)).sum::<i64>();
    let n = |x: usize| i64::try_from(x).unwrap_or(i64::MAX);
    println!(
        "{} pipelines  bends>2 {}  skip>4 {}  back>4 {}  junc>1 {}  overlap {}  cross {}  asym {}  detour {}  total {}",
        rows.len(),
        sum(&|s| n(s.bends_over_fwd)),
        sum(&|s| n(s.bends_over_skip)),
        sum(&|s| n(s.bends_over_back)),
        sum(&|s| n(s.junction_over)),
        sum(&|s| n(s.overlaps)),
        sum(&|s| n(s.crossings)),
        sum(&|s| i64::from(s.asymmetry)),
        sum(&|s| i64::from(s.detour)),
        sum(&|s| s.total),
    );
}
