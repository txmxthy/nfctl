//! In-memory adapters for tests, demos and recordings. Enabled with the `fake` feature.

mod fixture;

pub use fixture::{DaemonFixture, Fixture, PodFixture};

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::model::{
    BufferInfo, BufferName, ContainerName, DesiredPhase, EdgeWatermark, IsbName, IsbService,
    Lifecycle, Limits, LogLine, LogOptions, MonoVertex, MonoVertexKey, MonoVertexPhase, Namespace,
    ObjectMeta, Pipeline, PipelineHealth, PipelineKey, PipelineName, PipelinePhase, PipelineSpec,
    PipelineStatus, PodEvent, PodName, PodRef, ReplicaErrors, ResumeStrategy, ScaleSpec, Selector,
    Timestamp, Topology, Vertex, VertexCounts, VertexKind, VertexMetrics, VertexName,
};
use crate::ports::{ClusterPort, DaemonConnector, DaemonPort};
use crate::{Error, Result};

/// What one `tail_logs` call returns, in order of calls for that key.
#[derive(Debug, Clone)]
pub struct LogScript {
    pub lines: Vec<LogLine>,
    /// Keep the stream open after `lines` (until the tail is aborted).
    pub hang: bool,
}

/// A recorded `tail_logs` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailCall {
    pub pod: PodName,
    pub container: ContainerName,
    pub opts: LogOptions,
}

#[derive(Debug, Default)]
struct State {
    pipelines: Vec<Pipeline>,
    monovertices: Vec<MonoVertex>,
    isbs: Vec<IsbService>,
    manifests: HashMap<String, Pipeline>,
    scaled: Vec<(PipelineKey, VertexName, u32)>,
    pods: Vec<PodRef>,
    pod_watchers: Vec<mpsc::Sender<Result<PodEvent>>>,
    scripts: HashMap<(PodName, ContainerName), VecDeque<LogScript>>,
    tail_calls: Vec<TailCall>,
    /// Senders kept so a hanging script's stream stays open until dropped here.
    hung: Vec<mpsc::Sender<Result<LogLine>>>,
    /// Replayed on every open when no script is queued for the key.
    fixture: Option<Fixture>,
}

/// A cluster whose state lives behind a mutex and can be driven from a test.
#[derive(Debug, Default, Clone)]
pub struct FakeCluster {
    state: Arc<Mutex<State>>,
}

impl FakeCluster {
    #[must_use]
    pub fn with_pipelines(pipelines: Vec<Pipeline>) -> Self {
        let f = Self::default();
        f.lock().pipelines = pipelines;
        f
    }

