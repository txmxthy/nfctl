use orthodag::io::mermaid_in::from_mermaid;
use orthodag::{Graph, Options};

/// The graph in Mermaid source, with the options `dag` draws it at.
fn read(mermaid: &str, width: Option<usize>) -> Option<(Graph, Options)> {
    let (_, graph) = from_mermaid(mermaid).ok()?.drain(..).next()?;
    let mut options = Options::new().labels(true);
    if let Some(width) = width {
        options = options.width(width);
    }
    Some((graph, options))
}

/// Render Mermaid source as box-drawing text with its edge tags on the edges,
/// fitted to `width` columns when given. Blank or unreadable input renders as
/// nothing.
#[must_use]
pub fn render_ascii(mermaid: &str, width: Option<usize>) -> String {
    let Some((graph, options)) = read(mermaid, width) else {
        return String::new();
    };
    let mut s = orthodag::draw_with(&graph, options);
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// SGR foreground codes for the six palette slots, in `orthodag::Colour` slot
/// order.
const SGR: [&str; orthodag::PALETTE as usize] = ["36", "35", "33", "32", "34", "31"];
const DIM: &str = "2";
const RESET: &str = "\x1b[0m";

/// Like [`render_ascii`], with ANSI colour on edges: every run of line,
/// arrowhead or tag takes the hue of its tag combination and untagged edges
/// are dim. Boxes and their text stay uncoloured, so the output without
/// escapes equals [`render_ascii`].
#[must_use]
pub fn render_ascii_coloured(mermaid: &str, width: Option<usize>) -> String {
    let Some((graph, options)) = read(mermaid, width) else {
        return String::new();
    };
    let mut out = String::new();
    for row in orthodag::spans_with(&graph, options) {
        for span in row {
            match span.colour {
                Some(c) => {
                    out.push_str("\x1b[");
                    out.push_str(SGR[usize::from(c.slot()) % SGR.len()]);
                    out.push('m');
                    out.push_str(&span.text);
                    out.push_str(RESET);
                }
                None if span.text.chars().any(|ch| "─│┌┐└┘├┤┬┴┼╴╶▶▲".contains(ch)) => {
                    out.push_str("\x1b[");
                    out.push_str(DIM);
                    out.push('m');
                    out.push_str(&span.text);
                    out.push_str(RESET);
                }
                None => out.push_str(&span.text),
            }
        }
        out.push('\n');
    }
    out
}
