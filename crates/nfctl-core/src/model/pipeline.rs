use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{IsbName, PipelineKey, Timestamp, Topology, VertexKind, duration_secs};

/// `spec.lifecycle.desiredPhase`. Only these two are meaningful to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesiredPhase {
    Running,
    Paused,
}

impl DesiredPhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DesiredPhase::Running => "Running",
            DesiredPhase::Paused => "Paused",
        }
    }
}

/// `status.phase` as reported by the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PipelinePhase {
    #[default]
    Unknown,
    Running,
    Paused,
    Failed,
    Pausing,
    Deleting,
}

impl PipelinePhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PipelinePhase::Unknown => "Unknown",
            PipelinePhase::Running => "Running",
            PipelinePhase::Paused => "Paused",
            PipelinePhase::Failed => "Failed",
            PipelinePhase::Pausing => "Pausing",
            PipelinePhase::Deleting => "Deleting",
        }
    }

    /// Parse the controller's string. Unknown strings become `Unknown` rather than an
    /// error: a newer operator must not break `ls`.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "Running" => PipelinePhase::Running,
            "Paused" => PipelinePhase::Paused,
            "Failed" => PipelinePhase::Failed,
            "Pausing" => PipelinePhase::Pausing,
            "Deleting" => PipelinePhase::Deleting,
            _ => PipelinePhase::Unknown,
        }
    }
}

/// Value of the `numaflow.numaproj.io/resume-strategy` annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeStrategy {
    /// Restore the replica counts from before the pause (operator default).
    #[default]
    Fast,
    /// Come back at `scale.min` (or 1) and let autoscaling ramp up.
    Slow,
}

impl ResumeStrategy {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ResumeStrategy::Fast => "fast",
            ResumeStrategy::Slow => "slow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    pub desired: DesiredPhase,
    #[serde(with = "duration_secs")]
    pub pause_grace: Duration,
    #[serde(with = "duration_secs")]
    pub deletion_grace: Duration,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            desired: DesiredPhase::Running,
            pause_grace: Duration::from_secs(30),
            deletion_grace: Duration::from_secs(30),
        }
    }
}

/// `spec.limits` with operator defaults applied. `buffer_usage_limit` is a fraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub read_batch_size: u64,
    pub buffer_max_length: u64,
    pub buffer_usage_limit: super::Fraction,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            read_batch_size: 500,
            buffer_max_length: 30_000,
            buffer_usage_limit: super::Fraction::new(0.8).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineSpec {
    pub isb: IsbName,
    pub lifecycle: Lifecycle,
    pub limits: Limits,
    pub topology: Topology,
}

/// One `status.conditions[]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub kind: String,
    pub ok: bool,
    pub reason: Option<String>,
    pub message: Option<String>,
}

/// Vertex counts as reported in status (may lag the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VertexCounts {
    pub total: u32,
    pub sources: u32,
    pub sinks: u32,
    pub udfs: u32,
    pub map_udfs: u32,
    pub reduce_udfs: u32,
}

impl VertexCounts {
    /// Derive counts from a topology (used by fakes and for spec/status drift checks).
    #[must_use]
    pub fn from_topology(t: &Topology) -> Self {
        let c = |k| u32::try_from(t.count(k)).unwrap_or(u32::MAX);
        let map = c(VertexKind::Map);
        let reduce = c(VertexKind::Reduce);
        Self {
            total: u32::try_from(t.vertices().len()).unwrap_or(u32::MAX),
            sources: c(VertexKind::Source),
            sinks: c(VertexKind::Sink),
            udfs: map + reduce,
            map_udfs: map,
            reduce_udfs: reduce,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PipelineStatus {
    pub phase: PipelinePhase,
    pub message: Option<String>,
    pub conditions: Vec<Condition>,
    pub observed_generation: Option<i64>,
    /// Only meaningful while paused.
    pub drained_on_pause: bool,
    pub counts: VertexCounts,
    /// When the controller started the pause (from its annotation).
    pub pause_started: Option<Timestamp>,
    pub last_updated: Option<Timestamp>,
}

/// The slice of `metadata` the tool cares about.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub generation: Option<i64>,
    pub created: Option<Timestamp>,
    /// `numaflow.numaproj.io/instance`, when set.
    pub instance: Option<String>,
    pub resume_strategy: Option<ResumeStrategy>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pipeline {
    pub key: PipelineKey,
    pub meta: ObjectMeta,
    pub spec: PipelineSpec,
    pub status: PipelineStatus,
}

impl Pipeline {
    /// Age relative to `now`, if the creation timestamp is known.
    #[must_use]
    pub fn age(&self, now: Timestamp) -> Option<Duration> {
        self.meta.created?.elapsed_until(now)
    }
}