    /// Build a cluster whose state is the fixture: pipelines, `MonoVertices`, ISBs
    /// and pods with replayable logs. Daemon data comes from [`FakeDaemons::from_fixture`].
    #[must_use]
    pub fn from_fixture(f: &Fixture) -> Self {
        let c = Self::default();
        {
            let mut g = c.lock();
            g.pipelines.clone_from(&f.pipelines);
            g.monovertices.clone_from(&f.monovertices);
            g.isbs.clone_from(&f.isbs);
            g.pods = f.pods.iter().map(|p| p.pod.clone()).collect();
            g.fixture = Some(f.clone());
        }
        c
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned mutex in a test double is a test bug; recovering hides nothing useful.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue what the next `tail_logs` for this pod/container returns.
    pub fn script(&self, pod: &PodName, container: &ContainerName, script: LogScript) {
        self.lock()
            .scripts
            .entry((pod.clone(), container.clone()))
            .or_default()
            .push_back(script);
    }

    /// Push a pod event to every open watch (and update the pod list).
    pub fn emit(&self, ev: &PodEvent) {
        let mut g = self.lock();
        match ev {
            PodEvent::Applied(p) => {
                g.pods.retain(|x| x.name != p.name);
                g.pods.push(p.clone());
            }
            PodEvent::Deleted(p) => g.pods.retain(|x| x.name != p.name),
            PodEvent::Resync | PodEvent::ResyncDone => {}
        }
        g.pod_watchers
            .retain(|tx| tx.try_send(Ok(ev.clone())).is_ok());
    }

    /// Teach the fake what a manifest text parses to (the fake has no YAML parser).
    pub fn register_manifest(&self, text: &str, pipeline: Pipeline) {
        self.lock().manifests.insert(text.to_owned(), pipeline);
    }

    #[must_use]
    pub fn scale_calls(&self) -> Vec<(PipelineKey, VertexName, u32)> {
        self.lock().scaled.clone()
    }

    pub fn add_monovertices(&self, mvs: impl IntoIterator<Item = MonoVertex>) {
        self.lock().monovertices.extend(mvs);
    }

    pub fn add_isbs(&self, isbs: impl IntoIterator<Item = IsbService>) {
        self.lock().isbs.extend(isbs);
    }

    /// Add pipelines to the cluster.
    pub fn add_pipelines(&self, pipelines: impl IntoIterator<Item = Pipeline>) {
        self.lock().pipelines.extend(pipelines);
    }

    /// Every `tail_logs` call so far.
    #[must_use]
    pub fn tail_calls(&self) -> Vec<TailCall> {
        self.lock().tail_calls.clone()
    }
}

#[async_trait]
impl ClusterPort for FakeCluster {
    async fn list_pipelines(&self, ns: Option<&Namespace>) -> Result<Vec<Pipeline>> {
        Ok(self
            .lock()
            .pipelines
            .iter()
            .filter(|p| ns.is_none_or(|ns| &p.key.namespace == ns))
            .cloned()
            .collect())
    }

    async fn get_pipeline(&self, key: &PipelineKey) -> Result<Pipeline> {
        self.lock()
            .pipelines
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
            .pipelines
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

    fn parse_manifest(&self, text: &str, _default_ns: &Namespace) -> Result<Pipeline> {
        self.lock()
            .manifests
            .get(text)
            .cloned()
            .ok_or_else(|| Error::Invalid {
                kind: "manifest",
                name: "<text>".into(),
                reason: "unregistered".into(),
            })
    }

    async fn apply_manifest(
        &self,
        text: &str,
        default_ns: &Namespace,
        dry_run: bool,
    ) -> Result<Pipeline> {
        let p = self.parse_manifest(text, default_ns)?;
        if !dry_run {
            let mut g = self.lock();
            g.pipelines.retain(|x| x.key != p.key);
            g.pipelines.push(p.clone());
        }
        Ok(p)
    }

    async fn scale_vertex(
        &self,
        key: &PipelineKey,
        vertex: &VertexName,
        replicas: u32,
        dry_run: bool,
    ) -> Result<()> {
        if !dry_run {
            self.lock()
                .scaled
                .push((key.clone(), vertex.clone(), replicas));
        }
        Ok(())
    }

    async fn list_isb(&self, ns: Option<&Namespace>) -> Result<Vec<IsbService>> {
        Ok(self
            .lock()
            .isbs
            .iter()
            .filter(|i| ns.is_none_or(|n| &i.namespace == n))
            .cloned()
            .collect())
    }

    async fn list_monovertices(&self, ns: Option<&Namespace>) -> Result<Vec<MonoVertex>> {
        Ok(self
            .lock()
            .monovertices
            .iter()
            .filter(|m| ns.is_none_or(|ns| &m.key.namespace == ns))
            .cloned()
            .collect())
    }

    async fn get_monovertex(&self, key: &MonoVertexKey) -> Result<MonoVertex> {
        self.lock()
            .monovertices
            .iter()
            .find(|m| &m.key == key)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                kind: "monovertex",
                name: key.to_string(),
            })
    }

