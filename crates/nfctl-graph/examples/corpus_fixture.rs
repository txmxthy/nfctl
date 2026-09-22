#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Turn the private corpus into a `--fixture` file with synthetic runtime data:
//! `cargo run -p nfctl-graph --example corpus_fixture -- testdata/private testdata/private/fixture.yaml`

use nfctl_core::fake::{DaemonFixture, Fixture};
use nfctl_core::model::{
    BufferInfo, BufferName, Fraction, Health, Namespace, PipelineHealth, PipelineKey, PipelineName,
    PipelinePhase, VertexCounts, VertexMetrics, Windows,
};

fn seed(s: &str) -> u32 {
    s.bytes().map(u32::from).sum()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let in_dir = args.next().ok_or("usage: corpus_fixture IN_DIR OUT_FILE")?;
    let out = args.next().ok_or("usage: corpus_fixture IN_DIR OUT_FILE")?;
    let mut files: Vec<_> = std::fs::read_dir(&in_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mmd"))
        .collect();
    files.sort();
    let mut fixture = Fixture::default();
    let ns = Namespace::new("corpus")?;
    for path in files {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = nfctl_graph::from_mermaid(&std::fs::read_to_string(&path)?)?;
        for (i, (_, topology)) in parsed.into_iter().enumerate() {
            let name = PipelineName::new(if i == 0 {
                stem.clone()
            } else {
                format!("{stem}-{i}")
            })?;
            let mut p = nfctl_core::fake::sample_pipeline("corpus", "p", PipelinePhase::Running);
            p.key = PipelineKey::new(ns.clone(), name.clone());
            p.status.counts = VertexCounts::from_topology(&topology);
            p.spec.topology = topology;
            let t = &p.spec.topology;
            let metrics = t
                .vertices()
                .iter()
                .map(|v| {
                    let r = 10.0 + f64::from(seed(v.name.as_str()) % 900) / 10.0;
                    let pend = i64::from(seed(v.name.as_str()) % 500);
                    VertexMetrics {
                        vertex: v.name.clone(),
                        rate: Windows {
                            m1: Some(r),
                            m5: Some(r * 0.97),
                            m15: Some(r * 0.95),
                            default: Some(r),
                        },
                        pending: Windows {
                            m1: Some(pend),
                            m5: Some(pend),
                            m15: Some(pend),
                            default: Some(pend),
                        },
                    }
                })
                .collect();
            let buffers = t
                .edges()
                .iter()
                .map(|e| -> Result<BufferInfo, Box<dyn std::error::Error>> {
                    Ok(BufferInfo {
                        name: BufferName::new(format!("default-{name}-{}-0", e.to))?,
                        sources: vec![e.from.clone()],
                        to: e.to.clone(),
                        pending: Some(i64::from(seed(e.to.as_str()) % 100)),
                        ack_pending: Some(1),
                        total: Some(101),
                        length: Some(30000),
                        usage: Fraction::new(f64::from(seed(e.to.as_str()) % 60) / 100.0),
                        usage_limit: Fraction::new(0.8),
                        is_full: Some(false),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            fixture.daemons.insert(
                name,
                DaemonFixture {
                    health: Some(PipelineHealth {
                        status: Health::Healthy,
                        message: "synthetic".into(),
                        code: "D1".into(),
                    }),
                    buffers,
                    metrics,
                    watermarks: vec![],
                },
            );
            fixture.pipelines.push(p);
        }
    }
    std::fs::write(&out, serde_yaml_ng::to_string(&fixture)?)?;
    eprintln!("wrote {} pipelines to {out}", fixture.pipelines.len());
    Ok(())
}
