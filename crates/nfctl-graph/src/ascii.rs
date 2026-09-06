use orthodag::Options;
use orthodag::io::mermaid_in::from_mermaid;

/// Render Mermaid source as box-drawing text with its edge tags on the edges,
/// fitted to `width` columns when given. Blank or unreadable input renders as
/// nothing.
#[must_use]
pub fn render_ascii(mermaid: &str, width: Option<usize>) -> String {
    let Some((_, graph)) = from_mermaid(mermaid).ok().and_then(|mut g| g.drain(..).next()) else {
        return String::new();
    };
    let mut options = Options::new().labels(true);
    if let Some(width) = width {
        options = options.width(width);
    }
    let mut s = orthodag::draw_with(&graph, options);
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}