    async fn set_monovertex_lifecycle(
        &self,
        key: &MonoVertexKey,
        desired: DesiredPhase,
        dry_run: bool,
    ) -> Result<()> {
        let mut g = self.lock();
        let m = g
            .monovertices
            .iter_mut()
            .find(|m| &m.key == key)
            .ok_or_else(|| Error::NotFound {
                kind: "monovertex",
                name: key.to_string(),
            })?;
        if !dry_run {
            m.desired = desired;
            m.phase = match desired {
                DesiredPhase::Running => MonoVertexPhase::Running,
                DesiredPhase::Paused => MonoVertexPhase::Paused,
            };
        }
        Ok(())
    }

    async fn list_pods(&self, _ns: &Namespace, _selector: &Selector) -> Result<Vec<PodRef>> {
        Ok(self.lock().pods.clone())
    }

    async fn watch_pods(
        &self,
        _ns: &Namespace,
        _selector: &Selector,
    ) -> Result<BoxStream<'static, Result<PodEvent>>> {
        let (tx, rx) = mpsc::channel(64);
        let mut g = self.lock();
        // Replay current pods as an initial list, like a real watcher.
        let _ = tx.try_send(Ok(PodEvent::Resync));
        for p in &g.pods {
            let _ = tx.try_send(Ok(PodEvent::Applied(p.clone())));
        }
        let _ = tx.try_send(Ok(PodEvent::ResyncDone));
        g.pod_watchers.push(tx);
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn delete_pods(
        &self,
        _ns: &Namespace,
        _selector: &Selector,
        dry_run: bool,
    ) -> Result<Vec<PodName>> {
        let names: Vec<PodName> = self.lock().pods.iter().map(|p| p.name.clone()).collect();
        if !dry_run {
            for n in &names {
                let pod = self.lock().pods.iter().find(|p| &p.name == n).cloned();
                if let Some(pod) = pod {
                    self.emit(&PodEvent::Deleted(pod));
                }
            }
        }
        Ok(names)
    }

    /// Deletes the pod and, standing in for the controller, applies a
    /// replacement named `<pod>-r` so rolling restarts can observe recovery.
    async fn delete_pod(&self, _ns: &Namespace, pod: &PodName, dry_run: bool) -> Result<()> {
        let found = self.lock().pods.iter().find(|p| &p.name == pod).cloned();
        let Some(old) = found else {
            return Err(Error::NotFound {
                kind: "pod",
                name: pod.to_string(),
            });
        };
        if dry_run {
            return Ok(());
        }
        self.emit(&PodEvent::Deleted(old.clone()));
        let mut replacement = old;
        replacement.name = PodName::new(format!("{pod}-r"))
            .unwrap_or_else(|_| unreachable!("a pod name plus `-r` is still a pod name"));
        self.emit(&PodEvent::Applied(replacement));
        Ok(())
    }

    async fn tail_logs(
        &self,
        _ns: &Namespace,
        pod: &PodName,
        container: &ContainerName,
        opts: &LogOptions,
    ) -> Result<BoxStream<'static, Result<LogLine>>> {
        let mut g = self.lock();
        g.tail_calls.push(TailCall {
            pod: pod.clone(),
            container: container.clone(),
            opts: *opts,
        });
        let script = g
            .scripts
            .get_mut(&(pod.clone(), container.clone()))
            .and_then(VecDeque::pop_front)
            .or_else(|| {
                g.fixture
                    .as_ref()
                    .and_then(|f| f.pod_logs(pod, container))
                    .map(|lines| LogScript { lines, hang: true })
            })
            .unwrap_or(LogScript {
                lines: vec![],
                hang: !opts.follow,
            });
        let (tx, rx) = mpsc::channel(script.lines.len().max(1));
        for l in script.lines {
            let _ = tx.try_send(Ok(l));
        }
        if script.hang && opts.follow {
            g.hung.push(tx);
        }
        Ok(Box::pin(ReceiverStream::new(rx)))
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

/// Hands out one [`FakeDaemon`] per pipeline name, falling back to a default.
#[derive(Debug, Default, Clone)]
pub struct FakeDaemons {
    pub default: FakeDaemon,
    pub by_name: HashMap<PipelineName, FakeDaemon>,
}

impl FakeDaemons {
    #[must_use]
    pub fn one(daemon: FakeDaemon) -> Self {
        Self {
            default: daemon,
            by_name: HashMap::new(),
        }
    }

