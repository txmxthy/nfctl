//! In-memory adapters for tests. Enabled with the `fake` feature.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};

use crate::model::{
    BufferInfo, BufferName, ContainerName, DesiredPhase, EdgeWatermark, IsbName, IsbService,
    Lifecycle, Limits, LogLine, Namespace, ObjectMeta, Pipeline, PipelineHealth, PipelineKey,
    PipelineName, PipelinePhase, PipelineSpec, PipelineStatus, PodEvent, PodName, PodRef,
    ReplicaErrors, ResumeStrategy, ScaleSpec, Selector, Timestamp, Topology, Vertex, VertexCounts,
    VertexKind, VertexMetrics, VertexName,
};
use crate::ports::{ClusterPort, DaemonConnector, DaemonPort};
use crate::{Error, Result};

/// A cluster whose state is a `Vec<Pipeline>` behind a mutex.
#[derive(Debug, Default, Clone)]
pub struct FakeCluster {
    pipelines: Arc<Mutex<Vec<Pipeline>>>,
}

impl FakeCluster {
    #[must_use]
    pub fn with_pipelines(pipelines: Vec<Pipeline>) -> Self {
        Self {
            pipelines: Arc::new(Mutex::new(pipelines)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Pipeline>> {
        // A poisoned mutex in a test double is a test bug; recovering hides nothing useful.
        self.pipelines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl ClusterPort for FakeCluster {
    async fn list_pipelines(&self, ns: Option<&Namespace>) -> Result<Vec<Pipeline>> {
        Ok(self
            .lock()
            .iter()
            .filter(|p| ns.is_none_or(|ns| &p.key.namespace == ns))
            .cloned()
            .collect())
    }

    async fn get_pipeline(&self, key: &PipelineKey) -> Result<Pipeline> {
        self.lock()
            .iter()
            .find(|p| &p.key == key)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                kind: "pipeline",
                name: key.to_string(),
            })
    }

    async fn watch_pipeline(
        &self,
        key: &PipelineKey,
    ) -> Result<BoxStream<'static, Result<Pipeline>>> {
        let current = self.get_pipeline(key).await?;
        Ok(Box::pin(stream::once(async move { Ok(current) })))
    }

    async fn set_lifecycle(
        &self,
        key: &PipelineKey,
        desired: DesiredPhase,
        resume: Option<ResumeStrategy>,
        dry_run: bool,
    ) -> Result<()> {
        let mut g = self.lock();
        let p = g
            .iter_mut()
            .find(|p| &p.key == key)
            .ok_or_else(|| Error::NotFound {
                kind: "pipeline",
                name: key.to_string(),
            })?;
        if dry_run {
            return Ok(());
        }
        p.spec.lifecycle.desired = desired;
        p.meta.resume_strategy = resume;
        // The fake controller reconciles instantly.
        p.status.phase = match desired {
            DesiredPhase::Running => PipelinePhase::Running,
            DesiredPhase::Paused => PipelinePhase::Paused,
        };
        Ok(())
    }

    async fn list_isb(&self, _ns: &Namespace) -> Result<Vec<IsbService>> {
        Ok(vec![])
    }

    async fn list_pods(&self, _ns: &Namespace, _selector: &Selector) -> Result<Vec<PodRef>> {
        Ok(vec![])
    }

    async fn watch_pods(
        &self,
        _ns: &Namespace,
        _selector: &Selector,
    ) -> Result<BoxStream<'static, Result<PodEvent>>> {
        Ok(Box::pin(stream::empty()))
    }

    async fn delete_pods(
        &self,
        _ns: &Namespace,
        _selector: &Selector,
        _dry_run: bool,
    ) -> Result<Vec<PodName>> {
        Ok(vec![])
    }

    async fn tail_logs(
        &self,
        _ns: &Namespace,
        _pod: &PodName,
        _container: &ContainerName,
        _since: Option<Timestamp>,
        _tail_lines: Option<u32>,
    ) -> Result<BoxStream<'static, Result<LogLine>>> {
        Ok(Box::pin(stream::empty()))
    }
}

/// A daemon that answers with fixed data.
#[derive(Debug, Default, Clone)]
pub struct FakeDaemon {
    pub buffers: Vec<BufferInfo>,
    pub metrics: Vec<VertexMetrics>,
    pub watermarks: Vec<EdgeWatermark>,
    pub health: Option<PipelineHealth>,
}

