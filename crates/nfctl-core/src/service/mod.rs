//! Use cases. Everything the CLI and TUI do goes through here, so both share one
//! implementation of every multi-step Numaflow procedure.

mod pipeline;

pub use pipeline::PipelineService;

use async_trait::async_trait;

use crate::model::PipelineKey;
use crate::ports::{DaemonConnector, DaemonPort};
use crate::{Error, Result};

/// Null object: a connector for builds without a daemon adapter. Every connect
/// fails with a clear error, so read-only commands keep working.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoDaemon;

#[async_trait]
impl DaemonConnector for NoDaemon {
    async fn connect(&self, key: &PipelineKey) -> Result<Box<dyn DaemonPort>> {
        Err(Error::Daemon(
            format!("no daemon adapter configured for {key}").into(),
        ))
    }
}
