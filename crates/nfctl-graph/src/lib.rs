//! Render a [`Topology`] as Mermaid, DOT, or box-drawing text.
//!
//! The Mermaid and DOT emitters are pure functions of the domain model. The
//! text renderer is the orthodag crate, fed the same `ViewGraph` the Mermaid
//! emitter reads: one view, two pictures.

mod ascii;
mod dot;
pub mod layout;
mod mermaid;
mod mermaid_in;

pub use dot::to_dot;
pub use mermaid::{Direction, to_mermaid, view_to_mermaid};
pub use mermaid_in::{ImportError, from_mermaid};

use nfctl_core::model::Topology;

/// What the CLI's `dag --format` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ascii,
    Mermaid,
    Dot,
}

/// How `render_with` draws.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Fit the ASCII render to this many columns.
    pub width: Option<usize>,
    /// Draw `stem-0..stem-N` shard groups as one node (Mermaid and ASCII).
    pub collapse_shards: bool,
    /// ANSI-colour ASCII edges by tag combination.
    pub colour: bool,
}

/// Render in the requested format. `width` only affects `Ascii`. Every vertex
/// is its own node and nothing is coloured; see [`render_with`].
#[must_use]
pub fn render(topology: &Topology, format: Format, width: Option<usize>) -> String {
    render_with(
        topology,
        format,
        RenderOptions {
            width,
            ..RenderOptions::default()
        },
    )
}

/// Render with shard collapsing and colour. DOT ignores both: it is meant
/// for Graphviz, which lays out the full graph itself.
#[must_use]
pub fn render_with(topology: &Topology, format: Format, opts: RenderOptions) -> String {
    if format == Format::Dot {
        return to_dot(topology);
    }
    let view = if opts.collapse_shards {
        layout::ViewGraph::collapsed(topology)
    } else {
        layout::ViewGraph::expanded(topology)
    };
    match format {
        Format::Mermaid | Format::Dot => view_to_mermaid(&view, Direction::LeftRight),
        Format::Ascii => ascii::render_ascii(&view, opts.width, opts.colour),
    }
}
