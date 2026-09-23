//! Use cases. Everything the CLI and TUI do goes through here, so both share one
//! implementation of every multi-step Numaflow procedure.

pub mod apply;
pub mod lifecycle;
pub mod logs;
mod pipeline;
pub mod status;

pub use apply::{ApplyReport, check};
pub use lifecycle::{PauseReport, RecycleReport, pause, recycle, resume, wait_for_phase};
pub use logs::{ContainerSelect, TailHandle, TailOptions};
pub use pipeline::PipelineService;
pub use status::{
    EdgeView, MonoVertexView, PipelineView, Timings, VertexView, monovertex_view, pipeline_numbers,
    pipeline_shape, pipeline_view, total_pending,
};

use std::sync::Arc;

use async_trait::async_trait;

use crate::model::{PipelineKey, Topology};
use crate::ports::{DaemonConnector, DaemonPort};
use crate::{Error, Result};

/// Null object: a connector for builds without a daemon adapter. Every connect
/// fails with a clear error, so read-only commands keep working.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoDaemon;

#[async_trait]
impl DaemonConnector for NoDaemon {
    async fn connect(
        &self,
        key: &PipelineKey,
        _topology: Option<&Topology>,
    ) -> Result<Arc<dyn DaemonPort>> {
        Err(Error::Daemon(
            format!("no daemon adapter configured for {key}").into(),
        ))
    }

    async fn connect_monovertex(
        &self,
        key: &crate::model::MonoVertexKey,
    ) -> Result<Arc<dyn DaemonPort>> {
        Err(Error::Daemon(
            format!("no daemon adapter configured for {key}").into(),
        ))
    }
}
