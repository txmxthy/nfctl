use crossterm::event::KeyEvent;
use nfctl_core::model::{VertexName, WorkloadKey};

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
    /// A press, drag or release from the mouse.
    Mouse(crossterm::event::MouseEvent),
    Worker(WorkerReply),
}

/// What a panel asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Open whichever detail panel the workload's kind calls for.
    OpenDetail(WorkloadKey),
    OpenLogs(WorkloadKey, Option<VertexName>),
    Back,
}
