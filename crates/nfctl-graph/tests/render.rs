#![allow(clippy::unwrap_used)]
//! Golden renders for representative topologies.

use nfctl_core::model::{
    Edge, OnFull, ScaleSpec, TagCondition, TagOperator, Topology, Vertex, VertexKind, VertexName,
};
use nfctl_graph::{Format, render};

fn v(name: &str, kind: VertexKind) -> Vertex {
    Vertex {
        name: VertexName::new(name).unwrap(),
        kind,
        partitions: 1,
        scale: ScaleSpec::default(),
        image: None,
    }
}

fn e(from: &str, to: &str) -> Edge {
    Edge {
        from: VertexName::new(from).unwrap(),
        to: VertexName::new(to).unwrap(),
        conditions: None,
        on_full: OnFull::default(),
    }
}

fn tagged(from: &str, to: &str, op: TagOperator, values: &[&str]) -> Edge {
    Edge {
        conditions: Some(TagCondition {
            operator: op,
            values: values.iter().map(|s| (*s).to_owned()).collect(),
        }),
        ..e(from, to)
    }
}

/// `in -> cat -> out`.
fn linear() -> Topology {
    Topology::new(
        vec![
            v("in", VertexKind::Source),
            v("cat", VertexKind::Map),
            v("out", VertexKind::Sink),
        ],
        vec![e("in", "cat"), e("cat", "out")],
    )
    .unwrap()
}

/// Conditional fan-out to two sinks plus a default sink, like the upstream even-odd example.
fn even_odd() -> Topology {
    Topology::new(
        vec![
            v("in", VertexKind::Source),
            v("even-or-odd", VertexKind::Map),
            v("even-sink", VertexKind::Sink),
            v("odd-sink", VertexKind::Sink),
            v("number-sink", VertexKind::Sink),
        ],
        vec![
            e("in", "even-or-odd"),
            tagged("even-or-odd", "even-sink", TagOperator::Or, &["even-tag"]),
            tagged("even-or-odd", "odd-sink", TagOperator::Or, &["odd-tag"]),
            tagged(
                "even-or-odd",
                "number-sink",
                TagOperator::Or,
                &["even-tag", "odd-tag"],
            ),
        ],
    )
    .unwrap()
}

/// Two sources, a reduce, a lossy edge and a self-loop.
fn complex() -> Topology {
    let mut lossy = e("enrich", "alerts");
    lossy.on_full = OnFull::DiscardLatest;
    Topology::new(
        vec![
            v("kafka-in", VertexKind::Source),
            v("http-in", VertexKind::Source),
            v("decode", VertexKind::Map),
            v("enrich", VertexKind::Map),
            v("window-agg", VertexKind::Reduce),
            v("warehouse", VertexKind::Sink),
            v("alerts", VertexKind::Sink),
            v("dlq", VertexKind::Sink),
        ],
        vec![
            e("kafka-in", "decode"),
            e("http-in", "decode"),
            tagged("decode", "enrich", TagOperator::Or, &["ok"]),
            tagged("decode", "dlq", TagOperator::Not, &["ok"]),
            e("enrich", "enrich"),
            e("enrich", "window-agg"),
            lossy,
            e("window-agg", "warehouse"),
        ],
    )
    .unwrap()
}

#[test]
fn mermaid() {
    insta::assert_snapshot!("linear_mermaid", render(&linear(), Format::Mermaid, None));
    insta::assert_snapshot!(
        "even_odd_mermaid",
        render(&even_odd(), Format::Mermaid, None)
    );
    insta::assert_snapshot!("complex_mermaid", render(&complex(), Format::Mermaid, None));
}

#[test]
fn dot() {
    insta::assert_snapshot!("complex_dot", render(&complex(), Format::Dot, None));
}

#[test]
fn ascii() {
    insta::assert_snapshot!("linear_ascii", render(&linear(), Format::Ascii, None));
    insta::assert_snapshot!("even_odd_ascii", render(&even_odd(), Format::Ascii, None));
    insta::assert_snapshot!("complex_ascii", render(&complex(), Format::Ascii, None));
}

/// Asking for less width shrinks the drawing through a ladder of tighter
/// styles. `complex` bottoms out at 72 columns: the layout never flips to
/// top-down and never clips a box, so a width below the floor gets the floor.
#[test]
fn ascii_fits_width() {
    let wide = render(&complex(), Format::Ascii, None);
    let narrow = render(&complex(), Format::Ascii, Some(60));
    let max = |s: &str| s.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    assert!(max(&narrow) < max(&wide));
    assert!(max(&narrow) <= 72, "narrow render is {} cols", max(&narrow));
    insta::assert_snapshot!("complex_ascii_narrow", narrow);
}

