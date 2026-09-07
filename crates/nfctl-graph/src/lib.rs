//! Render a [`Topology`] as Mermaid, DOT, or box-drawing text.
//!
//! The Mermaid and DOT emitters are pure functions of the domain model. The
//! text renderer is the orthodag crate, fed with the Mermaid we emit: one
//! emitter, two consumers.

mod ascii;
mod dot;
mod mermaid;
mod mermaid_in;

pub use ascii::render_ascii;
pub use dot::to_dot;
pub use mermaid::{Direction, to_mermaid};
pub use mermaid_in::{ImportError, from_mermaid};

use nfctl_core::model::Topology;

/// What the CLI's `dag --format` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ascii,
    Mermaid,
    Dot,
}

/// Render in the requested format. `width` only affects `Ascii`.
#[must_use]
pub fn render(topology: &Topology, format: Format, width: Option<usize>) -> String {
    match format {
        Format::Mermaid => to_mermaid(topology, Direction::LeftRight),
        Format::Dot => to_dot(topology),
        Format::Ascii => render_ascii(&to_mermaid(topology, Direction::LeftRight), width),
    }
}