#[async_trait]
impl DaemonPort for FakeDaemon {
    async fn buffers(&self) -> Result<Vec<BufferInfo>> {
        Ok(self.buffers.clone())
    }
    async fn buffer(&self, name: &BufferName) -> Result<BufferInfo> {
        self.buffers
            .iter()
            .find(|b| &b.name == name)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                kind: "buffer",
                name: name.to_string(),
            })
    }
    async fn vertex_metrics(&self, vertex: Option<&VertexName>) -> Result<Vec<VertexMetrics>> {
        Ok(self
            .metrics
            .iter()
            .filter(|m| vertex.is_none_or(|v| &m.vertex == v))
            .cloned()
            .collect())
    }
    async fn watermarks(&self) -> Result<Vec<EdgeWatermark>> {
        Ok(self.watermarks.clone())
    }
    async fn health(&self) -> Result<PipelineHealth> {
        self.health
            .clone()
            .ok_or_else(|| Error::Daemon("no health configured".into()))
    }
    async fn vertex_errors(&self, _vertex: &VertexName) -> Result<Vec<ReplicaErrors>> {
        Ok(vec![])
    }
}

/// Hands out clones of one [`FakeDaemon`] for every pipeline.
#[derive(Debug, Default, Clone)]
pub struct FakeDaemons(pub FakeDaemon);

#[async_trait]
impl DaemonConnector for FakeDaemons {
    async fn connect(&self, _key: &PipelineKey) -> Result<Box<dyn DaemonPort>> {
        Ok(Box::new(self.0.clone()))
    }
}

/// A three-vertex `in -> cat -> out` pipeline for tests and snapshots.
///
/// # Panics
/// Never: every literal is valid.
#[must_use]
pub fn sample_pipeline(ns: &str, name: &str, phase: PipelinePhase) -> Pipeline {
    let vn = |s: &str| VertexName::new(s).unwrap_or_else(|_| unreachable!("literal is valid"));
    let v = |s: &str, kind: VertexKind| Vertex {
        name: vn(s),
        kind,
        partitions: 1,
        scale: ScaleSpec::default(),
        image: None,
    };
    let e = |a: &str, b: &str| crate::model::Edge {
        from: vn(a),
        to: vn(b),
        conditions: None,
        on_full: crate::model::OnFull::default(),
    };
    let topology = Topology::new(
        vec![
            v("in", VertexKind::Source),
            v("cat", VertexKind::Map),
            v("out", VertexKind::Sink),
        ],
        vec![e("in", "cat"), e("cat", "out")],
    )
    .unwrap_or_else(|_| unreachable!("sample topology is valid"));
    let counts = VertexCounts::from_topology(&topology);
    Pipeline {
        key: PipelineKey::new(
            Namespace::new(ns).unwrap_or_else(|_| unreachable!("literal is valid")),
            PipelineName::new(name).unwrap_or_else(|_| unreachable!("literal is valid")),
        ),
        meta: ObjectMeta {
            generation: Some(1),
            created: Some(Timestamp::new(time::OffsetDateTime::UNIX_EPOCH)),
            instance: None,
            resume_strategy: None,
        },
        spec: PipelineSpec {
            isb: IsbName::new("default").unwrap_or_else(|_| unreachable!("literal is valid")),
            lifecycle: Lifecycle::default(),
            limits: Limits::default(),
            topology,
        },
        status: PipelineStatus {
            phase,
            counts,
            ..PipelineStatus::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::PipelineService;

    #[tokio::test]
    async fn service_lists_sorted_by_name() {
        let cluster = FakeCluster::with_pipelines(vec![
            sample_pipeline("ns", "zeta", PipelinePhase::Running),
            sample_pipeline("ns", "alpha", PipelinePhase::Paused),
            sample_pipeline("other", "beta", PipelinePhase::Running),
        ]);
        let svc = PipelineService::new(Arc::new(cluster), Arc::new(FakeDaemons::default()));
        let names: Vec<_> = svc
            .list(Some(&Namespace::new("ns").unwrap()))
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.key.name.to_string())
            .collect();
        assert_eq!(names, ["alpha", "zeta"]);
    }
}
