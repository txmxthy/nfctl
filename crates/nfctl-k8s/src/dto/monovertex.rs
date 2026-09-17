use kube::core::Object;
use nfctl_core::Error;
use nfctl_core::model::{
    Condition, DesiredPhase, MonoVertex, MonoVertexKey, MonoVertexName, MonoVertexPhase, Namespace,
};
use serde::{Deserialize, Serialize};

pub type MonoVertexObject = Object<MonoVertexSpecDto, MonoVertexStatusDto>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonoVertexSpecDto {
    pub replicas: Option<u32>,
    pub source: Option<SourceDto>,
    pub udf: Option<serde_json::Value>,
    pub sink: Option<SinkDto>,
    #[serde(default)]
    pub lifecycle: LifecycleDto,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceDto {
    pub transformer: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SinkDto {
    pub fallback: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleDto {
    pub desired_phase: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonoVertexStatusDto {
    pub phase: Option<String>,
    pub message: Option<String>,
    pub replicas: Option<u32>,
    pub desired_replicas: Option<u32>,
    pub ready_replicas: Option<u32>,
    #[serde(default)]
    pub conditions: Vec<super::pipeline::ConditionDto>,
}

pub fn into_monovertex(o: MonoVertexObject) -> Result<MonoVertex, Error> {
    let raw = o.metadata.name.clone().unwrap_or_default();
    let invalid = |reason: String| Error::Invalid {
        kind: "monovertex",
        name: raw.clone(),
        reason,
    };
    let name = MonoVertexName::new(&raw).map_err(|e| invalid(e.to_string()))?;
    let namespace = Namespace::new(o.metadata.namespace.clone().unwrap_or_default())
        .map_err(|e| invalid(format!("namespace: {e}")))?;
    let st = o.status.unwrap_or_default();
    Ok(MonoVertex {
        key: MonoVertexKey { namespace, name },
        desired: match o.spec.lifecycle.desired_phase.as_deref() {
            Some("Paused") => DesiredPhase::Paused,
            _ => DesiredPhase::Running,
        },
        phase: st
            .phase
            .as_deref()
            .map(MonoVertexPhase::parse_lenient)
            .unwrap_or_default(),
        replicas: st.replicas.unwrap_or(0),
        desired_replicas: st.desired_replicas,
        ready_replicas: st.ready_replicas,
        has_transformer: o
            .spec
            .source
            .as_ref()
            .is_some_and(|s| s.transformer.is_some()),
        has_map: o.spec.udf.is_some(),
        has_fallback: o.spec.sink.as_ref().is_some_and(|s| s.fallback.is_some()),
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
        created: o
            .metadata
            .creation_timestamp
            .as_ref()
            .and_then(super::to_timestamp),
    })
}
