use std::sync::Arc;

use crate::Result;
use crate::model::{
    MonoVertex, MonoVertexKey, Namespace, Pipeline, PipelineKey, Timestamp, Workload, WorkloadKey,
    WorkloadKind,
};
use crate::ports::{ClusterPort, DaemonConnector};

/// Pipeline-level operations composed from the two ports.
#[derive(Clone)]
pub struct PipelineService {
    cluster: Arc<dyn ClusterPort>,
    daemons: Arc<dyn DaemonConnector>,
}

impl std::fmt::Debug for PipelineService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PipelineService")
    }
}

impl PipelineService {
    #[must_use]
    pub fn new(cluster: Arc<dyn ClusterPort>, daemons: Arc<dyn DaemonConnector>) -> Self {
        Self { cluster, daemons }
    }

    /// Pipelines in one namespace (or all), sorted by namespace then name.
    pub async fn list(&self, ns: Option<&Namespace>) -> Result<Vec<Pipeline>> {
        let mut v = self.cluster.list_pipelines(ns).await?;
        v.sort_by(|a, b| (&a.key.namespace, &a.key.name).cmp(&(&b.key.namespace, &b.key.name)));
        Ok(v)
    }

    pub async fn get(&self, key: &PipelineKey) -> Result<Pipeline> {
        self.cluster.get_pipeline(key).await
    }

    /// Pipelines and `MonoVertices` together, sorted by namespace then name.
    /// Both lists are asked for at once: they are independent API calls and
    /// one after the other doubles what a distant cluster costs.
    pub async fn list_workloads(&self, ns: Option<&Namespace>) -> Result<Vec<Workload>> {
        let (pipelines, monovertices) = futures::try_join!(
            self.cluster.list_pipelines(ns),
            self.cluster.list_monovertices(ns),
        )?;
        let mut v: Vec<Workload> = pipelines
            .into_iter()
            .map(Workload::from)
            .chain(monovertices.into_iter().map(Workload::from))
            .collect();
        v.sort_by(|a, b| {
            (a.namespace(), a.name(), a.kind()).cmp(&(b.namespace(), b.name(), b.kind()))
        });
        Ok(v)
    }

    /// One workload of either kind.
    pub async fn get_workload(&self, key: &WorkloadKey) -> Result<Workload> {
        match key.kind {
            WorkloadKind::Pipeline => {
                let k = PipelineKey::new(key.namespace.clone(), key.name.clone());
                self.cluster.get_pipeline(&k).await.map(Workload::from)
            }
            WorkloadKind::MonoVertex => {
                let k = MonoVertexKey {
                    namespace: key.namespace.clone(),
                    name: key.name.clone(),
                };
                self.cluster.get_monovertex(&k).await.map(Workload::from)
            }
        }
    }

    pub async fn get_monovertex(&self, key: &MonoVertexKey) -> Result<MonoVertex> {
        self.cluster.get_monovertex(key).await
    }

    /// A `MonoVertex` fused with what its own daemon knows. Never fails
    /// because of the daemon.
    pub async fn monovertex_view(
        &self,
        key: &MonoVertexKey,
        now: Timestamp,
    ) -> Result<super::MonoVertexView> {
        super::monovertex_view(self.cluster.as_ref(), self.daemons.as_ref(), key, now).await
    }

    /// CRD state fused with daemon runtime data. Never fails because of the daemon.
    pub async fn view(
        &self,
        key: &PipelineKey,
        now: crate::model::Timestamp,
    ) -> Result<super::PipelineView> {
        super::pipeline_view(self.cluster.as_ref(), self.daemons.as_ref(), key, now).await
    }

    /// The shape alone, from one call to the cluster: something to draw while
    /// the daemon is still being asked.
    pub async fn shape(
        &self,
        key: &PipelineKey,
        now: crate::model::Timestamp,
    ) -> Result<super::PipelineView> {
        super::pipeline_shape(self.cluster.as_ref(), key, now).await
    }

    /// The numbers, on a shape already read.
    pub async fn numbers(
        &self,
        shape: super::PipelineView,
        now: crate::model::Timestamp,
    ) -> super::PipelineView {
        super::pipeline_numbers(self.daemons.as_ref(), shape, now).await
    }

    /// The daemon factory, for procedures that need runtime data.
    #[must_use]
    pub fn daemons(&self) -> &dyn DaemonConnector {
        self.daemons.as_ref()
    }

    pub async fn list_isb(&self, ns: Option<&Namespace>) -> Result<Vec<crate::model::IsbService>> {
        let mut v = self.cluster.list_isb(ns).await?;
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }
}
