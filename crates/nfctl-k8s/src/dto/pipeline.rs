use std::time::Duration;

use kube::core::Object;
use nfctl_core::Error;
use nfctl_core::model::{
    Condition, DesiredPhase, Edge, Fraction, IsbName, Lifecycle, Limits, Namespace, ObjectMeta,
    OnFull, Pipeline, PipelineKey, PipelineName, PipelinePhase, PipelineSpec, PipelineStatus,
    ResumeStrategy, ScaleSpec, TagCondition, TagOperator, Timestamp, Topology, Vertex,
    VertexCounts, VertexKind, VertexName, labels,
};
use serde::{Deserialize, Serialize};

/// `kube` object type for the Pipeline CRD.
pub type PipelineObject = Object<PipelineSpecDto, PipelineStatusDto>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSpecDto {
    pub inter_step_buffer_service_name: Option<String>,
    #[serde(default)]
    pub vertices: Vec<VertexDto>,
    #[serde(default)]
    pub edges: Vec<EdgeDto>,
    #[serde(default)]
    pub lifecycle: LifecycleDto,
    #[serde(default)]
    pub limits: LimitsDto,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleDto {
    pub desired_phase: Option<String>,
    pub pause_grace_period_seconds: Option<u64>,
    pub deletion_grace_period_seconds: Option<u64>,
    /// Deprecated spelling still emitted by the operator's defaults.
    pub delete_grace_period_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsDto {
    pub read_batch_size: Option<u64>,
    pub buffer_max_length: Option<u64>,
    /// Percentage here; the daemon reports a fraction.
    pub buffer_usage_limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexDto {
    pub name: String,
    /// Presence decides the kind; the contents are not modelled.
    pub source: Option<serde_json::Value>,
    pub sink: Option<serde_json::Value>,
    pub udf: Option<UdfDto>,
    pub partitions: Option<u32>,
    pub scale: Option<ScaleDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UdfDto {
    pub container: Option<ContainerDto>,
    pub group_by: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerDto {
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScaleDto {
    pub min: Option<u32>,
    pub max: Option<u32>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeDto {
    pub from: String,
    pub to: String,
    pub conditions: Option<ConditionsDto>,
    pub on_full: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConditionsDto {
    pub tags: Option<TagsDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagsDto {
    pub operator: Option<String>,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStatusDto {
    pub phase: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub conditions: Vec<ConditionDto>,
    pub observed_generation: Option<i64>,
    #[serde(default)]
    pub drained_on_pause: bool,
    pub vertex_count: Option<u32>,
    pub source_count: Option<u32>,
    pub sink_count: Option<u32>,
    pub udf_count: Option<u32>,
    #[serde(rename = "mapUDFCount")]
    pub map_udf_count: Option<u32>,
    #[serde(rename = "reduceUDFCount")]
    pub reduce_udf_count: Option<u32>,
    pub last_updated: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionDto {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

fn invalid(name: &str, reason: impl std::fmt::Display) -> Error {
    Error::Invalid {
        kind: "pipeline",
        name: name.to_owned(),
        reason: reason.to_string(),
    }
}

impl TryFrom<VertexDto> for Vertex {
    type Error = String;

    fn try_from(d: VertexDto) -> Result<Self, String> {
        let name = VertexName::new(&d.name).map_err(|e| format!("vertex `{}`: {e}", d.name))?;
        if d.partitions == Some(0) {
            return Err(format!("vertex `{}` partitions must be at least 1", d.name));
        }
        let (kind, image) = match (&d.source, &d.sink, &d.udf) {
            (Some(_), None, None) => (VertexKind::Source, None),
            (None, Some(_), None) => (VertexKind::Sink, None),
            (None, None, Some(u)) => {
                let kind = if u.group_by.is_some() {
                    VertexKind::Reduce
                } else {
                    VertexKind::Map
                };
                (kind, u.container.as_ref().and_then(|c| c.image.clone()))
            }
            _ => {
                return Err(format!(
                    "vertex `{}` must set exactly one of source, sink or udf",
                    d.name
                ));
            }
        };
        // Sources are always single-partition; Numaflow ignores the field for them.
        let partitions = match kind {
            VertexKind::Source => 1,
            _ => d.partitions.unwrap_or(1),
        };
        let scale = d
            .scale
            .map(|s| ScaleSpec {
                min: s.min,
                max: s.max,
                disabled: s.disabled,
            })
            .unwrap_or_default();
        Ok(Vertex {
            name,
            kind,
            partitions,
            scale,
            image,
        })
    }
}

impl TryFrom<EdgeDto> for Edge {
    type Error = String;

    fn try_from(d: EdgeDto) -> Result<Self, String> {
        let from = VertexName::new(&d.from).map_err(|e| format!("edge from `{}`: {e}", d.from))?;
        let to = VertexName::new(&d.to).map_err(|e| format!("edge to `{}`: {e}", d.to))?;
        let conditions = d
            .conditions
            .and_then(|c| c.tags)
            .map(|t| {
                let operator = match t.operator.as_deref() {
                    None | Some("or") => TagOperator::Or,
                    Some("and") => TagOperator::And,
                    Some("not") => TagOperator::Not,
                    Some(other) => return Err(format!("unknown tag operator `{other}`")),
                };
                Ok(TagCondition {
                    operator,
                    values: t.values,
                })
            })
            .transpose()?;
        let on_full = match d.on_full.as_deref() {
            Some("discardLatest") => OnFull::DiscardLatest,
            None | Some("retryUntilSuccess") => OnFull::RetryUntilSuccess,
            Some(other) => return Err(format!("unknown onFull policy `{other}`")),
        };
        Ok(Edge {
            from,
            to,
            conditions,
            on_full,
        })
    }
}

fn lifecycle_from(lc: &LifecycleDto) -> Result<Lifecycle, String> {
    Ok(Lifecycle {
        desired: match lc.desired_phase.as_deref() {
            Some("Paused") => DesiredPhase::Paused,
            None | Some("Running") => DesiredPhase::Running,
            Some(other) => return Err(format!("unknown desired phase `{other}`")),
        },
        pause_grace: Duration::from_secs(lc.pause_grace_period_seconds.unwrap_or(30)),
        deletion_grace: Duration::from_secs(
            lc.deletion_grace_period_seconds
                .or(lc.delete_grace_period_seconds)
                .unwrap_or(30),
        ),
    })
}

fn limits_from(l: &LimitsDto) -> Limits {
    let defaults = Limits::default();
    Limits {
        read_batch_size: l.read_batch_size.unwrap_or(defaults.read_batch_size),
        buffer_max_length: l.buffer_max_length.unwrap_or(defaults.buffer_max_length),
        buffer_usage_limit: l
            .buffer_usage_limit
            .and_then(|p| Fraction::from_percent(f64::from(p)))
            .unwrap_or(defaults.buffer_usage_limit),
    }
}

fn status_from(st: PipelineStatusDto, pause_started: Option<Timestamp>) -> PipelineStatus {
    PipelineStatus {
        phase: st
            .phase
            .as_deref()
            .map(PipelinePhase::parse_lenient)
            .unwrap_or_default(),
        message: st.message.filter(|m| !m.is_empty()),
        conditions: st
            .conditions
            .into_iter()
            .map(|c| Condition {
                kind: c.kind,
                ok: c.status == "True",
                reason: c.reason,
                message: c.message,
            })
            .collect(),
        observed_generation: st.observed_generation,
        drained_on_pause: st.drained_on_pause,
        counts: VertexCounts {
            total: st.vertex_count.unwrap_or(0),
            sources: st.source_count.unwrap_or(0),
            sinks: st.sink_count.unwrap_or(0),
            udfs: st.udf_count.unwrap_or(0),
            map_udfs: st.map_udf_count.unwrap_or(0),
            reduce_udfs: st.reduce_udf_count.unwrap_or(0),
        },
        pause_started,
        last_updated: st.last_updated.as_ref().and_then(super::to_timestamp),
    }
}

/// Convert the wire object into the domain, validating everything on the way.
/// (A free function: both types are foreign to this crate, so `TryFrom` is not allowed.)
pub fn into_pipeline(o: PipelineObject) -> Result<Pipeline, Error> {
    let raw_name = o.metadata.name.clone().unwrap_or_default();
    let name = PipelineName::new(&raw_name).map_err(|e| invalid(&raw_name, e))?;
    let namespace = Namespace::new(o.metadata.namespace.clone().unwrap_or_default())
        .map_err(|e| invalid(&raw_name, format!("namespace: {e}")))?;
    let annotations = o.metadata.annotations.clone().unwrap_or_default();

    let vertices = o
        .spec
        .vertices
        .into_iter()
        .map(Vertex::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(&raw_name, e))?;
    let edges = o
        .spec
        .edges
        .into_iter()
        .map(Edge::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(&raw_name, e))?;
    let topology = Topology::new(vertices, edges).map_err(|e| invalid(&raw_name, e))?;

    let isb_raw = o
        .spec
        .inter_step_buffer_service_name
        .unwrap_or_else(|| "default".to_owned());
    let isb = IsbName::new(&isb_raw).map_err(|e| invalid(&raw_name, format!("isb: {e}")))?;

    let pause_started = annotations
        .get(labels::PAUSE_TIMESTAMP)
        .and_then(|s| Timestamp::parse_rfc3339(s).ok());
    let resume_strategy = match annotations.get(labels::RESUME_STRATEGY).map(String::as_str) {
        None => None,
        Some("slow") => Some(ResumeStrategy::Slow),
        Some("fast") => Some(ResumeStrategy::Fast),
        Some(other) => {
            return Err(invalid(
                &raw_name,
                format!("unknown resume strategy `{other}`"),
            ));
        }
    };
    let meta = ObjectMeta {
        generation: o.metadata.generation,
        created: o
            .metadata
            .creation_timestamp
            .as_ref()
            .and_then(super::to_timestamp),
        instance: annotations.get(labels::INSTANCE).cloned(),
        resume_strategy,
    };

    Ok(Pipeline {
        key: PipelineKey::new(namespace, name),
        meta,
        spec: PipelineSpec {
            isb,
            lifecycle: lifecycle_from(&o.spec.lifecycle).map_err(|e| invalid(&raw_name, e))?,
            limits: limits_from(&o.spec.limits),
            topology,
        },
        status: status_from(o.status.unwrap_or_default(), pause_started),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_invalid(value: serde_json::Value, expected_reason: &str) {
        let err = into_pipeline(serde_json::from_value(value).unwrap()).unwrap_err();
        match err {
            Error::Invalid { reason, .. } => assert!(
                reason.contains(expected_reason),
                "expected `{expected_reason}` in `{reason}`"
            ),
            other => panic!("expected invalid pipeline, got {other}"),
        }
    }

    /// Shaped like Numaflow's `1-simple-pipeline` example after the operator has
    /// defaulted and reconciled it. Synthetic values throughout.
    const SIMPLE: &str = r#"{
      "apiVersion": "numaflow.numaproj.io/v1alpha1",
      "kind": "Pipeline",
      "metadata": {
        "name": "simple-pipeline", "namespace": "demo", "generation": 2,
        "creationTimestamp": "2026-01-01T00:00:00Z",
        "annotations": {"numaflow.numaproj.io/resume-strategy": "slow"}
      },
      "spec": {
        "interStepBufferServiceName": "default",
        "lifecycle": {"desiredPhase": "Running", "pauseGracePeriodSeconds": 30, "deleteGracePeriodSeconds": 30},
        "limits": {"readBatchSize": 500, "bufferMaxLength": 30000, "bufferUsageLimit": 80},
        "vertices": [
          {"name": "in", "source": {"generator": {"rpu": 5, "duration": "1s"}}},
          {"name": "cat", "udf": {"builtin": {"name": "cat"}}, "partitions": 2},
          {"name": "agg", "udf": {"container": {"image": "example.invalid/agg:1"}, "groupBy": {"window": {}}}},
          {"name": "out", "sink": {"log": {}}}
        ],
        "edges": [
          {"from": "in", "to": "cat"},
          {"from": "cat", "to": "agg", "conditions": {"tags": {"operator": "and", "values": ["a", "b"]}}},
          {"from": "agg", "to": "out", "onFull": "discardLatest"}
        ]
      },
      "status": {
        "phase": "Running", "message": "", "observedGeneration": 2,
        "vertexCount": 4, "sourceCount": 1, "sinkCount": 1, "udfCount": 2, "mapUDFCount": 1, "reduceUDFCount": 1,
        "conditions": [{"type": "Configured", "status": "True", "reason": "Successful"}],
        "lastUpdated": "2026-01-02T00:00:00Z"
      }
    }"#;

    #[test]
    fn converts_a_reconciled_pipeline() {
        let obj: PipelineObject = serde_json::from_str(SIMPLE).unwrap();
        let p = into_pipeline(obj).unwrap();
        assert_eq!(p.key.to_string(), "demo/simple-pipeline");
        assert_eq!(p.status.phase, PipelinePhase::Running);
        assert_eq!(p.status.message, None);
        assert_eq!(p.meta.resume_strategy, Some(ResumeStrategy::Slow));
        assert!((p.spec.limits.buffer_usage_limit.get() - 0.8).abs() < 1e-9);
        assert_eq!(p.spec.lifecycle.deletion_grace, Duration::from_secs(30));
        let t = &p.spec.topology;
        assert_eq!(t.vertices().len(), 4);
        let cat = t.vertex(&VertexName::new("cat").unwrap()).unwrap();
        assert_eq!((cat.kind, cat.partitions), (VertexKind::Map, 2));
        let agg = t.vertex(&VertexName::new("agg").unwrap()).unwrap();
        assert_eq!(agg.kind, VertexKind::Reduce);
        assert_eq!(agg.image.as_deref(), Some("example.invalid/agg:1"));
        assert_eq!(
            t.edges()[1].conditions.as_ref().unwrap().operator,
            TagOperator::And
        );
        assert_eq!(t.edges()[2].on_full, OnFull::DiscardLatest);
        assert_eq!(p.status.counts.reduce_udfs, 1);
        assert!(p.status.conditions[0].ok);
        assert!(p.meta.created.is_some());
    }

    #[test]
    fn unknown_phase_is_lenient_but_bad_topology_is_not() {
        let mut v: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
        v["status"]["phase"] = "Hibernating".into();
        let p =
            into_pipeline(serde_json::from_value::<PipelineObject>(v.clone()).unwrap()).unwrap();
        assert_eq!(p.status.phase, PipelinePhase::Unknown);

        v["spec"]["edges"][0]["to"] = "ghost".into();
        let err = into_pipeline(serde_json::from_value::<PipelineObject>(v).unwrap()).unwrap_err();
        assert!(matches!(err, Error::Invalid { .. }), "{err}");
    }

    #[test]
    fn rejects_vertices_with_multiple_kind_markers() {
        let vertices = [
            serde_json::json!({"name": "in", "source": {}, "sink": {}}),
            serde_json::json!({"name": "in", "source": {}, "udf": {}}),
            serde_json::json!({"name": "in", "sink": {}, "udf": {}}),
            serde_json::json!({"name": "in", "source": {}, "sink": {}, "udf": {}}),
        ];

        for vertex in vertices {
            let mut value: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
            value["spec"]["vertices"][0] = vertex;
            assert_invalid(value, "exactly one of source, sink or udf");
        }
    }

    #[test]
    fn rejects_zero_partitions() {
        for vertex_index in [0, 1] {
            let mut value: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
            value["spec"]["vertices"][vertex_index]["partitions"] = 0.into();
            assert_invalid(value, "partitions must be at least 1");
        }
    }

    #[test]
    fn rejects_an_unknown_tag_operator() {
        let mut value: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
        value["spec"]["edges"][1]["conditions"]["tags"]["operator"] = "xor".into();

        assert_invalid(value, "unknown tag operator `xor`");
    }

    #[test]
    fn rejects_an_unknown_on_full_policy() {
        let mut value: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
        value["spec"]["edges"][0]["onFull"] = "dropOldest".into();

        assert_invalid(value, "unknown onFull policy `dropOldest`");
    }

    #[test]
    fn rejects_an_unknown_desired_phase() {
        let mut value: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
        value["spec"]["lifecycle"]["desiredPhase"] = "Hibernating".into();

        assert_invalid(value, "unknown desired phase `Hibernating`");
    }

    #[test]
    fn rejects_an_unknown_resume_strategy() {
        let mut value: serde_json::Value = serde_json::from_str(SIMPLE).unwrap();
        value["metadata"]["annotations"][labels::RESUME_STRATEGY] = "instant".into();

        assert_invalid(value, "unknown resume strategy `instant`");
    }
}
