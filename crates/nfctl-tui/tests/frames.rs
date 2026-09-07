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

#[tokio::test]
async fn detail_routes_every_edge() {
    use nfctl_core::model::{Edge, OnFull, ScaleSpec, Topology, Vertex, VertexKind};
    let v = |n: &str, k: VertexKind| Vertex {
        name: VertexName::new(n).unwrap(),
        kind: k,
        partitions: 1,
        scale: ScaleSpec::default(),
        image: None,
    };
    let e = |a: &str, b: &str| Edge {
        from: VertexName::new(a).unwrap(),
        to: VertexName::new(b).unwrap(),
        conditions: None,
        on_full: OnFull::default(),
    };
    let topology = Topology::new(
        vec![
            v("src", VertexKind::Source),
            v("shard", VertexKind::Map),
            v("hot-0", VertexKind::Map),
            v("hot-1", VertexKind::Map),
            v("cold", VertexKind::Map),
            v("sink-a", VertexKind::Sink),
            v("sink-b", VertexKind::Sink),
        ],
        vec![
            e("src", "shard"),
            e("shard", "hot-0"),
            e("shard", "hot-1"),
            e("hot-0", "cold"),
            e("hot-1", "cold"),
            e("cold", "sink-a"),
            e("cold", "sink-b"),
            e("shard", "sink-b"), // skips two columns: routed through a lane
            e("cold", "shard"),   // back edge (UDF cycle): also a lane
        ],
    )
    .unwrap();
    let mut fx = fixture();
    let mut p = fx.pipelines[0].clone();
    p.key.name = nfctl_core::model::PipelineName::new("routing").unwrap();
    p.spec.topology = topology;
    fx.pipelines = vec![p.clone()];
    let cluster = FakeCluster::from_fixture(&fx);
    let daemons = FakeDaemons::from_fixture(&fx);
    let view = pipeline_view(
        &cluster,
        &daemons,
        &p.key,
        Timestamp::parse_rfc3339("2026-01-01T00:00:10Z").unwrap(),
    )
    .await
    .unwrap();
    let mut panel = DetailPanel::new(p.key.clone());
    panel.update(&AppEvent::Worker(WorkerReply::View(Box::new(Ok(view)))));
    insta::assert_snapshot!(frame(&panel, 120, 30));
}

async fn sharded_panel(expand: bool) -> DetailPanel {
    let fx = fixture();
    let cluster = FakeCluster::from_fixture(&fx);
    let daemons = FakeDaemons::from_fixture(&fx);
    let key = fx
        .pipelines
        .iter()
        .find(|p| p.key.name.as_str() == "sharded")
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
    if expand {
        panel.update(&AppEvent::Key(crossterm_key('x')));
    }
    panel
}

#[tokio::test]
async fn detail_collapses_shards_by_default() {
    let panel = sharded_panel(false).await;
    insta::assert_snapshot!(frame(&panel, 110, 26));
}

#[tokio::test]
async fn detail_expands_shards_on_x() {
    let panel = sharded_panel(true).await;
    insta::assert_snapshot!(frame(&panel, 110, 30));
}

#[tokio::test]
async fn detail_scrolls_columns_to_the_selection() {
    let mut panel = sharded_panel(false).await;
    // Too narrow for five columns: the last card is off-screen until selected.
    insta::assert_snapshot!("scroll_start", frame(&panel, 70, 24));
    for _ in 0..7 {
        panel.update(&AppEvent::Key(crossterm_key('j')));
    }
    insta::assert_snapshot!("scroll_to_selection", frame(&panel, 70, 24));
}

/// Same text either way; colour on paints tagged edges in more than one hue.
#[tokio::test]
async fn edge_colours_follow_tag_combinations() {
    use std::collections::BTreeSet;
    let colours = |panel: &DetailPanel| -> BTreeSet<String> {
        // Wide enough that no `N more ▶` marker adds its own arrowhead.
        let mut term = Terminal::new(TestBackend::new(140, 26)).unwrap();
        term.draw(|f| panel.view(f, f.area())).unwrap();
        let buf = term.backend().buffer().clone();
        buf.content()
            .iter()
            // Arrowheads only; the bold `N more ▶` marker is not an edge.
            .filter(|c| c.symbol() == "▶" && !c.modifier.contains(ratatui::style::Modifier::BOLD))
            .map(|c| format!("{:?}", c.fg))
            .collect()
    };
    let on = sharded_panel(false).await;
    let off = sharded_panel(false)
        .await
        .with_palette(nfctl_tui::Palette::monochrome());
    assert_eq!(frame(&on, 140, 26), frame(&off, 140, 26));
    // Untagged edges stay dim in both; tagged ones get a hue only with colour on.
    assert!(colours(&on).len() >= 3, "on: {:?}", colours(&on));
    assert!(colours(&off).len() <= 2, "off: {:?}", colours(&off));
}
