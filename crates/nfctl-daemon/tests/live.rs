#![allow(clippy::unwrap_used)]
//! Against a real cluster. Needs `NFCTL_TEST_CONTEXT` and a pipeline named by
//! `NFCTL_TEST_PIPELINE` (default `simple-pipeline`) in the default namespace.

use std::sync::Arc;

use nfctl_core::model::{Namespace, PipelineKey, PipelineName};
use nfctl_core::ports::{ClusterPort, DaemonConnector};
use nfctl_daemon::{ClientOptions, PortForwardConnector};

#[tokio::test]
#[ignore = "needs a cluster; run with NFCTL_TEST_CONTEXT=<ctx>"]
async fn port_forward_round_trip() {
    let ctx = std::env::var("NFCTL_TEST_CONTEXT").expect("NFCTL_TEST_CONTEXT");
    let pipeline =
        std::env::var("NFCTL_TEST_PIPELINE").unwrap_or_else(|_| "simple-pipeline".to_owned());
    let conn = nfctl_k8s::connect(&nfctl_k8s::ClientOptions {
        context: Some(ctx),
        request_timeout: None,
    })
    .await
    .unwrap();
    let cluster: Arc<dyn ClusterPort> = Arc::new(nfctl_k8s::KubeCluster::new(conn.client.clone()));
    let daemons = PortForwardConnector::new(conn.client, cluster, ClientOptions::default());
    let key = PipelineKey::new(
        Namespace::default_ns(),
        PipelineName::new(pipeline).unwrap(),
    );
    let d = daemons.connect(&key, None).await.unwrap();

    let h = d.health().await.unwrap();
    assert!(!h.code.is_empty());
    let buffers = d.buffers().await.unwrap();
    assert!(!buffers.is_empty());
    assert!(
        buffers
            .iter()
            .all(|b| b.sources.iter().all(|source| source.as_str() != "unknown")),
        "{buffers:?}"
    );
    let metrics = d.vertex_metrics(None).await.unwrap();
    assert!(!metrics.is_empty());
    let wm = d.watermarks().await.unwrap();
    assert!(!wm.is_empty());
    // Second call reuses the pooled connection.
    d.health().await.unwrap();
}
