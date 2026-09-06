//! `apply --check`: refuse changes the operator would reject, warn about ones
//! that are admitted but unsafe while messages are in flight.

use serde::Serialize;

use crate::model::{Pipeline, TopologyDiff};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ApplyReport {
    /// Would be rejected by the validating webhook, or requires delete-and-recreate.
    pub blocks: Vec<String>,
    /// Admitted, but risky with a backlog.
    pub warnings: Vec<String>,
    pub diff: Option<TopologyDiff>,
}

impl ApplyReport {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// Pure: compare the manifest against what is live. `backlog` is the total
/// pending across buffers when the daemon could report it.
#[must_use]
pub fn check(live: Option<&Pipeline>, new: &Pipeline, backlog: Option<i64>) -> ApplyReport {
    let mut r = ApplyReport::default();
    let Some(live) = live else {
        return r; // creating: nothing to conflict with
    };
    if live.spec.isb != new.spec.isb {
        r.blocks.push(format!(
            "interStepBufferServiceName changed ({} -> {}): immutable, delete and recreate",
            live.spec.isb, new.spec.isb
        ));
    }
    if live.meta.instance != new.meta.instance {
        r.blocks
            .push("numaflow.numaproj.io/instance annotation changed: immutable".to_owned());
    }
    let d = live.spec.topology.diff(&new.spec.topology);
    for (v, from, to) in &d.kind_changed {
        r.blocks.push(format!(
            "vertex `{v}` changes type {} -> {}: immutable",
            from.as_str(),
            to.as_str()
        ));
    }
    for (v, from, to) in &d.partitions_changed {
        let is_reduce = new
            .spec
            .topology
            .vertex(v)
            .is_some_and(|x| x.kind == crate::model::VertexKind::Reduce);
        if is_reduce {
            r.blocks.push(format!(
                "reduce vertex `{v}` changes partitions {from} -> {to}: immutable"
            ));
        } else {
            r.warnings.push(format!(
                "vertex `{v}` changes partitions {from} -> {to}; in-flight data may be re-keyed"
            ));
        }
    }
    let has_backlog = backlog.is_some_and(|n| n > 0);
    if d.shape_changed() {
        let msg = format!(
            "topology changes (+{} -{} vertices, +{} -{} edges) are admitted but unprocessed messages may be lost{}",
            d.added_vertices.len(),
            d.removed_vertices.len(),
            d.added_edges.len(),
            d.removed_edges.len(),
            match backlog {
                Some(n) if n > 0 => format!("; {n} messages pending now"),
                Some(_) => "; buffers are empty now".to_owned(),
                None => "; backlog unknown (daemon unreachable)".to_owned(),
            }
        );
        r.warnings.push(msg);
    }
    if !d.image_changed.is_empty() && has_backlog {
        r.warnings.push(format!(
            "image changes on {} with {} messages pending: the new image must handle the backlog",
            d.image_changed
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            backlog.unwrap_or(0)
        ));
    }
    r.diff = Some(d);
    r
}

#[cfg(all(test, feature = "fake"))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::fake::sample_pipeline;
    use crate::model::{IsbName, PipelinePhase, VertexKind, VertexName};

    #[test]
    fn create_is_always_ok() {
        let new = sample_pipeline("ns", "p", PipelinePhase::Unknown);
        assert!(check(None, &new, None).ok());
    }

    #[test]
    fn immutable_fields_block() {
        let live = sample_pipeline("ns", "p", PipelinePhase::Running);
        let mut new = live.clone();
        new.spec.isb = IsbName::new("other").unwrap();
        new.meta.instance = Some("x".into());
        let r = check(Some(&live), &new, Some(0));
        assert_eq!(r.blocks.len(), 2, "{r:?}");
        assert!(!r.ok());
    }

    #[test]
    fn topology_change_warns_and_mentions_backlog() {
        let live = sample_pipeline("ns", "p", PipelinePhase::Running);
        let mut new = live.clone();
        let mut vs: Vec<_> = new.spec.topology.vertices().to_vec();
        let mut es: Vec<_> = new.spec.topology.edges().to_vec();
        let mut extra = vs[1].clone();
        extra.name = VertexName::new("extra").unwrap();
        extra.kind = VertexKind::Map;
        vs.push(extra);
        es.push(crate::model::Edge {
            from: VertexName::new("cat").unwrap(),
            to: VertexName::new("extra").unwrap(),
            conditions: None,
            on_full: crate::model::OnFull::default(),
        });
        new.spec.topology = crate::model::Topology::new(vs, es).unwrap();
        let r = check(Some(&live), &new, Some(17));
        assert!(r.ok());
        assert_eq!(r.warnings.len(), 1);
        assert!(
            r.warnings[0].contains("17 messages pending"),
            "{}",
            r.warnings[0]
        );
    }
}
