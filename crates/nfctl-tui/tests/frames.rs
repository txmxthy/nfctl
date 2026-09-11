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
    insta::assert_snapshot!(frame(&panel, 110, 34));
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

/// A pipeline named `routing` with `topology`, loaded into a detail panel.
async fn topology_panel(topology: nfctl_core::model::Topology) -> DetailPanel {
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
    panel
}

/// A fork, a join, a two-column skip and a back edge: every route kind.
fn routing_topology() -> nfctl_core::model::Topology {
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
    Topology::new(
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
    .unwrap()
}

#[tokio::test]
async fn detail_routes_every_edge() {
    let panel = topology_panel(routing_topology()).await;
    insta::assert_snapshot!(frame(&panel, 120, 34));
}

/// `Bridge` breaks the horizontal either side of a true crossing and leaves
/// junctions alone; `Cross`, the default, draws the same frame as before.
#[tokio::test]
async fn detail_bridges_true_crossings() {
    use nfctl_core::model::{Edge, OnFull, ScaleSpec, Topology, Vertex, VertexKind};
    use nfctl_tui::{CrossingStyle, Palette};
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
    // `a` fans out to three; `p -> sink` passes straight through the fan's bus.
    let topology = Topology::new(
        vec![
            v("src", VertexKind::Source),
            v("a", VertexKind::Map),
            v("p", VertexKind::Map),
            v("b1", VertexKind::Map),
            v("b2", VertexKind::Map),
            v("b3", VertexKind::Map),
            v("sink", VertexKind::Sink),
        ],
        vec![
            e("src", "a"),
            e("src", "p"),
            e("a", "b1"),
            e("a", "b2"),
            e("a", "b3"),
            e("p", "sink"),
            e("b1", "sink"),
            e("b2", "sink"),
            e("b3", "sink"),
        ],
    )
    .unwrap();
    let bridge = Palette::default().with_crossing(CrossingStyle::Bridge);
    let cross = Palette::default().with_crossing(CrossingStyle::Cross);
    // The default is the bridge, so the panel drawn plain is the bridged one.
    let plain = frame(&topology_panel(topology.clone()).await, 120, 40);
    let bridged = frame(
        &topology_panel(topology.clone()).await.with_palette(bridge),
        120,
        40,
    );
    assert_eq!(bridged, plain);
    let crossed = frame(&topology_panel(topology).await.with_palette(cross), 120, 40);
    // The horizontal is broken beside the crossing, so the vertical reads as
    // passing over it. Only one side, where the cell on the other is the
    // edge's own end and there is nothing to break.
    assert!(plain.contains('╶') || plain.contains('╴'), "{plain}");
    assert_eq!(crossed.matches('┼').count(), 1);
    assert_eq!(bridged.matches('┼').count(), 0);
    insta::assert_snapshot!(bridged);

    // The routing fixture's one crossing sits where the horizontal turns:
    // only the plain side is cut, the corner stays.
    let mut panel = topology_panel(routing_topology())
        .await
        .with_palette(bridge);
    panel.update(&AppEvent::Key(crossterm_key('x')));
    let expanded = frame(&panel, 120, 40);
    assert!(expanded.contains("─╴│┘"), "{expanded}");
    panel = topology_panel(routing_topology()).await.with_palette(cross);
    panel.update(&AppEvent::Key(crossterm_key('x')));
    assert_eq!(frame(&panel, 120, 40).matches('┼').count(), 1);
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
    insta::assert_snapshot!(frame(&panel, 110, 32));
}

#[tokio::test]
async fn detail_expands_shards_on_x() {
    let panel = sharded_panel(true).await;
    insta::assert_snapshot!(frame(&panel, 110, 46));
}

#[tokio::test]
async fn detail_scrolls_columns_to_the_selection() {
    let mut panel = sharded_panel(false).await;
    // Too narrow for five columns: the last card is off-screen until selected.
    insta::assert_snapshot!("scroll_start", frame(&panel, 70, 32));
    for _ in 0..7 {
        panel.update(&AppEvent::Key(crossterm_key('j')));
    }
    insta::assert_snapshot!("scroll_to_selection", frame(&panel, 70, 32));
}

/// Same text either way; colour on paints tagged edges in more than one hue.
#[tokio::test]
async fn edge_colours_follow_tag_combinations() {
    use std::collections::BTreeSet;
    let colours = |panel: &DetailPanel| -> BTreeSet<String> {
        // Wide enough that no `N more ▶` marker adds its own arrowhead.
        let mut term = Terminal::new(TestBackend::new(140, 32)).unwrap();
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
    assert_eq!(frame(&on, 140, 32), frame(&off, 140, 32));
    // Untagged edges stay dim in both; tagged ones get a hue only with colour on.
    assert!(colours(&on).len() >= 3, "on: {:?}", colours(&on));
    assert!(colours(&off).len() <= 2, "off: {:?}", colours(&off));
}

/// The TAGS column and the `->` between the names take the edge's colour.
#[tokio::test]
async fn edge_table_is_coloured_like_the_edges() {
    let panel = sharded_panel(false).await;
    let mut term = Terminal::new(TestBackend::new(140, 32)).unwrap();
    term.draw(|f| panel.view(f, f.area())).unwrap();
    let buf = term.backend().buffer().clone();
    // Find the row for `router -> audit`: its tag cell must not be the default colour.
    let mut found = false;
    for y in 0..buf.area.height {
        let line: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect();
        if let Some(col) = line.find("audit") {
            if line.contains("router -> audit") {
                // Cells, not bytes: the panel's borders are three bytes each.
                let cell = |byte: usize| line[..byte].chars().count();
                let tag_x = cell(line.rfind("audit").unwrap());
                let arrow_x = cell(line.find("->").unwrap());
                let tag_fg = buf[(u16::try_from(tag_x).unwrap(), y)].fg;
                let arrow_fg = buf[(u16::try_from(arrow_x).unwrap(), y)].fg;
                assert_ne!(tag_fg, ratatui::style::Color::Reset, "tag coloured");
                assert_eq!(tag_fg, arrow_fg, "arrow matches its tag");
                found = true;
            }
            let _ = col;
        }
    }
    assert!(found, "edge row present");
}

/// A source fanning into `n` sinks: tall, and one edge a sink.
fn wide_topology(n: usize) -> nfctl_core::model::Topology {
    use nfctl_core::model::{Edge, OnFull, ScaleSpec, Topology, Vertex, VertexKind};
    let v = |name: String, k: VertexKind| Vertex {
        name: VertexName::new(name).unwrap(),
        kind: k,
        partitions: 1,
        scale: ScaleSpec::default(),
        image: None,
    };
    let mut vs = vec![v("src".to_owned(), VertexKind::Source)];
    let mut es = Vec::new();
    for i in 0..n {
        vs.push(v(format!("sink-{i}"), VertexKind::Sink));
        es.push(Edge {
            from: VertexName::new("src").unwrap(),
            to: VertexName::new(format!("sink-{i}")).unwrap(),
            conditions: None,
            on_full: OnFull::default(),
        });
    }
    Topology::new(vs, es).unwrap()
}

/// A drawing taller than its box starts at the top, says how much is out of
/// sight, and scrolls.
#[tokio::test]
async fn detail_scrolls_a_tall_flow() {
    let mut panel = topology_panel(wide_topology(10)).await;
    panel.update(&AppEvent::Key(crossterm_key('x')));
    let short = frame(&panel, 120, 26);
    assert!(short.contains('▼'), "{short}");
    assert!(!short.contains('▲'), "{short}");
    panel.update(&AppEvent::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::PageDown,
        crossterm::event::KeyModifiers::NONE,
    )));
    let down = frame(&panel, 120, 26);
    assert!(down.contains('▲'), "{down}");
    assert_ne!(down, short);
    // Tall enough for the whole drawing: no markers, nothing cut off.
    let tall = frame(&panel, 120, 110);
    assert!(!tall.contains('▼') && !tall.contains('▲'), "{tall}");
}