fn sharded() -> Topology {
    Topology::new(
        vec![
            v("in", VertexKind::Source),
            v("router", VertexKind::Map),
            v("worker-0", VertexKind::Map),
            v("worker-1", VertexKind::Map),
            v("merge", VertexKind::Map),
            v("out", VertexKind::Sink),
            v("audit", VertexKind::Sink),
        ],
        vec![
            e("in", "router"),
            tagged("router", "worker-0", TagOperator::Or, &["shard-0"]),
            tagged("router", "worker-1", TagOperator::Or, &["shard-1"]),
            e("worker-0", "merge"),
            e("worker-1", "merge"),
            e("merge", "out"),
            tagged("router", "audit", TagOperator::Or, &["audit"]),
        ],
    )
    .unwrap()
}

#[test]
fn shards_collapse_unless_expanded() {
    use nfctl_graph::{RenderOptions, render_with};
    let collapsed = RenderOptions {
        collapse_shards: true,
        ..RenderOptions::default()
    };
    insta::assert_snapshot!(
        "sharded_mermaid_collapsed",
        render_with(&sharded(), Format::Mermaid, collapsed)
    );
    insta::assert_snapshot!(
        "sharded_ascii_collapsed",
        render_with(&sharded(), Format::Ascii, collapsed)
    );
    let expanded = render_with(&sharded(), Format::Ascii, RenderOptions::default());
    assert_eq!(expanded, render(&sharded(), Format::Ascii, None));
    assert!(expanded.contains("worker-1") && !expanded.contains("×2"));
}

/// Colour adds SGR sequences around edge runs and nothing else.
#[test]
fn ascii_colour_is_only_escapes() {
    use nfctl_graph::{RenderOptions, render_with};
    let opts = RenderOptions {
        colour: true,
        collapse_shards: true,
        ..RenderOptions::default()
    };
    let coloured = render_with(&sharded(), Format::Ascii, opts);
    let plain = render_with(
        &sharded(),
        Format::Ascii,
        RenderOptions {
            colour: false,
            ..opts
        },
    );
    let stripped: String = {
        let mut out = String::new();
        let mut chars = coloured.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    let trim = |s: &str| s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    assert_eq!(trim(&stripped), trim(&plain));
    // Two tag combinations, two hues; untagged edges are dim.
    assert!(coloured.contains("\x1b[36m") && coloured.contains("\x1b[35m"));
    assert!(coloured.contains("\x1b[2m"));
}

/// A crowded graph whose long tag labels used to be corrupted: the renderer
/// drew each label while routing, so a later edge painted its line or
/// arrowhead through an earlier label, and a label whose anchor sat one column
/// left of another was cut off after a single character — which read as the
/// next label's first letter doubled (`ddlq-target:…`).
fn crowded_tags() -> Topology {
    let tag = |n: &str| format!("dlq-target:alpha-forwarder-{n}");
    let long = |from: &str, to: &str, vs: &[String]| Edge {
        conditions: Some(TagCondition {
            operator: TagOperator::Or,
            values: vs.to_vec(),
        }),
        ..e(from, to)
    };
    let mut verts = vec![v("node-0", VertexKind::Source)];
    verts.extend((1..7).map(|i| v(&format!("node-{i}"), VertexKind::Map)));
    verts.push(v("node-7", VertexKind::Sink));
    Topology::new(
        verts,
        vec![
            e("node-0", "node-1"),
            e("node-0", "node-2"),
            e("node-1", "node-3"),
            e("node-0", "node-4"),
            e("node-1", "node-5"),
            e("node-1", "node-6"),
            e("node-6", "node-7"),
            long("node-3", "node-4", &[tag("3400"), tag("3401")]),
            long("node-6", "node-5", &[tag("6500")]),
            long("node-2", "node-7", &[tag("2700")]),
            long("node-1", "node-7", &[tag("1700")]),
        ],
    )
    .unwrap()
}

/// Box-drawing glyphs and label text never share a cell: no line runs through
/// a label, and no label is left as a one-character stub against another.
#[test]
fn edge_labels_are_never_overdrawn() {
    const BOX: &str = "─│┌┐└┘├┤┬┴┼╌╎╭╮╰╯━┃╴╶▶◀▲▼";

    let out = render(&crowded_tags(), Format::Ascii, Some(100));
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == ':' || c == ',';
    for line in out.lines() {
        let cs: Vec<char> = line.chars().collect();
        for i in 1..cs.len().saturating_sub(1) {
            assert!(
                !(BOX.contains(cs[i]) && word(cs[i - 1]) && word(cs[i + 1])),
                "line drawn through a label at column {i}:\n{out}"
            );
        }
        assert!(
            !line.contains("ddlq-target:"),
            "one-character label stub against its neighbour:\n{out}"
        );
    }
    insta::assert_snapshot!("crowded_tags_ascii", out);
}
