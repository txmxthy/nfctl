use orthodag::{Options, Part};

use crate::view::{ViewGraph, to_graph};

/// SGR foreground codes for the six palette slots, in `orthodag::Colour` slot
/// order.
const SGR: [&str; orthodag::PALETTE as usize] = ["36", "35", "33", "32", "34", "31"];
const DIM: &str = "2";
const RESET: &str = "\x1b[0m";

/// Render a view graph as box-drawing text with its edge tags written on the
/// edges, fitted to `width` columns when given. An empty view renders as
/// nothing.
///
/// With `colour`, every run of line, arrowhead or tag carries the ANSI hue of
/// its tag combination and untagged edges are dim; boxes and their text stay
/// uncoloured, so the output with escapes removed equals the plain render.
#[must_use]
pub(crate) fn render_ascii(view: &ViewGraph, width: Option<usize>, colour: bool) -> String {
    if view.nodes.is_empty() {
        return String::new();
    }
    let graph = to_graph(view);
    let mut options = Options::new().labels(true);
    if let Some(width) = width {
        options = options.width(width);
    }
    if !colour {
        return orthodag::draw_with(&graph, options);
    }

    let mut out = String::new();
    for row in orthodag::spans_with(&graph, options) {
        for span in row {
            match (span.part, span.colour) {
                (Part::Flow, ink) => {
                    let sgr = ink.map_or(DIM, |c| SGR[usize::from(c.slot()) % SGR.len()]);
                    out.push_str("\x1b[");
                    out.push_str(sgr);
                    out.push('m');
                    out.push_str(&span.text);
                    out.push_str(RESET);
                }
                (Part::Frame, _) => out.push_str(&span.text),
            }
        }
        out.push('\n');
    }
    out
}
