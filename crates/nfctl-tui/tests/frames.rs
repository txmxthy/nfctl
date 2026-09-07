#![allow(clippy::unwrap_used)]
//! Golden frames: every panel rendered into an in-memory terminal from the demo
//! fixture. The same fixture drives `nfctl --fixture` and the README recordings,
//! so what the tests pin is what people see.

use std::sync::Arc;

use nfctl_core::fake::{FakeCluster, FakeDaemons, Fixture};
use nfctl_core::model::{MonoVertexKey, PipelineKey, Timestamp, VertexName};
use nfctl_core::service::{PipelineService, pipeline_view};
use nfctl_tui::panels::detail::DetailPanel;
use nfctl_tui::panels::logs::LogsPanel;
use nfctl_tui::panels::pipelines::PipelinesPanel;
use nfctl_tui::{AppEvent, Model, WorkerReply};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FIXTURE: &str = include_str!("../../../examples/fixtures/demo.yaml");

fn fixture() -> Fixture {
    Fixture::parse(FIXTURE).unwrap()
}

fn frame(model: &dyn Model, width: u16, height: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
    term.draw(|f| model.view(f, f.area())).unwrap();
    term.backend().to_string()
}

#[tokio::test]
async fn pipelines_panel() {
    let fx = fixture();
    let mut panel = PipelinesPanel::new(None);
    panel.update(&AppEvent::Worker(WorkerReply::Pipelines(Ok(fx
        .pipelines
        .clone()))));
    insta::assert_snapshot!(frame(&panel, 100, 12));
}

#[tokio::test]
async fn detail_panel_with_cards() {
    let fx = fixture();
    let cluster = FakeCluster::from_fixture(&fx);
    let daemons = FakeDaemons::from_fixture(&fx);
    let key = fx
        .pipelines
        .iter()
        .find(|p| p.key.name.as_str() == "fanout")
        .unwrap()
        .key
        .clone();
    let view = pipeline_view(
        &cluster,
        &daemons,
        &key,
        Timestamp::parse_rfc3339("2026-01-01T00:00:10Z").unwrap(),
    )
    .await
    .unwrap();
    let mut panel = DetailPanel::new(key);
    panel.update(&AppEvent::Worker(WorkerReply::View(Box::new(Ok(view)))));
    // Move the selection to the second vertex so the highlight is exercised.
    panel.update(&AppEvent::Key(crossterm_key('j')));
    insta::assert_snapshot!(frame(&panel, 110, 24));
}

#[tokio::test]
async fn logs_panel_follows_then_pins() {
    let fx = fixture();
    let cluster: Arc<dyn nfctl_core::ports::ClusterPort> = Arc::new(FakeCluster::from_fixture(&fx));
    let _svc = PipelineService::new(
        Arc::clone(&cluster),
        Arc::new(FakeDaemons::from_fixture(&fx)),
    );
    let key = PipelineKey::new(
        fx.pipelines[0].key.namespace.clone(),
        nfctl_core::model::PipelineName::new("linear").unwrap(),
    );
    let mut panel = LogsPanel::new(key, Some(VertexName::new("cat").unwrap()));
    for pod in &fx.pods {
        let mut containers: Vec<_> = pod.logs.iter().collect();
        containers.sort_by(|a, b| a.0.cmp(b.0));
        for (container, lines) in containers {
            if pod.pod.name.as_str().starts_with("linear-cat") {
                for line in lines {
                    let l = nfctl_core::model::TaggedLine {
                        pod: pod.pod.name.clone(),
                        container: container.clone(),
                        line: line.clone(),
                    };
                    panel.update(&AppEvent::Worker(WorkerReply::Log(l)));
                }
            }
        }
    }
    insta::assert_snapshot!("logs_following", frame(&panel, 100, 8));
    panel.update(&AppEvent::Key(crossterm_key('k')));
    insta::assert_snapshot!("logs_pinned", frame(&panel, 100, 8));
    let _ = MonoVertexKey {
        namespace: fx.pipelines[0].key.namespace.clone(),
        name: fx.monovertices[0].key.name.clone(),
    };
}

fn crossterm_key(c: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(c),
        crossterm::event::KeyModifiers::NONE,
    )
}
