use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::{AsyncBufReadExt, StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, LogParams, Patch, PatchParams};
use kube::runtime::{WatchStreamExt, watcher};
use nfctl_core::model::ContainerName;
use nfctl_core::model::{
    DesiredPhase, IsbService, LogLine, LogOptions, Namespace, Pipeline, PipelineKey, PodEvent,
    PodName, PodRef, ResumeStrategy, Selector, Timestamp, labels,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::{Error, Result};

use crate::dto::isb::{IsbObject, into_isb};
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
        _ => Error::cluster(e),
    }
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

    async fn list_isb(&self, ns: &Namespace) -> Result<Vec<IsbService>> {
        let list = self
            .isbs(ns)
            .list(&ListParams::default())
            .await
            .map_err(|e| map_kube(e, "namespace", ns.as_str()))?;
        list.items.into_iter().map(into_isb).collect()
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
