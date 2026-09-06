use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::{ContainerName, PipelineName, PodName, Timestamp, VertexName};

/// Label and annotation keys Numaflow puts on the objects it manages. Public API.
pub mod labels {
    pub const PIPELINE_NAME: &str = "numaflow.numaproj.io/pipeline-name";
    pub const VERTEX_NAME: &str = "numaflow.numaproj.io/vertex-name";
    pub const MONO_VERTEX_NAME: &str = "numaflow.numaproj.io/mono-vertex-name";
    pub const ISBSVC_NAME: &str = "numaflow.numaproj.io/isbsvc-name";
    pub const INSTANCE: &str = "numaflow.numaproj.io/instance";
    pub const PAUSE_TIMESTAMP: &str = "numaflow.numaproj.io/pause-timestamp";
    pub const RESUME_STRATEGY: &str = "numaflow.numaproj.io/resume-strategy";
    pub const COMPONENT: &str = "app.kubernetes.io/component";
    pub const PART_OF: &str = "app.kubernetes.io/part-of";
    pub const MANAGED_BY: &str = "app.kubernetes.io/managed-by";
    pub const DEFAULT_CONTAINER: &str = "kubectl.kubernetes.io/default-container";

    pub const COMPONENT_VERTEX: &str = "vertex";
    pub const COMPONENT_DAEMON: &str = "daemon";
    pub const COMPONENT_ISBSVC: &str = "isbsvc";
    pub const COMPONENT_MONO_VERTEX: &str = "mono-vertex";

    /// The main container on every vertex pod.
    pub const MAIN_CONTAINER: &str = "numa";
}

/// An equality label selector. Built only through the constructors so the label
/// keys are never typed by hand at call sites.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Selector(BTreeMap<&'static str, String>);

impl Selector {
    /// Pods of every vertex in a pipeline, or of one vertex.
    #[must_use]
    pub fn vertex_pods(pipeline: &PipelineName, vertex: Option<&VertexName>) -> Self {
        let mut m = BTreeMap::new();
        m.insert(labels::COMPONENT, labels::COMPONENT_VERTEX.to_owned());
        m.insert(labels::PIPELINE_NAME, pipeline.to_string());
        if let Some(v) = vertex {
            m.insert(labels::VERTEX_NAME, v.to_string());
        }
        Self(m)
    }

    /// The pipeline's daemon pod(s).
    #[must_use]
    pub fn daemon_pods(pipeline: &PipelineName) -> Self {
        let mut m = BTreeMap::new();
        m.insert(labels::COMPONENT, labels::COMPONENT_DAEMON.to_owned());
        m.insert(labels::PIPELINE_NAME, pipeline.to_string());
        Self(m)
    }

    #[must_use]
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.0.iter().all(|(k, v)| labels.get(*k) == Some(v))
    }
}

impl fmt::Display for Selector {
    /// `k=v,k=v` in key order, as accepted by the Kubernetes API.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (k, v) in &self.0 {
            if !first {
                f.write_str(",")?;
            }
            first = false;
            write!(f, "{k}={v}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    #[default]
    Unknown,
}

impl PodPhase {
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "Pending" => PodPhase::Pending,
            "Running" => PodPhase::Running,
            "Succeeded" => PodPhase::Succeeded,
            "Failed" => PodPhase::Failed,
            _ => PodPhase::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerState {
    pub name: ContainerName,
    pub running: bool,
    pub restart_count: u32,
    pub is_init: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodRef {
    pub name: PodName,
    pub phase: PodPhase,
    pub containers: Vec<ContainerState>,
    /// `kubectl.kubernetes.io/default-container`, set on pods with a user container.
    pub default_container: Option<ContainerName>,
}

/// Pod watch events. Add and modify are collapsed; consumers key on the pod name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodEvent {
    Applied(PodRef),
    Deleted(PodRef),
    /// The watch is re-listing; a full set of `Applied` follows, then `ResyncDone`.
    Resync,
    ResyncDone,
}

/// How to open one container's log stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogOptions {
    pub follow: bool,
    /// Inclusive lower bound on line timestamps.
    pub since: Option<Timestamp>,
    /// Only the last N lines of the backlog.
    pub tail_lines: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub at: Option<Timestamp>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaggedLine {
    pub pod: PodName,
    pub container: ContainerName,
    pub line: LogLine,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_renders_in_key_order() {
        let s = Selector::vertex_pods(
            &PipelineName::new("p").unwrap(),
            Some(&VertexName::new("v").unwrap()),
        );
        assert_eq!(
            s.to_string(),
            "app.kubernetes.io/component=vertex,numaflow.numaproj.io/pipeline-name=p,numaflow.numaproj.io/vertex-name=v"
        );
    }
}
