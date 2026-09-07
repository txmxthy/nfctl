//! Import Mermaid flowcharts into [`Topology`] values. Accepts what
//! [`crate::to_mermaid`] emits and the common `graph LR` + `subgraph` dialect
//! (one subgraph per pipeline). Anything it does not understand is skipped.

use std::collections::HashMap;

use nfctl_core::model::{
    Edge, OnFull, ScaleSpec, TagCondition, TagOperator, Topology, TopologyError, Vertex,
    VertexKind, VertexName,
};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a flowchart: first statement must be `graph` or `flowchart`")]
    NotAFlowchart,
    #[error("pipeline `{0}`: {1}")]
    Topology(String, TopologyError),
    #[error("pipeline `{0}`: vertex `{1}` has an invalid name")]
    BadName(String, String),
}

/// A node as declared in the source.
#[derive(Debug, Clone)]
struct Node {
    label: String,
    kind: Option<VertexKind>, // None = external/ghost, dropped with its edges
    group: Option<String>,
}

#[derive(Debug, Clone)]
struct Link {
    from: String,
    to: String,
    label: Option<String>,
    dotted: bool,
}

struct Parsed {
    nodes: HashMap<String, Node>,
    order: Vec<String>,
    links: Vec<Link>,
    groups: Vec<(String, String)>,
}

/// Parse a flowchart into one topology per subgraph (or one for the whole graph
/// when there are no subgraphs). Cross-subgraph, dotted and external edges are
/// dropped: a Numaflow pipeline is one subgraph.
pub fn from_mermaid(src: &str) -> Result<Vec<(String, Topology)>, ImportError> {
    build(parse(src)?)
}

fn parse(src: &str) -> Result<Parsed, ImportError> {
    let mut nodes: HashMap<String, Node> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut links: Vec<Link> = Vec::new();
    let mut groups: Vec<(String, String)> = Vec::new(); // (id, label) in order
    let mut stack: Vec<String> = Vec::new();
    let mut header_seen = false;

    let declare = |id: &str,
                   label: String,
                   kind: Option<VertexKind>,
                   group: Option<String>,
                   nodes: &mut HashMap<String, Node>,
                   order: &mut Vec<String>| {
        if !nodes.contains_key(id) {
            order.push(id.to_owned());
        }
        nodes
            .entry(id.to_owned())
            .or_insert(Node { label, kind, group });
    };

    for raw in src.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if !header_seen {
            let head = line
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if head != "graph" && head != "flowchart" {
                return Err(ImportError::NotAFlowchart);
            }
            header_seen = true;
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = line.strip_prefix("subgraph ") {
            let (id, label) = parse_subgraph(rest.trim());
            groups.push((id.clone(), label));
            stack.push(id);
            continue;
        }
        if lower == "end" {
            stack.pop();
            continue;
        }
        if [
            "style ",
            "classdef ",
            "class ",
            "linkstyle ",
            "direction ",
            "click ",
        ]
        .iter()
        .any(|k| lower.starts_with(k))
        {
            continue;
        }
        let group = stack.last().cloned();
        if let Some((src_tok, op, label, dst_tok)) = parse_edge(line) {
            let (sid, s_label, s_kind) = parse_node(src_tok);
            let (did, d_label, d_kind) = parse_node(dst_tok);
            declare(&sid, s_label, s_kind, group.clone(), &mut nodes, &mut order);
            declare(&did, d_label, d_kind, group.clone(), &mut nodes, &mut order);
            links.push(Link {
                from: sid,
                to: did,
                label,
                dotted: op.contains('.'),
            });
            continue;
        }
        let (id, label, kind) = parse_node(line);
        declare(&id, label, kind, group, &mut nodes, &mut order);
    }

    Ok(Parsed {
        nodes,
        order,
        links,
        groups,
    })
}

fn build(p: Parsed) -> Result<Vec<(String, Topology)>, ImportError> {
    let Parsed {
        nodes,
        order,
        links,
        groups,
    } = p;
    let group_list: Vec<(Option<String>, String)> = if groups.is_empty() {
        vec![(None, "pipeline".to_owned())]
    } else {
        groups
            .into_iter()
            .map(|(id, label)| (Some(id), label))
            .collect()
    };

    let mut out = Vec::new();
    for (gid, glabel) in group_list {
        let members: Vec<&String> = order.iter().filter(|id| nodes[*id].group == gid).collect();
        let mut vertices = Vec::new();
        let mut names: HashMap<&str, VertexName> = HashMap::new();
        for id in &members {
            let n = &nodes[*id];
            let Some(kind) = n.kind else { continue };
            let name = VertexName::new(vertex_name(&n.label))
                .map_err(|_| ImportError::BadName(glabel.clone(), n.label.clone()))?;
            names.insert(id.as_str(), name.clone());
            vertices.push(Vertex {
                name,
                kind,
                partitions: 1,
                scale: ScaleSpec::default(),
                image: None,
            });
        }
        if vertices.is_empty() {
            continue;
        }
        let mut edges = Vec::new();
        for l in &links {
            if l.dotted {
                continue;
            }
            let (Some(from), Some(to)) = (names.get(l.from.as_str()), names.get(l.to.as_str()))
            else {
                continue;
            };
            if edges.iter().any(|e: &Edge| &e.from == from && &e.to == to) {
                continue;
            }
            edges.push(Edge {
                from: from.clone(),
                to: to.clone(),
                conditions: l.label.as_deref().map(tags),
                on_full: OnFull::default(),
            });
        }
        let t =
            Topology::new(vertices, edges).map_err(|e| ImportError::Topology(glabel.clone(), e))?;
        out.push((glabel, t));
    }
    Ok(out)
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '%' if !in_quote && line[i..].starts_with("%%") => return &line[..i],
            _ => {}
        }
    }
    line
}

