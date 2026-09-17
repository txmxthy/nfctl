use std::fmt;

use serde::{Deserialize, Serialize};

use super::{Condition, DesiredPhase, Namespace, PipelineName, Timestamp};

/// A `MonoVertex` name shares the pipeline name rules.
pub type MonoVertexName = PipelineName;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonoVertexKey {
    pub namespace: Namespace,
    pub name: MonoVertexName,
}

impl fmt::Display for MonoVertexKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MonoVertexPhase {
    #[default]
    Unknown,
    Running,
    Paused,
    Failed,
}

impl MonoVertexPhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MonoVertexPhase::Unknown => "Unknown",
            MonoVertexPhase::Running => "Running",
            MonoVertexPhase::Paused => "Paused",
            MonoVertexPhase::Failed => "Failed",
        }
    }

    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "Running" => MonoVertexPhase::Running,
            "Paused" => MonoVertexPhase::Paused,
            "Failed" => MonoVertexPhase::Failed,
            _ => MonoVertexPhase::Unknown,
        }
    }
}

/// A single-pod-per-replica source → (transformer) → (map) → sink unit.
///
/// The `has_*` flags are the internal container chain. They are structure, not
/// topology: the stages share one pod and one process, with no buffer and no
/// watermark between them, so they are never vertices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoVertex {
    pub key: MonoVertexKey,
    pub desired: DesiredPhase,
    pub phase: MonoVertexPhase,
    pub replicas: u32,
    pub desired_replicas: Option<u32>,
    pub ready_replicas: Option<u32>,
    pub has_transformer: bool,
    pub has_map: bool,
    /// The sink has a fallback, written to when the primary sink fails.
    pub has_fallback: bool,
    pub message: Option<String>,
    pub conditions: Vec<Condition>,
    pub created: Option<Timestamp>,
}
