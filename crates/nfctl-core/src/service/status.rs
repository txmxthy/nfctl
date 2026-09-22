//! The fused view: CRD state joined with what the daemon knows.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::Result;
use crate::model::{
    BufferInfo, EdgeWatermark, MonoVertex, MonoVertexKey, Pipeline, PipelineHealth, PipelineKey,
    Timestamp, Topology, VertexKind, VertexMetrics, VertexName, Windows,
};
use crate::ports::{ClusterPort, DaemonConnector};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VertexView {
    pub name: VertexName,
    pub kind: VertexKind,
    pub partitions: u32,
    /// Messages per second (daemon).
    pub rate: Windows<f64>,
    /// Pending messages (daemon).
    pub pending: Windows<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgeView {
    pub from: VertexName,
    pub to: VertexName,
    pub buffers: Vec<BufferInfo>,
    pub watermark: Option<EdgeWatermark>,
}

impl EdgeView {
    /// Sum pending messages across the edge's partitions.
    ///
    /// Returns `None` when there are no observations, or any partition is unknown,
    /// invalid, or makes the total overflow.
    #[must_use]
    pub fn pending(&self) -> Option<i64> {
        if self.buffers.is_empty() {
            None
        } else {
            total_pending(&self.buffers)
        }
    }

    /// Highest buffer usage across partitions.
    #[must_use]
    pub fn usage(&self) -> Option<f64> {
        self.buffers
            .iter()
            .filter_map(|b| b.usage.map(crate::model::Fraction::get))
            .reduce(f64::max)
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.buffers.iter().any(|b| b.is_full == Some(true))
    }
}

/// Sum known, non-negative pending counts without inventing a partial total.
#[must_use]
pub fn total_pending(buffers: &[BufferInfo]) -> Option<i64> {
    buffers.iter().try_fold(0_i64, |total, buffer| {
        let pending = buffer.pending?;
        if pending < 0 {
            return None;
        }
        total.checked_add(pending)
    })
}

/// How long each step of a view took. The TUI owns the terminal and so
/// cannot print to stderr the way `--timings` does; it reads these instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Timings {
    /// Reading the pipeline from the cluster.
    pub spec: Duration,
    /// Reaching the daemon: its pod, a port-forward and a TLS handshake.
    pub connect: Duration,
    /// The daemon's own answers: health, metrics, buffers, watermarks.
    pub numbers: Duration,
}

impl Timings {
    #[must_use]
    pub fn total(&self) -> Duration {
        self.spec + self.connect + self.numbers
    }
}

/// Everything `top`/`status` show. Daemon data is optional: when the daemon is
/// unreachable the CRD half is still returned and `warnings` says why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipelineView {
    pub pipeline: Pipeline,
    pub health: Option<PipelineHealth>,
    pub vertices: Vec<VertexView>,
    pub edges: Vec<EdgeView>,
    pub warnings: Vec<String>,
    pub at: Timestamp,
    #[serde(skip)]
    pub timings: Timings,
}

impl PipelineView {
    /// Pause is complete when every buffer is empty (mirrors the daemon client's `IsDrained`).
    #[must_use]
    pub fn drained(&self) -> Option<bool> {
        if self.edges.iter().all(|e| e.buffers.is_empty()) {
            return None;
        }
        Some(
            self.edges
                .iter()
                .flat_map(|e| &e.buffers)
                .all(|b| b.pending == Some(0) && b.ack_pending == Some(0)),
        )
    }
}

/// Build the view. Each daemon call fails independently; a failure becomes a warning.
/// The shape alone: what the cluster knows without asking the daemon
/// anything. One API call, so it lands while the numbers are still coming,
/// and a reader has the graph to look at meanwhile.
#[tracing::instrument(level = "info", skip_all, fields(pipeline = %key))]
pub async fn pipeline_shape(
    cluster: &dyn ClusterPort,
    key: &PipelineKey,
    now: Timestamp,
) -> Result<PipelineView> {
    let started = Instant::now();
    let pipeline = cluster.get_pipeline(key).await?;
    Ok(assemble(
        pipeline,
        Daemon::default(),
        Vec::new(),
        Timings {
            spec: started.elapsed(),
            ..Timings::default()
        },
        now,
    ))
}

/// What a daemon answered, or the parts of it that did.
#[derive(Default)]
struct Daemon {
    health: Option<PipelineHealth>,
    metrics: Vec<VertexMetrics>,
    buffers: Vec<BufferInfo>,
    watermarks: Vec<EdgeWatermark>,
}

/// Fill in the numbers on a shape already read. Never fails: a daemon that
/// cannot be reached leaves the shape as it was, with a warning saying why.
#[tracing::instrument(level = "info", skip_all)]
pub async fn pipeline_numbers(
    daemons: &dyn DaemonConnector,
    shape: PipelineView,
    now: Timestamp,
) -> PipelineView {
    let key = shape.pipeline.key.clone();
    let mut timings = shape.timings;
    let mut warnings = Vec::new();
    let (got, connect, numbers) =
        ask(daemons, &key, &shape.pipeline.spec.topology, &mut warnings).await;
    timings.connect = connect;
    timings.numbers = numbers;
    assemble(shape.pipeline, got, warnings, timings, now)
}