/// `id["label"]` or `"label"` or bare id.
fn parse_subgraph(rest: &str) -> (String, String) {
    if let Some((id, label)) = rest.split_once('[') {
        let label = label.trim_end_matches(']').trim_matches('"');
        return (id.trim().to_owned(), label.to_owned());
    }
    let s = rest.trim_matches('"');
    (s.to_owned(), s.to_owned())
}

/// `a --> b`, `a -->|x| b`, `a -.-> b`, `a -.->|x| b`; endpoints may be inline declarations.
fn parse_edge(line: &str) -> Option<(&str, &str, Option<String>, &str)> {
    let ops = ["-.->", "-->", "==>", "---"];
    let (idx, op) = ops
        .iter()
        .filter_map(|op| line.find(&format!(" {op}")).map(|i| (i, *op)))
        .min()?;
    let src = line[..idx].trim();
    let rest = line[idx + 1 + op.len()..].trim_start();
    let (label, dst) = if let Some(after) = rest.strip_prefix('|') {
        let (label, dst) = after.split_once('|')?;
        (Some(label.trim().to_owned()), dst.trim())
    } else {
        (None, rest.trim())
    };
    if src.is_empty() || dst.is_empty() {
        return None;
    }
    Some((src, op, label, dst))
}

/// Node token → (id, label, kind). Shapes: `([ ])` source, `[[ ]]` sink, `[ ]` map,
/// `{{ }}` reduce, `( )`/`(( ))` external (kind `None`). Bare id → map named by id.
fn parse_node(tok: &str) -> (String, String, Option<VertexKind>) {
    let shapes: [(&str, &str, Option<VertexKind>); 6] = [
        ("([", "])", Some(VertexKind::Source)),
        ("[[", "]]", Some(VertexKind::Sink)),
        ("((", "))", None),
        ("{{", "}}", Some(VertexKind::Reduce)),
        ("[", "]", Some(VertexKind::Map)),
        ("(", ")", None),
    ];
    for (open, close, kind) in shapes {
        if let Some(i) = tok.find(open)
            && tok.ends_with(close)
            && i > 0
        {
            let id = &tok[..i];
            let label = tok[i + open.len()..tok.len() - close.len()]
                .trim()
                .trim_matches('"');
            return (id.to_owned(), label.to_owned(), kind);
        }
    }
    (tok.to_owned(), tok.replace('_', "-"), Some(VertexKind::Map))
}

/// A label becomes a vertex name: lower-case, `_`→`-`, drop anything else.
fn vertex_name(label: &str) -> String {
    let mut s: String = label
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == '_' || c == ' ' { '-' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    while s.starts_with('-') {
        s.remove(0);
    }
    while s.ends_with('-') {
        s.pop();
    }
    s
}

fn tags(label: &str) -> TagCondition {
    let (operator, body) = match label.strip_prefix("not ") {
        Some(rest) => (TagOperator::Not, rest),
        None if label.contains(" & ") => (TagOperator::And, label),
        None => (TagOperator::Or, label),
    };
    let sep = if operator == TagOperator::And {
        " & "
    } else {
        ","
    };
    TagCondition {
        operator,
        values: body
            .split(sep)
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"graph LR
  subgraph p1__cluster["p1"]
    p1__in(["in"])
    p1__hot_0["hot-0"]
    p1__hot_1["hot-1"]
    p1__agg{{"agg"}}
    p1__out[["out"]]
    p1__in -->|t1-0| p1__hot_0
    p1__in -->|t1-1| p1__hot_1
    p1__hot_0 --> p1__agg
    p1__hot_1 --> p1__agg
    p1__agg -->|x1:v7, t2| p1__out
  end
  subgraph p2__cluster["p2"]
    p2__source(["source"])
    p2__sink[["sink"]]
    p2__source --> p2__sink
  end
  ext__1("ext1") -.-> p2__source
  p1__out -.->|topic1| p2__source
  %% comment
  style p1__in fill:#000
"#;

    #[test]
    fn imports_one_topology_per_subgraph() {
        let got = from_mermaid(SAMPLE).unwrap();
        assert_eq!(got.len(), 2);
        let (name, t) = &got[0];
        assert_eq!(name, "p1");
        assert_eq!(t.vertices().len(), 5);
        assert_eq!(t.edges().len(), 5);
        let agg = t.vertex(&VertexName::new("agg").unwrap()).unwrap();
        assert_eq!(agg.kind, VertexKind::Reduce);
        let last = &t.edges()[4];
        assert_eq!(
            last.conditions.as_ref().unwrap().values,
            vec!["x1:v7", "t2"]
        );
        assert_eq!(
            got[1].1.edges().len(),
            1,
            "dotted and external edges are dropped"
        );
    }

    #[test]
    fn round_trips_our_own_emitter() {
        let (_, t) = from_mermaid(SAMPLE).unwrap().remove(0);
        let text = crate::to_mermaid(&t, crate::Direction::LeftRight);
        let back = from_mermaid(&text).unwrap().remove(0).1;
        assert_eq!(back, t);
    }

    #[test]
    fn rejects_non_flowcharts() {
        assert!(matches!(
            from_mermaid("sequenceDiagram\n A->>B: hi"),
            Err(ImportError::NotAFlowchart)
        ));
    }
}
