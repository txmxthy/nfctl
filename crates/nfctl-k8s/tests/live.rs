#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Real-cluster smoke test. Opt in with `NFCTL_TEST_CONTEXT`; the context's
//! default namespace must contain `NFCTL_TEST_PIPELINE` (`simple-pipeline`).

use nfctl_core::model::{PipelineKey, PipelineName};
use nfctl_core::ports::ClusterPort;
use nfctl_k8s::{ClientOptions, KubeCluster, connect};

#[tokio::test]
#[ignore = "needs Numaflow; run with NFCTL_TEST_CONTEXT=<ctx> [NFCTL_TEST_PIPELINE=<name>]"]
async fn gets_a_pipeline_from_a_real_cluster() {
    let context = std::env::var("NFCTL_TEST_CONTEXT").expect("set NFCTL_TEST_CONTEXT");
    let name =
        std::env::var("NFCTL_TEST_PIPELINE").unwrap_or_else(|_| "simple-pipeline".to_owned());
    let connected = connect(&ClientOptions {
        context: Some(context),
        request_timeout: None,
    })
    .await
    .unwrap();
    let key = PipelineKey::new(
        connected.default_namespace.clone(),
        PipelineName::new(name).unwrap(),
    );
    let cluster = KubeCluster::new(connected.client);

    let pipeline = cluster.get_pipeline(&key).await.unwrap();

    assert_eq!(pipeline.key, key);
    assert!(!pipeline.spec.topology.vertices().is_empty());
}