#[tracing::instrument(level = "info", skip_all, fields(pipeline = %key))]
pub async fn pipeline_view(
    cluster: &dyn ClusterPort,
    daemons: &dyn DaemonConnector,
    key: &PipelineKey,
    now: Timestamp,
) -> Result<PipelineView> {
    let shape = pipeline_shape(cluster, key, now).await?;
    Ok(pipeline_numbers(daemons, shape, now).await)
}

async fn ask(
    daemons: &dyn DaemonConnector,
    key: &PipelineKey,
    t: &Topology,
    warnings: &mut Vec<String>,
) -> (Daemon, Duration, Duration) {
    let (mut health, mut metrics, mut buffers, mut watermarks) = (
        None,
        Vec::<VertexMetrics>::new(),
        Vec::<BufferInfo>::new(),
        Vec::<EdgeWatermark>::new(),
    );
    let at_connect = Instant::now();
    let connect;
    let mut numbers = Duration::default();
    match daemons.connect(key, Some(t)).await {
        Err(e) => {
            connect = at_connect.elapsed();
            warnings.push(format!("daemon unavailable: {e}"));
        }
        Ok(d) => {
            connect = at_connect.elapsed();
            let at_numbers = Instant::now();
            // Four questions of one daemon, asked together: one after another
            // is four round trips, and on a distant cluster a round trip is
            // most of what each one costs. `join` keeps every answer, so one
            // failing still leaves its own warning and the rest their data.
            let (got_health, got_metrics, got_buffers, got_marks) = futures::join!(
                d.health(),
                d.vertex_metrics(None),
                d.buffers(),
                d.watermarks(),
            );
            match got_health {
                Ok(got) => health = Some(got),
                Err(e) => warnings.push(format!("health: {e}")),
            }
            match got_metrics {
                Ok(got) => metrics = got,
                Err(e) => warnings.push(format!("vertex metrics: {e}")),
            }
            match got_buffers {
                Ok(got) => buffers = got,
                Err(e) => warnings.push(format!("buffers: {e}")),
            }
            match got_marks {
                Ok(got) => watermarks = got,
                Err(e) => warnings.push(format!("watermarks: {e}")),
            }
            numbers = at_numbers.elapsed();
        }
    }
    (
        Daemon {
            health,
            metrics,
            buffers,
            watermarks,
        },
        connect,
        numbers,
    )
}

/// The view a reader sees, from a pipeline and whatever the daemon gave.
fn assemble(
    pipeline: Pipeline,
    got: Daemon,
    warnings: Vec<String>,
    timings: Timings,
    now: Timestamp,
) -> PipelineView {
    let Daemon {
        health,
        metrics,
        buffers,
        watermarks,
    } = got;
    let t = &pipeline.spec.topology;
    let vertices = t
        .vertices()
        .iter()
        .map(|v| {
            let m = metrics.iter().find(|m| m.vertex == v.name);
            VertexView {
                name: v.name.clone(),
                kind: v.kind,
                partitions: v.partitions,
                rate: m.map(|m| m.rate).unwrap_or_default(),
                pending: m.map(|m| m.pending).unwrap_or_default(),
            }
        })
        .collect();
    let edges = t
        .edges()
        .iter()
        .map(|e| EdgeView {
            from: e.from.clone(),
            to: e.to.clone(),
            buffers: buffers
                .iter()
                .filter(|b| b.from == e.from && b.to == e.to)
                .cloned()
                .collect(),
            watermark: watermarks
                .iter()
                .find(|w| w.from == e.from && w.to == e.to)
                .cloned(),
        })
        .collect();

    PipelineView {
        pipeline,
        health,
        vertices,
        edges,
        warnings,
        at: now,
        timings,
    }
}

/// Everything a `MonoVertex` screen shows. It has no topology, so no edges and
/// no buffers: its daemon answers health and one vertex's metrics, and that is
/// the whole of the runtime half.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonoVertexView {
    pub monovertex: MonoVertex,
    pub health: Option<PipelineHealth>,
    pub metrics: Vec<VertexMetrics>,
    pub warnings: Vec<String>,
    pub at: Timestamp,
}

impl MonoVertexView {
    /// The one vertex's metrics, when the daemon gave any.
    #[must_use]
    pub fn metrics(&self) -> Option<&VertexMetrics> {
        self.metrics.first()
    }
}

