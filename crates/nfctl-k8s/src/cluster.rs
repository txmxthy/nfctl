use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::{AsyncBufReadExt, StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::{Api, DeleteParams, DynamicObject, ListParams, LogParams, Patch, PatchParams};
use kube::runtime::{WatchStreamExt, watcher};
use nfctl_core::model::ContainerName;
use nfctl_core::model::{
    DesiredPhase, IsbService, LogLine, LogOptions, MonoVertex, MonoVertexKey, Namespace, Pipeline,
    PipelineKey, PodEvent, PodName, PodRef, ResumeStrategy, Selector, Timestamp, VertexName,
    labels,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::{Error, Result};

use crate::dto::isb::{IsbObject, into_isb};
use crate::dto::monovertex::{MonoVertexObject, into_monovertex};
use crate::dto::pipeline::{PipelineObject, into_pipeline};
use crate::dto::pod::into_pod_ref;

/// [`ClusterPort`] backed by kube-rs.
#[derive(Clone)]
pub struct KubeCluster {
    client: Client,
}

impl std::fmt::Debug for KubeCluster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KubeCluster")
    }
}

impl KubeCluster {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn pipelines(&self, ns: &Namespace) -> Api<PipelineObject> {
        Api::namespaced_with(
            self.client.clone(),
            ns.as_str(),
            &crate::dto::pipeline_resource(),
        )
    }

    fn isbs(&self, ns: &Namespace) -> Api<IsbObject> {
        Api::namespaced_with(
            self.client.clone(),
            ns.as_str(),
            &crate::dto::isb_resource(),
        )
    }

    fn monovertices(&self, ns: &Namespace) -> Api<MonoVertexObject> {
        Api::namespaced_with(
            self.client.clone(),
            ns.as_str(),
            &crate::dto::monovertex_resource(),
        )
    }

    fn pods(&self, ns: &Namespace) -> Api<Pod> {
        Api::namespaced(self.client.clone(), ns.as_str())
    }
}

/// Map kube's error onto the domain's, keeping the original as `source`.
fn map_kube(e: kube::Error, kind: &'static str, name: &str) -> Error {
    match &e {
        kube::Error::Api(ae) if ae.code == 404 => Error::NotFound {
            kind,
            name: name.to_owned(),
        },
        kube::Error::Api(ae) if ae.code == 403 => Error::Forbidden(ae.message.clone()),
        // The API server's message is the useful part; the struct dump is not.
        kube::Error::Api(ae) => Error::Cluster(format!("{} (HTTP {})", ae.message, ae.code).into()),
        _ => Error::cluster(e),
    }
}

/// YAML or JSON text → a JSON document, checking it really is a Pipeline.
fn parse_document(text: &str) -> Result<serde_json::Value> {
    let invalid = |reason: String| Error::Invalid {
        kind: "manifest",
        name: "<input>".into(),
        reason,
    };
    let value: serde_json::Value =
        serde_yaml_ng::from_str(text).map_err(|e| invalid(e.to_string()))?;
    let kind = value
        .get("kind")
        .and_then(|k| k.as_str())
        .unwrap_or_default();
    if kind != "Pipeline" {
        return Err(invalid(format!("kind is `{kind}`, expected `Pipeline`")));
    }
    Ok(value)
}

/// YAML or JSON text → the wire object.
fn parse_pipeline_object(text: &str) -> Result<PipelineObject> {
    let invalid = |reason: String| Error::Invalid {
        kind: "manifest",
        name: "<input>".into(),
        reason,
    };
    serde_json::from_value(parse_document(text)?).map_err(|e| invalid(e.to_string()))
}

