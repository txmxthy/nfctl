//! The two things that vary between environments, as traits. Adapters implement
//! them; services and UIs only ever see `dyn ClusterPort` / `dyn DaemonPort`.

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::Result;
use crate::model::LogOptions;
use crate::model::{
    BufferInfo, BufferName, ContainerName, DesiredPhase, EdgeWatermark, IsbService, LogLine,
    Namespace, Pipeline, PipelineHealth, PipelineKey, PodEvent, PodName, PodRef, ReplicaErrors,
    ResumeStrategy, Selector, VertexMetrics, VertexName,
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

    async fn list_isb(&self, ns: &Namespace) -> Result<Vec<IsbService>>;

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
    async fn connect(&self, key: &PipelineKey) -> Result<Box<dyn DaemonPort>>;
}