    #[must_use]
    pub fn from_fixture(f: &Fixture) -> Self {
        let by_name = f
            .daemons
            .iter()
            .map(|(name, d)| {
                let daemon = FakeDaemon {
                    buffers: d.buffers.clone(),
                    metrics: d.metrics.clone(),
                    watermarks: d.watermarks.clone(),
                    health: d.health.clone(),
                };
                (name.clone(), daemon)
            })
            .collect();
        Self {
            default: FakeDaemon::default(),
            by_name,
        }
    }

    fn for_name(&self, name: &PipelineName) -> FakeDaemon {
        self.by_name
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }
}

#[async_trait]
impl DaemonConnector for FakeDaemons {
    async fn connect(
        &self,
        key: &PipelineKey,
        _topology: Option<&Topology>,
    ) -> Result<Arc<dyn DaemonPort>> {
        Ok(Arc::new(self.for_name(&key.name)))
    }

    async fn connect_monovertex(&self, key: &MonoVertexKey) -> Result<Arc<dyn DaemonPort>> {
        Ok(Arc::new(self.for_name(&key.name)))
    }
}

/// A running `MonoVertex` for tests.
#[must_use]
pub fn sample_monovertex(
    ns: &'static str,
    name: &'static str,
    phase: MonoVertexPhase,
) -> MonoVertex {
    MonoVertex {
        key: MonoVertexKey {
            namespace: lit(ns),
            name: lit(name),
        },
        desired: DesiredPhase::Running,
        phase,
        replicas: 1,
        desired_replicas: Some(1),
        ready_replicas: Some(1),
        has_transformer: true,
        has_map: false,
        message: None,
        conditions: vec![],
        created: Some(Timestamp::new(time::OffsetDateTime::UNIX_EPOCH)),
    }
}

fn lit<T: TryFrom<&'static str>>(s: &'static str) -> T {
    T::try_from(s).unwrap_or_else(|_| unreachable!("literal `{s}` is valid"))
}

/// A three-vertex `in -> cat -> out` pipeline for tests and snapshots.
#[must_use]
pub fn sample_pipeline(ns: &'static str, name: &'static str, phase: PipelinePhase) -> Pipeline {
    let v = |s: &'static str, kind: VertexKind| Vertex {
        name: lit(s),
        kind,
        partitions: 1,
        scale: ScaleSpec::default(),
        image: None,
    };
    let e = |a: &'static str, b: &'static str| crate::model::Edge {
        from: lit(a),
        to: lit(b),
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
        key: PipelineKey::new(lit(ns), lit(name)),
        meta: ObjectMeta {
            generation: Some(1),
            created: Some(Timestamp::new(time::OffsetDateTime::UNIX_EPOCH)),
            instance: None,
            resume_strategy: None,
        },
        spec: PipelineSpec {
            isb: lit::<IsbName>("default"),
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

/// A running vertex pod with a `numa` container and, optionally, a `udf` one.
#[must_use]
pub fn sample_pod(name: &'static str, with_udf: bool) -> PodRef {
    let c = |n: &'static str| crate::model::ContainerState {
        name: lit(n),
        running: true,
        restart_count: 0,
        is_init: false,
    };
    let mut containers = vec![c("numa")];
    if with_udf {
        containers.push(c("udf"));
    }
    PodRef {
        name: lit(name),
        phase: crate::model::PodPhase::Running,
        containers,
        default_container: with_udf.then(|| lit::<ContainerName>("udf")),
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
