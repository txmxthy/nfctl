use std::fmt::Write as _;

use nfctl_core::model::{OnFull, TagOperator, Topology, VertexKind};

fn shape(kind: VertexKind) -> &'static str {
    match kind {
        VertexKind::Source => "cds",
        VertexKind::Sink => "box3d",
        VertexKind::Map => "box",
        VertexKind::Reduce => "hexagon",
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Emit Graphviz DOT, left to right.
#[must_use]
pub fn to_dot(t: &Topology) -> String {
    let mut out = String::new();
    out.push_str("digraph pipeline {\n  rankdir=LR;\n  node [fontname=\"monospace\"];\n");
    for v in t.vertices() {
        let _ = writeln!(
            out,
            "  {} [shape={}];",
            quote(v.name.as_str()),
            shape(v.kind)
        );
    }
    for e in t.edges() {
        let mut attrs: Vec<String> = Vec::new();
        if let Some(c) = &e.conditions {
            let joiner = match c.operator {
                TagOperator::Or => " | ",
                TagOperator::And | TagOperator::Not => " & ",
            };
            let mut label = c.values.join(joiner);
            if c.operator == TagOperator::Not {
                label = format!("not {label}");
            }
            attrs.push(format!("label={}", quote(&label)));
        }
        if e.on_full == OnFull::DiscardLatest {
            attrs.push("style=dashed".to_owned());
        }
        let attr = if attrs.is_empty() {
            String::new()
        } else {
            format!(" [{}]", attrs.join(", "))
        };
        let _ = writeln!(
            out,
            "  {} -> {}{attr};",
            quote(e.from.as_str()),
            quote(e.to.as_str())
        );
    }
    out.push_str("}\n");
    out
}
