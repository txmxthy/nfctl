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

#[test]
fn ascii_fits_width() {
    let wide = render(&complex(), Format::Ascii, None);
    let narrow = render(&complex(), Format::Ascii, Some(60));
    let max = |s: &str| s.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    assert!(max(&narrow) <= max(&wide));
    assert!(max(&narrow) <= 72, "narrow render is {} cols", max(&narrow));
    insta::assert_snapshot!("complex_ascii_narrow", narrow);
}
