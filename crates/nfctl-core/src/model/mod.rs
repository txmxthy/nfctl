//! Domain types. Parse, don't validate: every identifier is a newtype that can
//! only be built from a valid string; every closed set is an enum.

mod ids;
mod isb;
mod monovertex;
mod pipeline;
mod pods;
mod runtime;
mod time;
mod topology;

pub use ids::{
    BufferName, ContainerName, InvalidName, IsbName, Namespace, PipelineKey, PipelineName, PodName,
    VertexName,
};
pub use isb::{IsbPhase, IsbService};
pub use monovertex::{MonoVertex, MonoVertexKey, MonoVertexName, MonoVertexPhase};
pub use pipeline::{
    Condition, DesiredPhase, Lifecycle, Limits, ObjectMeta, Pipeline, PipelinePhase, PipelineSpec,
    PipelineStatus, ResumeStrategy, VertexCounts,
};
pub use pods::{
    ContainerState, LogLine, LogOptions, PodEvent, PodPhase, PodRef, Selector, TaggedLine, labels,
};
pub use runtime::{
    BufferInfo, ContainerError, EdgeWatermark, Fraction, Health, PipelineHealth, ReplicaErrors,
    VertexMetrics, Windows,
};
pub use time::{Timestamp, duration_secs};
pub use topology::{
    Edge, OnFull, ScaleSpec, TagCondition, TagOperator, Topology, TopologyDiff, TopologyError,
    Vertex, VertexKind,
};
