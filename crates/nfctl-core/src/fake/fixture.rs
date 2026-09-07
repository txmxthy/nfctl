//! A fixture file: the whole world the fakes answer from. One format feeds the
//! CLI's `--fixture`, the TUI's golden-frame tests and the README recordings.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{
    BufferInfo, ContainerName, EdgeWatermark, IsbService, LogLine, MonoVertex, Pipeline,
    PipelineHealth, PipelineName, PodName, PodRef, VertexMetrics,
};

/// Runtime data the daemon reports for one pipeline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonFixture {
    pub health: Option<PipelineHealth>,
    #[serde(default)]
    pub buffers: Vec<BufferInfo>,
    #[serde(default)]
    pub metrics: Vec<VertexMetrics>,
    #[serde(default)]
    pub watermarks: Vec<EdgeWatermark>,
}

/// A pod and the log lines each of its containers replays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodFixture {
    #[serde(flatten)]
    pub pod: PodRef,
    /// Lines per container, replayed on every open (so `-f` and the TUI see them).
    #[serde(default)]
    pub logs: HashMap<ContainerName, Vec<LogLine>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    #[serde(default)]
    pub pipelines: Vec<Pipeline>,
    #[serde(default)]
    pub monovertices: Vec<MonoVertex>,
    #[serde(default)]
    pub isbs: Vec<IsbService>,
    /// Keyed by pipeline (or `MonoVertex`) name.
    #[serde(default)]
    pub daemons: HashMap<PipelineName, DaemonFixture>,
    #[serde(default)]
    pub pods: Vec<PodFixture>,
}

impl Fixture {
    /// Parse YAML or JSON.
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_yaml_ng::from_str(text).map_err(|e| e.to_string())
    }

    pub(crate) fn pod_logs(
        &self,
        pod: &PodName,
        container: &ContainerName,
    ) -> Option<Vec<LogLine>> {
        self.pods
            .iter()
            .find(|p| &p.pod.name == pod)
            .and_then(|p| p.logs.get(container).cloned())
    }
}
