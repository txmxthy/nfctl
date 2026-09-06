use k8s_openapi::api::core::v1::Pod;
use nfctl_core::model::{ContainerName, ContainerState, PodName, PodPhase, PodRef, labels};

/// Project a `Pod` onto the fields the tailer needs. Containers with invalid
/// names cannot exist (the API server validates them), so they are skipped rather
/// than failing the whole pod.
pub fn into_pod_ref(pod: &Pod) -> Option<PodRef> {
    let name = PodName::new(pod.metadata.name.clone()?).ok()?;
    let status = pod.status.as_ref();
    let phase = status
        .and_then(|s| s.phase.as_deref())
        .map(PodPhase::parse_lenient)
        .unwrap_or_default();
    let spec = pod.spec.as_ref();
    // Native sidecars (init containers with `restartPolicy: Always`) are how
    // Numaflow runs UDF and monitor containers; they are long-lived, so they are
    // not "init" for tailing purposes.
    let init_names: Vec<&str> = spec
        .and_then(|s| s.init_containers.as_ref())
        .map(|v| {
            v.iter()
                .filter(|c| c.restart_policy.as_deref() != Some("Always"))
                .map(|c| c.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    let all_init_names: Vec<&str> = spec
        .and_then(|s| s.init_containers.as_ref())
        .map(|v| v.iter().map(|c| c.name.as_str()).collect())
        .unwrap_or_default();
    let mut containers = Vec::new();
    let statuses = status
        .map(|s| {
            s.container_statuses
                .iter()
                .flatten()
                .chain(s.init_container_statuses.iter().flatten())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let declared = spec
        .map(|s| {
            s.containers
                .iter()
                .map(|c| c.name.as_str())
                .chain(all_init_names.iter().copied())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for cname in declared {
        let Ok(cn) = ContainerName::new(cname) else {
            continue;
        };
        let st = statuses.iter().find(|s| s.name == cname);
        containers.push(ContainerState {
            name: cn,
            running: st
                .and_then(|s| s.state.as_ref())
                .and_then(|s| s.running.as_ref())
                .is_some(),
            restart_count: st.map_or(0, |s| u32::try_from(s.restart_count).unwrap_or(0)),
            is_init: init_names.contains(&cname),
        });
    }
    let default_container = pod
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(labels::DEFAULT_CONTAINER))
        .and_then(|s| ContainerName::new(s.clone()).ok());
    Some(PodRef {
        name,
        phase,
        containers,
        default_container,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_init_containers_are_not_init() {
        let pod: Pod = serde_json::from_value(serde_json::json!({
            "metadata": {"name": "p-v-0-abc", "annotations": {"kubectl.kubernetes.io/default-container": "udf"}},
            "spec": {
                "initContainers": [
                    {"name": "init", "image": "x"},
                    {"name": "udf", "image": "x", "restartPolicy": "Always"}
                ],
                "containers": [{"name": "numa", "image": "x"}]
            },
            "status": {
                "phase": "Running",
                "containerStatuses": [{"name": "numa", "ready": true, "restartCount": 0, "image": "x", "imageID": "x", "state": {"running": {}}}],
                "initContainerStatuses": [
                    {"name": "init", "ready": true, "restartCount": 0, "image": "x", "imageID": "x", "state": {"terminated": {"exitCode": 0}}},
                    {"name": "udf", "ready": true, "restartCount": 2, "image": "x", "imageID": "x", "state": {"running": {}}}
                ]
            }
        }))
        .unwrap();
        let r = into_pod_ref(&pod).unwrap();
        let by = |n: &str| r.containers.iter().find(|c| c.name.as_str() == n).unwrap();
        assert!(by("init").is_init);
        assert!(!by("init").running);
        assert!(!by("udf").is_init, "sidecar must not count as init");
        assert!(by("udf").running);
        assert_eq!(by("udf").restart_count, 2);
        assert_eq!(
            r.default_container.as_ref().map(ContainerName::as_str),
            Some("udf")
        );
    }
}
