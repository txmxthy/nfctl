//! The graph the layout sees: vertices, with shard groups collapsed.

use std::collections::{BTreeMap, BTreeSet};

use nfctl_core::model::{
    Edge, OnFull, TagCondition, TagOperator, Topology, VertexKind, VertexName,
};

/// A node of the view graph: its position in `nodes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewNode {
    /// `name`, or `stem ×N` for a shard group.
    pub label: String,
    pub kind: VertexKind,
    pub members: Vec<VertexName>,
    pub partitions: u32,
}

impl ViewNode {
    #[must_use]
    pub fn is_group(&self) -> bool {
        self.members.len() > 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewEdge {
    pub from: NodeId,
    pub to: NodeId,
    /// Condition values (`not` rendered as a prefix), empty when unconditional.
    pub tags: Vec<String>,
    /// Drops on a full buffer (`discardLatest`) on any member edge.
    pub lossy: bool,
    /// Underlying topology edges (one, or one per shard member pair).
    pub members: Vec<(VertexName, VertexName)>,
}

impl ViewEdge {
    /// The tags as one label, empty when unconditional.
    #[must_use]
    pub fn label(&self) -> String {
        self.tags.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewGraph {
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
}

fn tag_strings(c: Option<&TagCondition>) -> Vec<String> {
    let Some(c) = c else { return Vec::new() };
    match c.operator {
        TagOperator::Not => c.values.iter().map(|v| format!("not {v}")).collect(),
        TagOperator::And => vec![c.values.join(" & ")],
        TagOperator::Or => c.values.clone(),
    }
}

/// Kind, partitions, inbound and outbound neighbourhoods outside the group.
type Neighbours = BTreeSet<(String, Vec<String>)>;
type Signature = (VertexKind, u32, Neighbours, Neighbours);

/// Tags on a shard member's edges often carry the member's own index
/// (`route-2` into `worker-2`); those become `route-*` so the group matches.
fn norm_tags(tags: Vec<String>, idx: Option<u32>) -> Vec<String> {
    let Some(idx) = idx else { return tags };
    let suffix = format!("-{idx}");
    tags.into_iter()
        .map(|t| {
            t.strip_suffix(&suffix)
                .map_or(t.clone(), |stem| format!("{stem}-*"))
        })
        .collect()
}

/// `stem-<digits>` → `(stem, index)`.
fn shard_split(name: &str) -> Option<(&str, u32)> {
    let (stem, idx) = name.rsplit_once('-')?;
    (!stem.is_empty() && !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()))
        .then(|| idx.parse().ok().map(|i| (stem, i)))
        .flatten()
}

impl ViewGraph {
    /// Every vertex is its own node.
    #[must_use]
    pub fn expanded(t: &Topology) -> Self {
        Self::build(t, |_, _| None)
    }

    /// Vertices `stem-N` with the same stem, kind, partitions and identical
    /// outside neighbourhoods collapse into one node `stem ×N`.
    #[must_use]
    pub fn collapsed(t: &Topology) -> Self {
        // Candidate groups by stem.
        let mut by_stem: BTreeMap<&str, Vec<&VertexName>> = BTreeMap::new();
        for v in t.vertices() {
            if let Some((stem, _)) = shard_split(v.name.as_str()) {
                by_stem.entry(stem).or_default().push(&v.name);
            }
        }
        let candidate_stem = |n: &VertexName| -> Option<String> {
            let (s, _) = shard_split(n.as_str())?;
            by_stem
                .get(s)
                .filter(|m| m.len() >= 2)
                .map(|_| s.to_owned())
        };
        let outside = |n: &VertexName| candidate_stem(n).unwrap_or_else(|| n.as_str().to_owned());
        let signature = |n: &VertexName| -> Signature {
            let v = t
                .vertex(n)
                .map_or((VertexKind::Map, 1), |v| (v.kind, v.partitions));
            let idx = shard_split(n.as_str()).map(|(_, i)| i);
            let tags = |e: &Edge| norm_tags(tag_strings(e.conditions.as_ref()), idx);
            let ins = t
                .edges()
                .iter()
                .filter(|e| &e.to == n)
                .map(|e| (outside(&e.from), tags(e)))
                .collect();
            let outs = t
                .edges()
                .iter()
                .filter(|e| &e.from == n)
                .map(|e| (outside(&e.to), tags(e)))
                .collect();
            (v.0, v.1, ins, outs)
        };
        let mut accepted: BTreeMap<&str, Vec<VertexName>> = BTreeMap::new();
        for (stem, members) in &by_stem {
            if members.len() < 2 {
                continue;
            }
            let sig0 = signature(members[0]);
            if members.iter().all(|m| signature(m) == sig0) {
                let mut ms: Vec<VertexName> = members.iter().map(|m| (*m).clone()).collect();
                ms.sort_by_key(|m| shard_split(m.as_str()).map_or(0, |(_, i)| i));
                accepted.insert(stem, ms);
            }
        }
        Self::build(t, |name, _| {
            let (stem, _) = shard_split(name.as_str())?;
            accepted.get(stem).map(|ms| (stem.to_owned(), ms.clone()))
        })
    }

    fn build(
        t: &Topology,
        group_of: impl Fn(&VertexName, &Topology) -> Option<(String, Vec<VertexName>)>,
    ) -> Self {
        let mut nodes: Vec<ViewNode> = Vec::new();
        let mut node_of: BTreeMap<VertexName, NodeId> = BTreeMap::new();
        for v in t.vertices() {
            if node_of.contains_key(&v.name) {
                continue;
            }
            let id = NodeId(u32::try_from(nodes.len()).unwrap_or(u32::MAX));
            let (label, members) = group_of(&v.name, t).map_or_else(
                || (v.name.to_string(), vec![v.name.clone()]),
                |(stem, ms)| (format!("{stem} ×{}", ms.len()), ms),
            );
            for m in &members {
                node_of.insert(m.clone(), id);
            }
            nodes.push(ViewNode {
                label,
                kind: v.kind,
                members,
                partitions: v.partitions,
            });
        }
        let mut edges: Vec<ViewEdge> = Vec::new();
        for e in t.edges() {
            let (Some(&from), Some(&to)) = (node_of.get(&e.from), node_of.get(&e.to)) else {
                continue;
            };
            let mut tags = tag_strings(e.conditions.as_ref());
            for end in [&e.from, &e.to] {
                if nodes[node_of[end].0 as usize].is_group() {
                    tags = norm_tags(tags, shard_split(end.as_str()).map(|(_, i)| i));
                }
            }
            if from == to && nodes[from.0 as usize].is_group() && e.from != e.to {
                continue; // intra-group edge between shard members
            }
            let lossy = e.on_full == OnFull::DiscardLatest;
            if let Some(existing) = edges
                .iter_mut()
                .find(|x| x.from == from && x.to == to && x.tags == tags)
            {
                existing.lossy |= lossy;
                existing.members.push((e.from.clone(), e.to.clone()));
            } else {
                edges.push(ViewEdge {
                    from,
                    to,
                    tags,
                    lossy,
                    members: vec![(e.from.clone(), e.to.clone())],
                });
            }
        }
        Self { nodes, edges }
    }

    /// The node a vertex belongs to.
    #[must_use]
    pub fn node_of(&self, name: &VertexName) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|n| n.members.contains(name))
            .map(|i| NodeId(u32::try_from(i).unwrap_or(u32::MAX)))
    }
}

/// The view graph as orthodag sees it.
///
/// Nodes and edges are added in order, so `orthodag::colour::of` lines up with
/// `edges` and, for an expanded view, with `Topology::edges()`.
#[must_use]
pub fn to_graph(g: &ViewGraph) -> orthodag::Graph {
    let mut out = orthodag::Graph::new();
    let ids: Vec<_> = g
        .nodes
        .iter()
        .map(|n| out.add_node(orthodag::Node::new(&n.label)))
        .collect();
    for e in &g.edges {
        let (Some(from), Some(to)) = (ids.get(e.from.0 as usize), ids.get(e.to.0 as usize)) else {
            continue;
        };
        let Ok(_) = out.add_tagged_edge(*from, *to, e.tags.iter().cloned()) else {
            continue;
        };
    }
    out
}
