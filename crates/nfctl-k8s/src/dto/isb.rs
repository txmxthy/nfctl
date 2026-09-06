use kube::core::Object;
use nfctl_core::Error;
use nfctl_core::model::{IsbName, IsbPhase, IsbService};
use serde::{Deserialize, Serialize};

pub type IsbObject = Object<IsbSpecDto, IsbStatusDto>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IsbSpecDto {
    pub jetstream: Option<JetStreamDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JetStreamDto {
    pub version: Option<String>,
    pub replicas: Option<u32>,
    pub persistence: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsbStatusDto {
    pub phase: Option<String>,
    #[serde(default)]
    pub conditions: Vec<super::pipeline::ConditionDto>,
}

pub fn into_isb(o: IsbObject) -> Result<IsbService, Error> {
    let raw = o.metadata.name.clone().unwrap_or_default();
    let name = IsbName::new(&raw).map_err(|e| Error::Invalid {
        kind: "isbsvc",
        name: raw.clone(),
        reason: e.to_string(),
    })?;
    let js = o.spec.jetstream.unwrap_or_default();
    let st = o.status.unwrap_or_default();
    let phase = st
        .phase
        .as_deref()
        .map(IsbPhase::parse_lenient)
        .unwrap_or_default();
    // The ISB controller reports Configured/Deployed/ChildrenResourcesHealthy; no `Ready`.
    let ready = !st.conditions.is_empty() && st.conditions.iter().all(|c| c.status == "True");
    Ok(IsbService {
        name,
        version: js.version.unwrap_or_default(),
        // The operator coerces 2 to 3; mirror that so the table matches reality.
        replicas: match js.replicas.unwrap_or(3) {
            r if r < 3 => 3,
            r => r,
        },
        persistent: js.persistence.is_some(),
        phase,
        healthy: ready && matches!(phase, IsbPhase::Running | IsbPhase::Deleting),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn healthy_when_running_and_all_conditions_true() {
        let obj: IsbObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "numaflow.numaproj.io/v1alpha1", "kind": "InterStepBufferService",
            "metadata": {"name": "default", "namespace": "demo"},
            "spec": {"jetstream": {"version": "2.10.3", "replicas": 3, "persistence": {"volumeSize": "3Gi"}}},
            "status": {
                "phase": "Running", "type": "jetstream", "observedGeneration": 1,
                "conditions": [
                    {"type": "ChildrenResourcesHealthy", "status": "True", "reason": "Healthy"},
                    {"type": "Configured", "status": "True"},
                    {"type": "Deployed", "status": "True"}
                ]
            }
        }))
        .unwrap();
        let isb = into_isb(obj).unwrap();
        assert_eq!(isb.phase, IsbPhase::Running);
        assert!(isb.persistent);
        assert!(isb.healthy);
    }
}