/// Build the view. As with a pipeline, a daemon that cannot be reached leaves
/// the CRD half intact and adds a warning saying why.
#[tracing::instrument(level = "info", skip_all, fields(monovertex = %key))]
pub async fn monovertex_view(
    cluster: &dyn ClusterPort,
    daemons: &dyn DaemonConnector,
    key: &MonoVertexKey,
    now: Timestamp,
) -> Result<MonoVertexView> {
    let monovertex = cluster.get_monovertex(key).await?;
    let mut warnings = Vec::new();
    let (health, metrics) = match daemons.connect_monovertex(key).await {
        Ok(d) => {
            // Two questions of one daemon, asked together, as a pipeline's are.
            let (got_health, got_metrics) = futures::join!(d.health(), d.vertex_metrics(None));
            let health = match got_health {
                Ok(h) => Some(h),
                Err(e) => {
                    warnings.push(format!("health: {e}"));
                    None
                }
            };
            let metrics = match got_metrics {
                Ok(m) => m,
                Err(e) => {
                    warnings.push(format!("metrics: {e}"));
                    Vec::new()
                }
            };
            (health, metrics)
        }
        Err(e) => {
            warnings.push(format!("daemon unavailable: {e}"));
            (None, Vec::new())
        }
    };
    Ok(MonoVertexView {
        monovertex,
        health,
        metrics,
        warnings,
        at: now,
    })
}

#[cfg(all(test, feature = "fake"))]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeCluster, FakeDaemon, FakeDaemons, sample_pipeline};
    use crate::model::{BufferName, Fraction, Health, PipelinePhase};
    use crate::service::NoDaemon;

    fn key() -> PipelineKey {
        sample_pipeline("ns", "p", PipelinePhase::Running).key
    }

    fn buffer(name: &str, pending: Option<i64>) -> BufferInfo {
        let vertex = |name: &str| VertexName::new(name).unwrap();
        BufferInfo {
            name: BufferName::new(name).unwrap(),
            from: vertex("in"),
            to: vertex("out"),
            pending,
            ack_pending: Some(0),
            total: pending,
            length: Some(100),
            usage: Fraction::new(0.0),
            usage_limit: Fraction::new(0.8),
            is_full: Some(false),
        }
    }

    #[test]
    fn total_pending_requires_complete_valid_counts() {
        let cases = [
            ("all known", vec![Some(4), Some(7)], Some(11)),
            ("one unknown", vec![Some(4), None], None),
            ("all unknown", vec![None, None], None),
            ("empty", vec![], Some(0)),
            ("overflow", vec![Some(i64::MAX), Some(1)], None),
            ("negative", vec![Some(-1)], None),
        ];

        for (case, pending, expected) in cases {
            let buffers = pending
                .into_iter()
                .enumerate()
                .map(|(index, value)| buffer(&format!("partition-{index}"), value))
                .collect::<Vec<_>>();

            assert_eq!(total_pending(&buffers), expected, "{case}");
        }
    }

    #[tokio::test]
    async fn fuses_daemon_data_onto_topology() {
        let cluster =
            FakeCluster::with_pipelines(vec![sample_pipeline("ns", "p", PipelinePhase::Running)]);
        let v = |s: &str| VertexName::new(s).unwrap();
        let daemon = FakeDaemon {
            buffers: vec![BufferInfo {
                name: BufferName::new("default-p-cat-0").unwrap(),
                from: v("in"),
                to: v("cat"),
                pending: Some(12),
                ack_pending: Some(0),
                total: Some(12),
                length: Some(30000),
                usage: Fraction::new(0.5),
                usage_limit: Fraction::new(0.8),
                is_full: Some(false),
            }],
            metrics: vec![VertexMetrics {
                vertex: v("cat"),
                rate: Windows {
                    m1: Some(3.0),
                    ..Default::default()
                },
                pending: Windows {
                    m1: Some(12),
                    ..Default::default()
                },
            }],
            watermarks: vec![],
            health: Some(PipelineHealth {
                status: Health::Healthy,
                message: "ok".into(),
                code: "D1".into(),
            }),
        };
        let view = pipeline_view(
            &cluster,
            &FakeDaemons::one(daemon),
            &key(),
            Timestamp::now(),
        )
        .await
        .unwrap();
        assert!(view.warnings.is_empty());
        assert_eq!(view.health.as_ref().unwrap().status, Health::Healthy);
        assert_eq!(view.vertices[1].rate.m1, Some(3.0));
        assert_eq!(view.edges[0].pending(), Some(12));
        assert_eq!(view.edges[1].pending(), None);
        assert_eq!(view.drained(), Some(false));
    }

    #[tokio::test]
    async fn degrades_to_crd_only_with_a_warning() {
        let cluster =
            FakeCluster::with_pipelines(vec![sample_pipeline("ns", "p", PipelinePhase::Running)]);
        let view = pipeline_view(&cluster, &NoDaemon, &key(), Timestamp::now())
            .await
            .unwrap();
        assert_eq!(view.vertices.len(), 3);
        assert!(view.health.is_none());
        assert_eq!(view.warnings.len(), 1);
        assert!(view.warnings[0].starts_with("daemon unavailable"));
        assert_eq!(view.drained(), None);
        let _ = Arc::new(cluster);
    }
}
