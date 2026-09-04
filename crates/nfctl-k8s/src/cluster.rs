use async_trait::async_trait;
use futures::stream::BoxStream;
use kube::Client;
use kube::api::{Api, ListParams};
use nfctl_core::model::{
    ContainerName, DesiredPhase, IsbService, LogLine, Namespace, Pipeline, PipelineKey, PodEvent,
    PodName, PodRef, ResumeStrategy, Selector, Timestamp,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::{Error, Result};

use crate::dto::pipeline::{PipelineObject, into_pipeline};

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
        _key: &PipelineKey,
    ) -> Result<BoxStream<'static, Result<Pipeline>>> {
        Err(Error::Unimplemented("watch pipeline"))
    }

    async fn set_lifecycle(
        &self,
        _key: &PipelineKey,
        _desired: DesiredPhase,
        _resume: Option<ResumeStrategy>,
        _dry_run: bool,
    ) -> Result<()> {
        Err(Error::Unimplemented("set lifecycle"))
    }

    async fn list_isb(&self, _ns: &Namespace) -> Result<Vec<IsbService>> {
        Err(Error::Unimplemented("list isb"))
    }

    async fn list_pods(&self, _ns: &Namespace, _selector: &Selector) -> Result<Vec<PodRef>> {
        Err(Error::Unimplemented("list pods"))
    }

    async fn watch_pods(
        &self,
        _ns: &Namespace,
        _selector: &Selector,
    ) -> Result<BoxStream<'static, Result<PodEvent>>> {
        Err(Error::Unimplemented("watch pods"))
    }

    async fn delete_pods(
        &self,
        _ns: &Namespace,
        _selector: &Selector,
        _dry_run: bool,
    ) -> Result<Vec<PodName>> {
        Err(Error::Unimplemented("delete pods"))
    }

    async fn tail_logs(
        &self,
        _ns: &Namespace,
        _pod: &PodName,
        _container: &ContainerName,
        _since: Option<Timestamp>,
        _tail_lines: Option<u32>,
    ) -> Result<BoxStream<'static, Result<LogLine>>> {
        Err(Error::Unimplemented("tail logs"))
    }
}