/// Split the `RFC3339Nano ` prefix kubelet adds with `timestamps=true`.
fn parse_log_line(raw: &str) -> LogLine {
    match raw.split_once(' ') {
        Some((ts, rest)) => match Timestamp::parse_rfc3339(ts) {
            Ok(at) => LogLine {
                at: Some(at),
                text: rest.to_owned(),
            },
            Err(_) => LogLine {
                at: None,
                text: raw.to_owned(),
            },
        },
        None => LogLine {
            at: None,
            text: raw.to_owned(),
        },
    }
}

#[async_trait]
impl ClusterPort for KubeCluster {
    async fn list_pipelines(&self, ns: Option<&Namespace>) -> Result<Vec<Pipeline>> {
        let api = match ns {
            Some(ns) => self.pipelines(ns),
            None => Api::all_with(self.client.clone(), &crate::dto::pipeline_resource()),
        };
        let list = api
            .list(&ListParams::default())
            .await
            .map_err(|e| map_kube(e, "namespace", ns.map_or("*", Namespace::as_str)))?;
        list.items.into_iter().map(into_pipeline).collect()
    }

    async fn get_pipeline(&self, key: &PipelineKey) -> Result<Pipeline> {
        let obj = self
            .pipelines(&key.namespace)
            .get(key.name.as_str())
            .await
            .map_err(|e| map_kube(e, "pipeline", &key.to_string()))?;
        into_pipeline(obj)
    }

