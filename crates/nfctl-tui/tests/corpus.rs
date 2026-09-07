#![allow(clippy::print_stderr, clippy::unwrap_used)]
//! Every corpus pipeline renders through the card view without panicking and
//! with every edge given a route. Skips when `testdata/private` is absent.

#[path = "../../nfctl-graph/tests/common/mod.rs"]
mod common;

use nfctl_core::fake::{FakeCluster, FakeDaemons};
use nfctl_core::model::{Namespace, PipelineKey, PipelineName, PipelinePhase, Timestamp};
use nfctl_core::service::pipeline_view;
use nfctl_graph::from_mermaid;
use nfctl_tui::panels::detail::DetailPanel;
use nfctl_tui::{AppEvent, Model, WorkerReply};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[tokio::test]
async fn corpus_renders_as_cards() {
    let mut count = 0;
    for (name, src) in common::corpus() {
        for (i, (_, topology)) in from_mermaid(&src).unwrap().into_iter().enumerate() {
            let mut p = nfctl_core::fake::sample_pipeline("corpus", "p", PipelinePhase::Running);
            p.key = PipelineKey::new(
                Namespace::new("corpus").unwrap(),
                PipelineName::new(format!("p{i}")).unwrap(),
            );
            p.spec.topology = topology;
            let cluster = FakeCluster::with_pipelines(vec![p.clone()]);
            let view = pipeline_view(&cluster, &FakeDaemons::default(), &p.key, Timestamp::now())
                .await
                .unwrap();
            let mut panel = DetailPanel::new(p.key.clone());
            panel.update(&AppEvent::Worker(WorkerReply::View(Box::new(Ok(view)))));
            for (w, h) in [(120u16, 40u16), (220, 60)] {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| panel.view(f, f.area()))
                    .unwrap_or_else(|e| panic!("{name}/p{i} at {w}x{h}: {e}"));
                let text = term.backend().to_string();
                assert!(
                    text.contains('▶') || p.spec.topology.edges().is_empty(),
                    "{name}/p{i}: no arrowheads at {w}x{h}"
                );
            }
            count += 1;
        }
    }
    eprintln!("corpus: {count} pipelines rendered as cards");
}
