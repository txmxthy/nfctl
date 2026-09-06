use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use super::VertexName;

/// What a vertex does. Derived from which of `source`/`sink`/`udf(.groupBy)` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VertexKind {
    Source,
    Sink,
    Map,
    Reduce,
}

impl VertexKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            VertexKind::Source => "source",
            VertexKind::Sink => "sink",
            VertexKind::Map => "map",
            VertexKind::Reduce => "reduce",
        }
    }
}

/// Autoscaling bounds for a vertex. `None` means "operator default".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScaleSpec {
    pub min: Option<u32>,
    pub max: Option<u32>,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vertex {
    pub name: VertexName,
    pub kind: VertexKind,
    /// Effective partition count (sources and non-keyed reduce are always 1).
    pub partitions: u32,
    pub scale: ScaleSpec,
    /// User-defined container image, when there is one.
    pub image: Option<String>,
}

/// How tags on a message are matched against an edge's condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagOperator {
    #[default]
    Or,
    And,
    Not,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCondition {
    pub operator: TagOperator,
    pub values: Vec<String>,
}

/// Writer behaviour when the downstream buffer is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnFull {
    #[default]
    RetryUntilSuccess,
    DiscardLatest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: VertexName,
    pub to: VertexName,
    pub conditions: Option<TagCondition>,
    pub on_full: OnFull,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TopologyError {
    #[error("no vertices")]
    Empty,
    #[error("duplicate vertex `{0}`")]
    DuplicateVertex(VertexName),
    #[error("edge `{from}` -> `{to}` references an unknown vertex")]
    DanglingEdge { from: VertexName, to: VertexName },
    #[error("duplicate edge `{from}` -> `{to}`")]
    DuplicateEdge { from: VertexName, to: VertexName },
    #[error("source `{0}` has an incoming edge")]
    EdgeIntoSource(VertexName),
    #[error("sink `{0}` has an outgoing edge")]
    EdgeOutOfSink(VertexName),
}

/// A pipeline's vertices and edges, validated on construction.
///
/// Numaflow permits cycles between UDF vertices, so this is a directed graph, not
/// strictly a DAG. Invariants: names unique, edges resolve, nothing flows into a
/// source or out of a sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TopologyRaw", into = "TopologyRaw")]
pub struct Topology {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
}

/// Unvalidated shape used only for serde.
#[derive(Serialize, Deserialize)]
struct TopologyRaw {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
}

impl TryFrom<TopologyRaw> for Topology {
    type Error = TopologyError;
    fn try_from(raw: TopologyRaw) -> Result<Self, TopologyError> {
        Topology::new(raw.vertices, raw.edges)
    }
}

impl From<Topology> for TopologyRaw {
    fn from(t: Topology) -> Self {
        TopologyRaw {
            vertices: t.vertices,
            edges: t.edges,
        }
    }
}

impl Topology {
    /// Validate and build. Vertex order is preserved (it is the spec order).
    pub fn new(vertices: Vec<Vertex>, edges: Vec<Edge>) -> Result<Self, TopologyError> {
        if vertices.is_empty() {
            return Err(TopologyError::Empty);
        }
        let mut seen: HashSet<&VertexName> = HashSet::new();
        for v in &vertices {
            if !seen.insert(&v.name) {
                return Err(TopologyError::DuplicateVertex(v.name.clone()));
            }
        }
        let kind_of = |n: &VertexName| vertices.iter().find(|v| &v.name == n).map(|v| v.kind);
        let mut seen_edges: BTreeSet<(&VertexName, &VertexName)> = BTreeSet::new();
        for e in &edges {
            let (Some(from_kind), Some(to_kind)) = (kind_of(&e.from), kind_of(&e.to)) else {
                return Err(TopologyError::DanglingEdge {
                    from: e.from.clone(),
                    to: e.to.clone(),
                });
            };
            if !seen_edges.insert((&e.from, &e.to)) {
                return Err(TopologyError::DuplicateEdge {
                    from: e.from.clone(),
                    to: e.to.clone(),
                });
            }
            if to_kind == VertexKind::Source {
                return Err(TopologyError::EdgeIntoSource(e.to.clone()));
            }
            if from_kind == VertexKind::Sink {
                return Err(TopologyError::EdgeOutOfSink(e.from.clone()));
            }
        }
        Ok(Self { vertices, edges })
    }

    #[must_use]
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    #[must_use]
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    #[must_use]
    pub fn vertex(&self, name: &VertexName) -> Option<&Vertex> {
        self.vertices.iter().find(|v| &v.name == name)
    }

    #[must_use]
    pub fn count(&self, kind: VertexKind) -> usize {
        self.vertices.iter().filter(|v| v.kind == kind).count()
    }

