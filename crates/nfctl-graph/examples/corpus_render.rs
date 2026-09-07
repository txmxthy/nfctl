#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Render every corpus pipeline to text files for eyeballing:
//! `cargo run -p nfctl-graph --example corpus_render -- testdata/private testdata/private/out`

use nfctl_graph::{Format, from_mermaid, render};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let in_dir = args.next().ok_or("usage: corpus_render IN_DIR OUT_DIR")?;
    let out_dir = args.next().ok_or("usage: corpus_render IN_DIR OUT_DIR")?;
    std::fs::create_dir_all(&out_dir)?;
    let mut n = 0;
    for entry in std::fs::read_dir(&in_dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "mmd") {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for (i, (_, t)) in from_mermaid(&std::fs::read_to_string(&path)?)?
            .into_iter()
            .enumerate()
        {
            std::fs::write(
                format!("{out_dir}/{stem}-{i}.txt"),
                render(&t, Format::Ascii, Some(160)),
            )?;
            n += 1;
        }
    }
    eprintln!("rendered {n} pipelines into {out_dir}");
    Ok(())
}
