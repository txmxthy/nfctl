//! A Pipeline and a `MonoVertex` are different Kubernetes resources but the same
//! thing to an operator: something named in a namespace that runs, pauses and
//! fails. [`Workload`] is that shared surface, with the kind kept alongside so
//! the ports can still reach the right resource.

use std::fmt;
use std::time::Duration;

use serde::Serialize;

use super::{
    DesiredPhase, IsbName, MonoVertex, MonoVertexKey, MonoVertexPhase, Namespace, Pipeline,
    PipelineKey, PipelineName, PipelinePhase, Timestamp, VertexCounts,
};

/// Which Numaflow resource a workload is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum WorkloadKind {
    Pipeline,
    MonoVertex,
}

impl WorkloadKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WorkloadKind::Pipeline => "Pipeline",
            WorkloadKind::MonoVertex => "MonoVertex",
        }
    }
}

impl fmt::Display for WorkloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a workload is, and which kind it is. Displays as `namespace/name`: the
/// kind is a column, not part of the operator's name for the thing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct WorkloadKey {
    pub kind: WorkloadKind,
    pub namespace: Namespace,
    pub name: PipelineName,
}

impl WorkloadKey {
    #[must_use]
    pub fn new(kind: WorkloadKind, namespace: Namespace, name: PipelineName) -> Self {
        Self {
            kind,
            namespace,
            name,
        }
    }

    /// The pipeline key, when this names a pipeline.
    #[must_use]
    pub fn as_pipeline(&self) -> Option<PipelineKey> {
        (self.kind == WorkloadKind::Pipeline)
            .then(|| PipelineKey::new(self.namespace.clone(), self.name.clone()))
    }

    /// The `MonoVertex` key, when this names a `MonoVertex`.
    #[must_use]
    pub fn as_monovertex(&self) -> Option<MonoVertexKey> {
        (self.kind == WorkloadKind::MonoVertex).then(|| MonoVertexKey {
            namespace: self.namespace.clone(),
            name: self.name.clone(),
        })
    }
}

impl From<&PipelineKey> for WorkloadKey {
    fn from(k: &PipelineKey) -> Self {
        Self::new(WorkloadKind::Pipeline, k.namespace.clone(), k.name.clone())
    }
}

impl From<&MonoVertexKey> for WorkloadKey {
    fn from(k: &MonoVertexKey) -> Self {
        Self::new(
            WorkloadKind::MonoVertex,
            k.namespace.clone(),
            k.name.clone(),
        )
    }
}

impl fmt::Display for WorkloadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// The union of the two controllers' phases. A `MonoVertex` never reports
/// `Pausing` or `Deleting`; a reader does not have to know which can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum WorkloadPhase {
    #[default]
    Unknown,
    Running,
    Paused,
    Failed,
    Pausing,
    Deleting,
}

impl WorkloadPhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WorkloadPhase::Unknown => "Unknown",
            WorkloadPhase::Running => "Running",
            WorkloadPhase::Paused => "Paused",
            WorkloadPhase::Failed => "Failed",
            WorkloadPhase::Pausing => "Pausing",
            WorkloadPhase::Deleting => "Deleting",
        }
    }
}

impl From<PipelinePhase> for WorkloadPhase {
    fn from(p: PipelinePhase) -> Self {
        match p {
            PipelinePhase::Unknown => WorkloadPhase::Unknown,
            PipelinePhase::Running => WorkloadPhase::Running,
            PipelinePhase::Paused => WorkloadPhase::Paused,
            PipelinePhase::Failed => WorkloadPhase::Failed,
            PipelinePhase::Pausing => WorkloadPhase::Pausing,
            PipelinePhase::Deleting => WorkloadPhase::Deleting,
        }
    }
}

impl From<MonoVertexPhase> for WorkloadPhase {
    fn from(p: MonoVertexPhase) -> Self {
        match p {
            MonoVertexPhase::Unknown => WorkloadPhase::Unknown,
            MonoVertexPhase::Running => WorkloadPhase::Running,
            MonoVertexPhase::Paused => WorkloadPhase::Paused,
            MonoVertexPhase::Failed => WorkloadPhase::Failed,
        }
    }
}

/// One listable, gettable Numaflow resource. Serialises with a `kind` field, so
/// `-o json` says what each entry is without a reader having to guess.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Workload {
    Pipeline(Box<Pipeline>),
    MonoVertex(MonoVertex),
}

impl Workload {
    #[must_use]
    pub fn kind(&self) -> WorkloadKind {
        match self {
            Workload::Pipeline(_) => WorkloadKind::Pipeline,
            Workload::MonoVertex(_) => WorkloadKind::MonoVertex,
        }
    }

    #[must_use]
    pub fn key(&self) -> WorkloadKey {
        match self {
            Workload::Pipeline(p) => WorkloadKey::from(&p.key),
            Workload::MonoVertex(m) => WorkloadKey::from(&m.key),
        }
    }

    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        match self {
            Workload::Pipeline(p) => &p.key.namespace,
            Workload::MonoVertex(m) => &m.key.namespace,
        }
    }

    #[must_use]
    pub fn name(&self) -> &PipelineName {
        match self {
            Workload::Pipeline(p) => &p.key.name,
            Workload::MonoVertex(m) => &m.key.name,
        }
    }

    #[must_use]
    pub fn phase(&self) -> WorkloadPhase {
        match self {
            Workload::Pipeline(p) => p.status.phase.into(),
            Workload::MonoVertex(m) => m.phase.into(),
        }
    }

    #[must_use]
    pub fn desired(&self) -> DesiredPhase {
        match self {
            Workload::Pipeline(p) => p.spec.lifecycle.desired,
            Workload::MonoVertex(m) => m.desired,
        }
    }

    /// How many vertices the workload runs. A `MonoVertex` is one by definition.
    #[must_use]
    pub fn vertices(&self) -> u32 {
        match self {
            Workload::Pipeline(p) => p.status.counts.total,
            Workload::MonoVertex(_) => 1,
        }
    }

    /// Per-kind vertex counts, which only a pipeline has.
    #[must_use]
    pub fn counts(&self) -> Option<VertexCounts> {
        match self {
            Workload::Pipeline(p) => Some(p.status.counts),
            Workload::MonoVertex(_) => None,
        }
    }

    /// Pods running now, which only a `MonoVertex` reports at this level.
    #[must_use]
    pub fn replicas(&self) -> Option<(u32, Option<u32>)> {
        match self {
            Workload::Pipeline(_) => None,
            Workload::MonoVertex(m) => Some((m.replicas, m.ready_replicas)),
        }
    }

    /// The inter-step buffer service, which a `MonoVertex` does not use.
    #[must_use]
    pub fn isb(&self) -> Option<&IsbName> {
        match self {
            Workload::Pipeline(p) => Some(&p.spec.isb),
            Workload::MonoVertex(_) => None,
        }
    }

    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            Workload::Pipeline(p) => p.status.message.as_deref(),
            Workload::MonoVertex(m) => m.message.as_deref(),
        }
    }

    #[must_use]
    pub fn age(&self, now: Timestamp) -> Option<Duration> {
        match self {
            Workload::Pipeline(p) => p.age(now),
            Workload::MonoVertex(m) => m.created?.elapsed_until(now),
        }
    }
}

impl From<Pipeline> for Workload {
    fn from(p: Pipeline) -> Self {
        Workload::Pipeline(Box::new(p))
    }
}

impl From<MonoVertex> for Workload {
    fn from(m: MonoVertex) -> Self {
        Workload::MonoVertex(m)
    }
}