    /// What changes when `self` is replaced by `new`.
    #[must_use]
    pub fn diff(&self, new: &Topology) -> TopologyDiff {
        let mut d = TopologyDiff::default();
        for v in &new.vertices {
            match self.vertex(&v.name) {
                None => d.added_vertices.push(v.name.clone()),
                Some(old) => {
                    if old.kind != v.kind {
                        d.kind_changed.push((v.name.clone(), old.kind, v.kind));
                    }
                    if old.partitions != v.partitions {
                        d.partitions_changed
                            .push((v.name.clone(), old.partitions, v.partitions));
                    }
                    if old.image != v.image {
                        d.image_changed.push(v.name.clone());
                    }
                }
            }
        }
        for v in &self.vertices {
            if new.vertex(&v.name).is_none() {
                d.removed_vertices.push(v.name.clone());
            }
        }
        let key = |e: &Edge| (e.from.clone(), e.to.clone());
        for e in &new.edges {
            if !self.edges.iter().any(|o| key(o) == key(e)) {
                d.added_edges.push(key(e));
            }
        }
        for e in &self.edges {
            if !new.edges.iter().any(|n| key(n) == key(e)) {
                d.removed_edges.push(key(e));
            }
        }
        d
    }
}

/// Differences between two topologies, by category.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct TopologyDiff {
    pub added_vertices: Vec<VertexName>,
    pub removed_vertices: Vec<VertexName>,
    pub added_edges: Vec<(VertexName, VertexName)>,
    pub removed_edges: Vec<(VertexName, VertexName)>,
    pub kind_changed: Vec<(VertexName, VertexKind, VertexKind)>,
    pub partitions_changed: Vec<(VertexName, u32, u32)>,
    pub image_changed: Vec<VertexName>,
}

impl TopologyDiff {
    /// Any vertex or edge added or removed.
    #[must_use]
    pub fn shape_changed(&self) -> bool {
        !(self.added_vertices.is_empty()
            && self.removed_vertices.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn v(name: &str, kind: VertexKind) -> Vertex {
        Vertex {
            name: VertexName::new(name).unwrap(),
            kind,
            partitions: 1,
            scale: ScaleSpec::default(),
            image: None,
        }
    }

    pub(crate) fn e(from: &str, to: &str) -> Edge {
        Edge {
            from: VertexName::new(from).unwrap(),
            to: VertexName::new(to).unwrap(),
            conditions: None,
            on_full: OnFull::default(),
        }
    }

    #[test]
    fn builds_a_linear_pipeline() {
        let t = Topology::new(
            vec![
                v("in", VertexKind::Source),
                v("cat", VertexKind::Map),
                v("out", VertexKind::Sink),
            ],
            vec![e("in", "cat"), e("cat", "out")],
        )
        .unwrap();
        assert_eq!(t.count(VertexKind::Map), 1);
    }

    #[test]
    fn allows_udf_cycles() {
        let t = Topology::new(
            vec![
                v("in", VertexKind::Source),
                v("a", VertexKind::Map),
                v("out", VertexKind::Sink),
            ],
            vec![e("in", "a"), e("a", "a"), e("a", "out")],
        );
        assert!(t.is_ok());
    }

    #[test]
    fn diff_reports_each_category() {
        let old = Topology::new(
            vec![
                v("in", VertexKind::Source),
                v("a", VertexKind::Map),
                v("out", VertexKind::Sink),
            ],
            vec![e("in", "a"), e("a", "out")],
        )
        .unwrap();
        let mut a2 = v("a", VertexKind::Reduce);
        a2.partitions = 3;
        let new = Topology::new(
            vec![
                v("in", VertexKind::Source),
                a2,
                v("b", VertexKind::Map),
                v("out", VertexKind::Sink),
            ],
            vec![e("in", "a"), e("a", "b"), e("b", "out")],
        )
        .unwrap();
        let d = old.diff(&new);
        assert_eq!(d.added_vertices.len(), 1);
        assert!(d.removed_vertices.is_empty());
        assert_eq!(d.added_edges.len(), 2);
        assert_eq!(d.removed_edges.len(), 1);
        assert_eq!(d.kind_changed.len(), 1);
        assert_eq!(
            d.partitions_changed,
            vec![(VertexName::new("a").unwrap(), 1, 3)]
        );
        assert!(d.shape_changed());
        assert!(!old.diff(&old).shape_changed());
    }

    #[test]
    fn rejects_bad_shapes() {
        let src_sink = || vec![v("in", VertexKind::Source), v("out", VertexKind::Sink)];
        assert_eq!(Topology::new(vec![], vec![]), Err(TopologyError::Empty));
        assert!(matches!(
            Topology::new(
                vec![v("a", VertexKind::Map), v("a", VertexKind::Map)],
                vec![]
            ),
            Err(TopologyError::DuplicateVertex(_))
        ));
        assert!(matches!(
            Topology::new(src_sink(), vec![e("in", "nope")]),
            Err(TopologyError::DanglingEdge { .. })
        ));
        assert!(matches!(
            Topology::new(src_sink(), vec![e("in", "out"), e("in", "out")]),
            Err(TopologyError::DuplicateEdge { .. })
        ));
        assert!(matches!(
            Topology::new(src_sink(), vec![e("out", "in")]),
            Err(TopologyError::EdgeIntoSource(_))
        ));
        let m = vec![
            v("in", VertexKind::Source),
            v("m", VertexKind::Map),
            v("out", VertexKind::Sink),
        ];
        assert!(matches!(
            Topology::new(m, vec![e("in", "out"), e("out", "m")]),
            Err(TopologyError::EdgeOutOfSink(_))
        ));
    }
}