/// More edges than the table has rows: they are dealt into blocks across the
/// width instead of being cut off.
#[tokio::test]
async fn detail_lays_edges_in_blocks() {
    let mut panel = topology_panel(wide_topology(10)).await;
    panel.update(&AppEvent::Key(crossterm_key('x')));
    let wide = frame(&panel, 200, 30);
    assert!(wide.matches("EDGE").count() >= 2, "{wide}");
    for i in 0..10 {
        assert!(
            wide.contains(&format!("sink-{i}")),
            "sink-{i} missing\n{wide}"
        );
    }
    // One block when the width only holds one.
    let narrow = frame(&panel, 90, 40);
    assert_eq!(narrow.matches("EDGE").count(), 1, "{narrow}");
}

/// `+` and `-` move the line between the flow and the edge table.
#[tokio::test]
async fn detail_split_is_adjustable() {
    let mut panel = topology_panel(wide_topology(6)).await;
    panel.update(&AppEvent::Key(crossterm_key('x')));
    let before = frame(&panel, 120, 40);
    for _ in 0..4 {
        panel.update(&AppEvent::Key(crossterm_key('+')));
    }
    let taller = frame(&panel, 120, 40);
    assert_ne!(taller, before);
    for _ in 0..8 {
        panel.update(&AppEvent::Key(crossterm_key('-')));
    }
    let shorter = frame(&panel, 120, 40);
    assert_ne!(shorter, taller);
}
