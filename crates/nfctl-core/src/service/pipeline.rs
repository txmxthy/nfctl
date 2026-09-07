use std::sync::Arc;

use crate::Result;
use crate::model::{Namespace, Pipeline, PipelineKey};
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

    /// CRD state fused with daemon runtime data. Never fails because of the daemon.
    pub async fn view(
        &self,
        key: &PipelineKey,
        now: crate::model::Timestamp,
    ) -> Result<super::PipelineView> {
        super::pipeline_view(self.cluster.as_ref(), self.daemons.as_ref(), key, now).await
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
