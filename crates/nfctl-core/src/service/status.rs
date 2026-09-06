//! The fused view: CRD state joined with what the daemon knows.

use serde::Serialize;

use crate::Result;
use crate::model::{
    BufferInfo, EdgeWatermark, Pipeline, PipelineHealth, PipelineKey, Timestamp, VertexKind,
    VertexMetrics, VertexName, Windows,
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
    /// Sum of pending across the edge's partitions; `None` if every partition is unknown.
    #[must_use]
    pub fn pending(&self) -> Option<i64> {
        self.buffers
            .iter()
            .filter_map(|b| b.pending)
            .reduce(|a, b| a + b)
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
pub async fn pipeline_view(
    cluster: &dyn ClusterPort,
    daemons: &dyn DaemonConnector,
    key: &PipelineKey,
    now: Timestamp,
) -> Result<PipelineView> {
    let pipeline = cluster.get_pipeline(key).await?;
    let mut warnings = Vec::new();
    let t = &pipeline.spec.topology;

    let (mut health, mut metrics, mut buffers, mut watermarks) = (
        None,
        Vec::<VertexMetrics>::new(),
        Vec::<BufferInfo>::new(),
        Vec::<EdgeWatermark>::new(),
    );
    match daemons.connect(key).await {
        Err(e) => warnings.push(format!("daemon unavailable: {e}")),
        Ok(d) => {
            match d.health().await {
                Ok(h) => health = Some(h),
                Err(e) => warnings.push(format!("health: {e}")),
            }
            match d.vertex_metrics(None).await {
                Ok(m) => metrics = m,
                Err(e) => warnings.push(format!("vertex metrics: {e}")),
            }
            match d.buffers().await {
                Ok(b) => buffers = b,
                Err(e) => warnings.push(format!("buffers: {e}")),
            }
            match d.watermarks().await {
                Ok(w) => watermarks = w,
                Err(e) => warnings.push(format!("watermarks: {e}")),
            }
        }
    }

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

    Ok(PipelineView {
        pipeline,
        health,
        vertices,
        edges,
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
        let view = pipeline_view(&cluster, &FakeDaemons(daemon), &key(), Timestamp::now())
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
