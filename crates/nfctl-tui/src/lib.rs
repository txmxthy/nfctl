//! Terminal UI for `nfctl`.
//!
//! Layering follows the reference TUI this project studied: each panel is a
//! [`Model`] that turns an [`AppEvent`] into an optional [`Action`] plus work
//! for the [`worker`]; the worker owns all I/O on its own task and replies
//! over a channel. State lives only in the UI task, so there is no shared mutex.

mod app;
mod cards;
mod event;
pub mod panels;
mod style;
mod table;
mod worker;

pub use app::run;
pub use event::{Action, AppEvent};
pub use style::{CrossingStyle, Palette};
pub use worker::{WorkerMessage, WorkerReply};

use ratatui::Frame;
use ratatui::layout::Rect;

/// One screen of the UI.
pub trait Model {
    fn update(&mut self, ev: &AppEvent) -> (Option<Action>, Vec<WorkerMessage>);
    fn view(&self, frame: &mut Frame, area: Rect);
    /// Work to request when this panel becomes visible.
    fn on_enter(&mut self) -> Vec<WorkerMessage>;
}
