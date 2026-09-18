use std::fmt::Write as _;

use nfctl_core::model::{Edge, OnFull, TagCondition, TagOperator, Topology, VertexKind};

use crate::view::ViewGraph;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    LeftRight,
    TopDown,
}

impl Direction {
    fn keyword(self) -> &'static str {
        match self {
            Direction::LeftRight => "LR",
            Direction::TopDown => "TD",
        }
    }
}

/// Node shape per vertex kind: `(open, close)` delimiters.
fn delims(kind: VertexKind) -> (&'static str, &'static str) {
    match kind {
        VertexKind::Source => ("([", "])"),
        VertexKind::Sink => ("[[", "]]"),
        VertexKind::Map => ("[", "]"),
        VertexKind::Reduce => ("{{", "}}"),
    }
}

/// Mermaid node ids may not contain `-`; vertex names are DNS labels, so this is
/// the only substitution needed and it cannot collide (`-` is the only non-alnum).
fn node_id(name: &str) -> String {
    name.replace('-', "_")
}

/// Quote a node label. Vertex names cannot contain quotes, so this is belt and braces.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "#quot;"))
}

/// Edge labels sit between `|` and are not quotable in every renderer, so strip
/// the two characters that would break the syntax.
fn edge_text(s: &str) -> String {
    s.replace(['|', '"'], "")
}

fn edge_label(c: &TagCondition) -> String {
    let joiner = match c.operator {
        TagOperator::Or => ", ",
        TagOperator::And | TagOperator::Not => " & ",
    };
    let body = edge_text(&c.values.join(joiner));
    match c.operator {
        TagOperator::Not => format!("not {body}"),
        _ => body,
    }
}

fn edge_line(e: &Edge) -> String {
    let arrow = match e.on_full {
        OnFull::RetryUntilSuccess => "-->",
        // Lossy edge: drawn dotted so it reads as "may drop".
        OnFull::DiscardLatest => "-.->",
    };
    let from = node_id(e.from.as_str());
    let to = node_id(e.to.as_str());
    match &e.conditions {
        Some(c) => format!("  {from} {arrow}|{}| {to}", edge_label(c)),
        None => format!("  {from} {arrow} {to}"),
    }
}

/// Emit a `flowchart`. Deterministic: vertices in spec order, edges in spec order.
#[must_use]
pub fn to_mermaid(t: &Topology, dir: Direction) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "flowchart {}", dir.keyword());
    for v in t.vertices() {
        let (open, close) = delims(v.kind);
        let _ = writeln!(
            out,
            "  {}{open}{}{close}",
            node_id(v.name.as_str()),
            quote(v.name.as_str())
        );
    }
    for e in t.edges() {
        out.push_str(&edge_line(e));
        out.push('\n');
    }
    out
}

/// Emit a `flowchart` of a view graph: shard groups are one node labelled
/// `stem ×N`. Edges keep the view graph's order, which the ASCII colouring
/// relies on.
#[must_use]
pub fn view_to_mermaid(g: &ViewGraph, dir: Direction) -> String {
    let id = |label: &str| node_id(&label.replace(" ×", "_x"));
    let mut out = String::new();
    let _ = writeln!(out, "flowchart {}", dir.keyword());
    for n in &g.nodes {
        let (open, close) = delims(n.kind);
        let _ = writeln!(out, "  {}{open}{}{close}", id(&n.label), quote(&n.label));
    }
    for e in &g.edges {
        let arrow = if e.lossy { "-.->" } else { "-->" };
        let from = id(&g.nodes[e.from.0 as usize].label);
        let to = id(&g.nodes[e.to.0 as usize].label);
        if e.tags.is_empty() {
            let _ = writeln!(out, "  {from} {arrow} {to}");
        } else {
            let _ = writeln!(out, "  {from} {arrow}|{}| {to}", edge_text(&e.label()));
        }
    }
    out
}
