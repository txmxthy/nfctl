//! The two things that vary between environments, as traits. Adapters implement
//! them; services and UIs only ever see `dyn ClusterPort` / `dyn DaemonPort`.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::Result;
use crate::model::LogOptions;
use crate::model::{
    BufferInfo, BufferName, ContainerName, DesiredPhase, EdgeWatermark, IsbService, LogLine,
    MonoVertex, MonoVertexKey, Namespace, Pipeline, PipelineHealth, PipelineKey, PodEvent, PodName,
    PodRef, ReplicaErrors, ResumeStrategy, Selector, Topology, VertexMetrics, VertexName,
};

/// Kubernetes: CRDs, pods and logs.
#[async_trait]
pub trait ClusterPort: Send + Sync {
    /// All namespaces when `ns` is `None`.
    async fn list_pipelines(&self, ns: Option<&Namespace>) -> Result<Vec<Pipeline>>;
    async fn get_pipeline(&self, key: &PipelineKey) -> Result<Pipeline>;
    async fn watch_pipeline(
        &self,
        key: &PipelineKey,
    ) -> Result<BoxStream<'static, Result<Pipeline>>>;

    /// Merge-patch `spec.lifecycle.desiredPhase` and, for resume, the strategy annotation.
    async fn set_lifecycle(
        &self,
        key: &PipelineKey,
        desired: DesiredPhase,
        resume: Option<ResumeStrategy>,
        dry_run: bool,
    ) -> Result<()>;

    /// Parse a Pipeline manifest (YAML or JSON) into the domain without touching the
    /// cluster. `default_ns` applies when the manifest has no namespace.
    fn parse_manifest(&self, text: &str, default_ns: &Namespace) -> Result<Pipeline>;

    /// Server-side apply of a Pipeline manifest (field manager `nfctl`).
    async fn apply_manifest(
        &self,
        text: &str,
        default_ns: &Namespace,
        dry_run: bool,
    ) -> Result<Pipeline>;

    /// Set a vertex's replica count through the scale subresource.
    async fn scale_vertex(
        &self,
        key: &PipelineKey,
        vertex: &VertexName,
        replicas: u32,
        dry_run: bool,
    ) -> Result<()>;

    /// All namespaces when `ns` is `None`.
    async fn list_isb(&self, ns: Option<&Namespace>) -> Result<Vec<IsbService>>;

    async fn list_monovertices(&self, ns: Option<&Namespace>) -> Result<Vec<MonoVertex>>;
    async fn get_monovertex(&self, key: &MonoVertexKey) -> Result<MonoVertex>;
    /// Pause, or resume. Resuming clears `spec.replicas` so autoscaling takes over
    /// again (`MonoVertex` has no resume-strategy annotation).
    async fn set_monovertex_lifecycle(
        &self,
        key: &MonoVertexKey,
        desired: DesiredPhase,
        dry_run: bool,
    ) -> Result<()>;

    async fn list_pods(&self, ns: &Namespace, selector: &Selector) -> Result<Vec<PodRef>>;
    async fn watch_pods(
        &self,
        ns: &Namespace,
        selector: &Selector,
    ) -> Result<BoxStream<'static, Result<PodEvent>>>;
    async fn delete_pods(
        &self,
        ns: &Namespace,
        selector: &Selector,
        dry_run: bool,
    ) -> Result<Vec<PodName>>;
    /// Delete one pod by name (the controller replaces it).
    async fn delete_pod(&self, ns: &Namespace, pod: &PodName, dry_run: bool) -> Result<()>;

    /// One container's log. With `follow`, the stream stays open until the
    /// container terminates; without, it ends after the current backlog.
    async fn tail_logs(
        &self,
        ns: &Namespace,
        pod: &PodName,
        container: &ContainerName,
        opts: &LogOptions,
    ) -> Result<BoxStream<'static, Result<LogLine>>>;
}

/// One pipeline's daemon. Bound to a pipeline at connect time.
#[async_trait]
pub trait DaemonPort: Send + Sync {
    async fn buffers(&self) -> Result<Vec<BufferInfo>>;
    async fn buffer(&self, name: &BufferName) -> Result<BufferInfo>;
    /// All vertices when `vertex` is `None`.
    async fn vertex_metrics(&self, vertex: Option<&VertexName>) -> Result<Vec<VertexMetrics>>;
    async fn watermarks(&self) -> Result<Vec<EdgeWatermark>>;
    async fn health(&self) -> Result<PipelineHealth>;
    async fn vertex_errors(&self, vertex: &VertexName) -> Result<Vec<ReplicaErrors>>;
}

/// Abstract factory: obtain a [`DaemonPort`] for a pipeline. Cheap and lazy; the
/// adapter connects on first use.
#[async_trait]
pub trait DaemonConnector: Send + Sync {
    /// `topology` saves the connector a second read of the pipeline when the
    /// caller already has it; buffers are attributed to edges with it.
    async fn connect(
        &self,
        key: &PipelineKey,
        topology: Option<&Topology>,
    ) -> Result<Arc<dyn DaemonPort>>;

    /// A `MonoVertex`'s daemon: `vertex_metrics` and `health` work; buffers and
    /// watermarks do not exist for a `MonoVertex` and return an error.
    async fn connect_monovertex(&self, key: &MonoVertexKey) -> Result<Arc<dyn DaemonPort>>;
}
