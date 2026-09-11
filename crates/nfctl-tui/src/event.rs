use crossterm::event::KeyEvent;
use nfctl_core::model::{PipelineKey, VertexName};

use crate::worker::WorkerReply;

/// Everything the UI loop reacts to.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    /// Time to ask the cluster again.
    Tick,
    /// Time to redraw, for anything that moves while waiting. Nothing is
    /// fetched on one of these.
    Frame,
    Worker(WorkerReply),
}

/// What a panel asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    OpenDetail(PipelineKey),
    OpenLogs(PipelineKey, Option<VertexName>),
    Back,
}