    async fn watch_pipeline(
        &self,
        key: &PipelineKey,
    ) -> Result<BoxStream<'static, Result<Pipeline>>> {
        let cfg = watcher::Config::default().fields(&format!("metadata.name={}", key.name));
        let name = key.to_string();
        let stream = watcher(self.pipelines(&key.namespace), cfg)
            .default_backoff()
            .map_err(Error::cluster)
            .try_filter_map(move |ev| {
                let name = name.clone();
                async move {
                    match ev {
                        watcher::Event::Apply(o) | watcher::Event::InitApply(o) => {
                            into_pipeline(o).map(Some)
                        }
                        watcher::Event::Delete(_) => Err(Error::NotFound {
                            kind: "pipeline",
                            name,
                        }),
                        watcher::Event::Init | watcher::Event::InitDone => Ok(None),
                    }
                }
            });
        Ok(Box::pin(stream))
    }

    async fn set_lifecycle(
        &self,
        key: &PipelineKey,
        desired: DesiredPhase,
        resume: Option<ResumeStrategy>,
        dry_run: bool,
    ) -> Result<()> {
        let mut body =
            serde_json::json!({ "spec": { "lifecycle": { "desiredPhase": desired.as_str() } } });
        if let Some(r) = resume {
            body["metadata"] =
                serde_json::json!({ "annotations": { labels::RESUME_STRATEGY: r.as_str() } });
        }
        let pp = PatchParams {
            dry_run,
            ..PatchParams::default()
        };
        self.pipelines(&key.namespace)
            .patch(key.name.as_str(), &pp, &Patch::Merge(&body))
            .await
            .map_err(|e| map_kube(e, "pipeline", &key.to_string()))?;
        Ok(())
    }

    fn parse_manifest(&self, text: &str, default_ns: &Namespace) -> Result<Pipeline> {
        let mut obj = parse_pipeline_object(text)?;
        if obj.metadata.namespace.is_none() {
            obj.metadata.namespace = Some(default_ns.to_string());
        }
        into_pipeline(obj)
    }

    async fn apply_manifest(
        &self,
        text: &str,
        default_ns: &Namespace,
        dry_run: bool,
    ) -> Result<Pipeline> {
        // Send the user's document, not a round-trip through our DTOs: absent
        // fields must stay absent for server-side apply.
        let mut doc = parse_document(text)?;
        let invalid = |reason: String| Error::Invalid {
            kind: "manifest",
            name: "<input>".into(),
            reason,
        };
        let Some(meta) = doc
            .get_mut("metadata")
            .and_then(serde_json::Value::as_object_mut)
        else {
            return Err(invalid("no metadata".into()));
        };
        if !meta.contains_key("namespace") {
            meta.insert(
                "namespace".into(),
                serde_json::Value::String(default_ns.to_string()),
            );
        }
        let name = meta
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let ns_raw = meta
            .get("namespace")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let ns = Namespace::new(ns_raw).map_err(|e| invalid(format!("namespace: {e}")))?;
        // Validate locally first so a bad manifest fails with a domain error.
        let obj: PipelineObject =
            serde_json::from_value(doc.clone()).map_err(|e| invalid(e.to_string()))?;
        into_pipeline(obj)?;
        let pp = PatchParams {
            dry_run,
            force: true,
            ..PatchParams::apply("nfctl")
        };
        let applied = self
            .pipelines(&ns)
            .patch(&name, &pp, &Patch::Apply(&doc))
            .await
            .map_err(|e| map_kube(e, "pipeline", &name))?;
        into_pipeline(applied)
    }

    async fn scale_vertex(
        &self,
        key: &PipelineKey,
        vertex: &VertexName,
        replicas: u32,
        dry_run: bool,
    ) -> Result<()> {
        // Vertex objects are named `<pipeline>-<vertex>`.
        let name = format!("{}-{}", key.name, vertex);
        let api: Api<DynamicObject> = Api::namespaced_with(
            self.client.clone(),
            key.namespace.as_str(),
            &crate::dto::vertex_resource(),
        );
        let pp = PatchParams {
            dry_run,
            ..PatchParams::default()
        };
        api.patch_scale(
            &name,
            &pp,
            &Patch::Merge(serde_json::json!({ "spec": { "replicas": replicas } })),
        )
        .await
        .map_err(|e| map_kube(e, "vertex", &format!("{key}/{vertex}")))?;
        Ok(())
    }

    async fn list_isb(&self, ns: Option<&Namespace>) -> Result<Vec<IsbService>> {
        let api = match ns {
            Some(ns) => self.isbs(ns),
            None => Api::all_with(self.client.clone(), &crate::dto::isb_resource()),
        };
        let list = api
            .list(&ListParams::default())
            .await
            .map_err(|e| map_kube(e, "namespace", ns.map_or("*", Namespace::as_str)))?;
        list.items.into_iter().map(into_isb).collect()
    }

    async fn list_monovertices(&self, ns: Option<&Namespace>) -> Result<Vec<MonoVertex>> {
        let api = match ns {
            Some(ns) => self.monovertices(ns),
            None => Api::all_with(self.client.clone(), &crate::dto::monovertex_resource()),
        };
        let list = api
            .list(&ListParams::default())
            .await
            .map_err(|e| map_kube(e, "namespace", ns.map_or("*", Namespace::as_str)))?;
        list.items.into_iter().map(into_monovertex).collect()
    }

    async fn get_monovertex(&self, key: &MonoVertexKey) -> Result<MonoVertex> {
        let obj = self
            .monovertices(&key.namespace)
            .get(key.name.as_str())
            .await
            .map_err(|e| map_kube(e, "monovertex", &key.to_string()))?;
        into_monovertex(obj)
    }

    async fn set_monovertex_lifecycle(
        &self,
        key: &MonoVertexKey,
        desired: DesiredPhase,
        dry_run: bool,
    ) -> Result<()> {
        let body = match desired {
            DesiredPhase::Paused => {
                serde_json::json!({ "spec": { "lifecycle": { "desiredPhase": "Paused" } } })
            }
            // Clearing replicas hands control back to the autoscaler.
            DesiredPhase::Running => {
                serde_json::json!({ "spec": { "lifecycle": { "desiredPhase": "Running" }, "replicas": null } })
            }
        };
        let pp = PatchParams {
            dry_run,
            ..PatchParams::default()
        };
        self.monovertices(&key.namespace)
            .patch(key.name.as_str(), &pp, &Patch::Merge(&body))
            .await
            .map_err(|e| map_kube(e, "monovertex", &key.to_string()))?;
        Ok(())
    }

    async fn list_pods(&self, ns: &Namespace, selector: &Selector) -> Result<Vec<PodRef>> {
        let list = self
            .pods(ns)
            .list(&ListParams::default().labels(&selector.to_string()))
            .await
            .map_err(|e| map_kube(e, "namespace", ns.as_str()))?;
        Ok(list.items.iter().filter_map(into_pod_ref).collect())
    }

    async fn watch_pods(
        &self,
        ns: &Namespace,
        selector: &Selector,
    ) -> Result<BoxStream<'static, Result<PodEvent>>> {
        let cfg = watcher::Config::default().labels(&selector.to_string());
        let stream = watcher(self.pods(ns), cfg)
            .default_backoff()
            .map_err(Error::cluster)
            .try_filter_map(|ev| async move {
                Ok(match ev {
                    watcher::Event::Apply(p) | watcher::Event::InitApply(p) => {
                        into_pod_ref(&p).map(PodEvent::Applied)
                    }
                    watcher::Event::Delete(p) => into_pod_ref(&p).map(PodEvent::Deleted),
                    watcher::Event::Init => Some(PodEvent::Resync),
                    watcher::Event::InitDone => Some(PodEvent::ResyncDone),
                })
            });
        Ok(Box::pin(stream))
    }

    async fn delete_pods(
        &self,
        ns: &Namespace,
        selector: &Selector,
        dry_run: bool,
    ) -> Result<Vec<PodName>> {
        let api = self.pods(ns);
        let pods = self.list_pods(ns, selector).await?;
        let dp = DeleteParams {
            dry_run,
            ..DeleteParams::default()
        };
        let mut deleted = Vec::with_capacity(pods.len());
        for p in pods {
            api.delete(p.name.as_str(), &dp)
                .await
                .map_err(|e| map_kube(e, "pod", p.name.as_str()))?;
            deleted.push(p.name);
        }
        Ok(deleted)
    }

    async fn tail_logs(
        &self,
        ns: &Namespace,
        pod: &PodName,
        container: &ContainerName,
        opts: &LogOptions,
    ) -> Result<BoxStream<'static, Result<LogLine>>> {
        let lp = LogParams {
            container: Some(container.to_string()),
            follow: opts.follow,
            timestamps: true,
            since_time: opts.since.and_then(|t| {
                k8s_openapi::jiff::Timestamp::from_nanosecond(t.get().unix_timestamp_nanos()).ok()
            }),
            tail_lines: opts.tail_lines.map(i64::from),
            ..LogParams::default()
        };
        let reader = self
            .pods(ns)
            .log_stream(pod.as_str(), &lp)
            .await
            .map_err(|e| map_kube(e, "pod", pod.as_str()))?;
        let lines = reader
            .lines()
            .map(|r| r.map(|l| parse_log_line(&l)).map_err(Error::cluster));
        Ok(Box::pin(lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_kubelet_timestamp_prefix() {
        let l = parse_log_line("2026-01-02T03:04:05.123456789Z hello world");
        assert_eq!(l.text, "hello world");
        assert!(l.at.is_some());
        let l = parse_log_line("no timestamp here");
        assert_eq!(l.at, None);
        assert_eq!(l.text, "no timestamp here");
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn parses_yaml_and_rejects_other_kinds() {
        let yaml = "apiVersion: numaflow.numaproj.io/v1alpha1\nkind: Pipeline\nmetadata:\n  name: p\nspec:\n  vertices:\n  - name: in\n    source: {generator: {}}\n  - name: out\n    sink: {log: {}}\n  edges:\n  - {from: in, to: out}\n";
        let obj = parse_pipeline_object(yaml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(obj.metadata.name.as_deref(), Some("p"));
        let bad = parse_pipeline_object("kind: Vertex\nmetadata: {name: x}\n");
        assert!(bad.is_err());
    }
}
