#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Public adapter behavior against a mock Kubernetes HTTP boundary.

use std::pin::pin;
use std::time::Duration;

use futures::StreamExt;
use http::{Method, Request, Response};
use kube::Client;
use kube::client::Body;
use nfctl_core::model::{
    ContainerName, DesiredPhase, LogOptions, Namespace, PipelineKey, PipelineName, PodName,
    ResumeStrategy, VertexName,
};
use nfctl_core::ports::ClusterPort;
use nfctl_k8s::KubeCluster;
use serde_json::{Value, json};

type ApiHandle = tower_test::mock::Handle<Request<Body>, Response<Body>>;

fn mock_cluster() -> (KubeCluster, ApiHandle) {
    let (service, handle) = tower_test::mock::pair();
    (KubeCluster::new(Client::new(service, "default")), handle)
}

fn json_response(value: &Value) -> Response<Body> {
    Response::new(Body::from(
        serde_json::to_vec(value).expect("JSON response"),
    ))
}

async fn json_body(request: Request<Body>) -> Value {
    serde_json::from_slice(
        &request
            .into_body()
            .collect_bytes()
            .await
            .expect("request body"),
    )
    .expect("JSON request")
}

fn key() -> PipelineKey {
    PipelineKey::new(
        Namespace::new("demo").unwrap(),
        PipelineName::new("orders").unwrap(),
    )
}

fn pipeline() -> Value {
    json!({
        "apiVersion": "numaflow.numaproj.io/v1alpha1",
        "kind": "Pipeline",
        "metadata": {"name": "orders", "namespace": "demo", "resourceVersion": "8"},
        "spec": {
            "lifecycle": {"desiredPhase": "Paused"},
            "vertices": [
                {"name": "in", "source": {}},
                {"name": "out", "sink": {}}
            ],
            "edges": [{"from": "in", "to": "out"}]
        },
        "status": {"phase": "Paused"}
    })
}

#[tokio::test]
async fn lifecycle_patch_maps_options_to_the_kubernetes_request() {
    let (cluster, handle) = mock_cluster();
    let server = tokio::spawn(async move {
        let mut handle = pin!(handle);
        let (request, send) = handle.next_request().await.expect("patch request");

        assert_eq!(request.method(), Method::PATCH);
        assert_eq!(
            request.uri().to_string(),
            "/apis/numaflow.numaproj.io/v1alpha1/namespaces/demo/pipelines/orders?&dryRun=All"
        );
        assert_eq!(
            request.headers()[http::header::CONTENT_TYPE],
            "application/merge-patch+json"
        );
        let body = json_body(request).await;
        assert_eq!(
            body,
            json!({
                "metadata": {"annotations": {"numaflow.numaproj.io/resume-strategy": "slow"}},
                "spec": {"lifecycle": {"desiredPhase": "Running"}}
            })
        );

        send.send_response(json_response(&pipeline()));
    });

    cluster
        .set_lifecycle(
            &key(),
            DesiredPhase::Running,
            Some(ResumeStrategy::Slow),
            true,
        )
        .await
        .unwrap();

    server.await.unwrap();
}

#[tokio::test]
async fn apply_preserves_the_manifest_and_maps_the_response() {
    let (cluster, handle) = mock_cluster();
    let server = tokio::spawn(async move {
        let mut handle = pin!(handle);
        let (request, send) = handle.next_request().await.expect("apply request");

        assert_eq!(request.method(), Method::PATCH);
        assert_eq!(
            request.uri().to_string(),
            "/apis/numaflow.numaproj.io/v1alpha1/namespaces/demo/pipelines/orders?&dryRun=All&force=true&fieldManager=nfctl"
        );
        assert_eq!(
            request.headers()[http::header::CONTENT_TYPE],
            "application/apply-patch+yaml"
        );
        let body = json_body(request).await;
        assert_eq!(body["metadata"]["namespace"], "demo");
        assert!(body["spec"].get("limits").is_none());

        send.send_response(json_response(&pipeline()));
    });
    let manifest = r"
apiVersion: numaflow.numaproj.io/v1alpha1
kind: Pipeline
metadata:
  name: orders
spec:
  lifecycle:
    desiredPhase: Paused
  vertices:
    - name: in
      source: {}
    - name: out
      sink: {}
  edges:
    - from: in
      to: out
";

    let applied = cluster
        .apply_manifest(manifest, &Namespace::new("demo").unwrap(), true)
        .await
        .unwrap();

    assert_eq!(applied.key, key());
    assert_eq!(applied.spec.lifecycle.desired, DesiredPhase::Paused);
    server.await.unwrap();
}

#[tokio::test]
async fn scale_uses_the_vertex_scale_subresource() {
    let (cluster, handle) = mock_cluster();
    let server = tokio::spawn(async move {
        let mut handle = pin!(handle);
        let (request, send) = handle.next_request().await.expect("scale request");

        assert_eq!(request.method(), Method::PATCH);
        assert_eq!(
            request.uri().to_string(),
            "/apis/numaflow.numaproj.io/v1alpha1/namespaces/demo/vertices/orders-worker/scale?&dryRun=All"
        );
        let body = json_body(request).await;
        assert_eq!(body, json!({"spec": {"replicas": 4}}));

        send.send_response(json_response(&json!({
                "apiVersion": "autoscaling/v1",
                "kind": "Scale",
                "metadata": {"name": "orders-worker", "namespace": "demo"},
                "spec": {"replicas": 4},
                "status": {"replicas": 2}
        })));
    });

    cluster
        .scale_vertex(&key(), &VertexName::new("worker").unwrap(), 4, true)
        .await
        .unwrap();

    server.await.unwrap();
}

#[tokio::test]
async fn delete_pod_sends_dry_run_delete_options() {
    let (cluster, handle) = mock_cluster();
    let server = tokio::spawn(async move {
        let mut handle = pin!(handle);
        let (request, send) = handle.next_request().await.expect("delete request");

        assert_eq!(request.method(), Method::DELETE);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/namespaces/demo/pods/orders-worker-0?"
        );
        let body = json_body(request).await;
        assert_eq!(body, json!({"dryRun": ["All"]}));

        send.send_response(json_response(&json!({
                "apiVersion": "v1",
                "kind": "Status",
                "status": "Success",
                "code": 200
        })));
    });

    cluster
        .delete_pod(
            &Namespace::new("demo").unwrap(),
            &PodName::new("orders-worker-0").unwrap(),
            true,
        )
        .await
        .unwrap();

    server.await.unwrap();
}

#[tokio::test]
async fn logs_map_query_options_and_kubelet_lines() {
    let (cluster, handle) = mock_cluster();
    let server = tokio::spawn(async move {
        let mut handle = pin!(handle);
        let (request, send) = handle.next_request().await.expect("logs request");

        assert_eq!(request.method(), Method::GET);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/namespaces/demo/pods/orders-worker-0/log?&container=numa&follow=true&tailLines=2&timestamps=true"
        );
        send.send_response(Response::new(Body::from(
            b"2026-01-02T03:04:05.123456789Z accepted\nplain line\n".to_vec(),
        )));
    });

    let lines = cluster
        .tail_logs(
            &Namespace::new("demo").unwrap(),
            &PodName::new("orders-worker-0").unwrap(),
            &ContainerName::new("numa").unwrap(),
            &LogOptions {
                follow: true,
                since: None,
                tail_lines: Some(2),
            },
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].as_ref().unwrap().text, "accepted");
    assert!(lines[0].as_ref().unwrap().at.is_some());
    assert_eq!(lines[1].as_ref().unwrap().text, "plain line");
    assert_eq!(lines[1].as_ref().unwrap().at, None);
    server.await.unwrap();
}

#[tokio::test]
async fn watch_pipeline_maps_the_list_watch_protocol_to_domain_updates() {
    let (cluster, handle) = mock_cluster();
    let server = tokio::spawn(async move {
        let mut handle = pin!(handle);

        let (list, send) = handle.next_request().await.expect("initial list request");
        assert_eq!(list.method(), Method::GET);
        let list_uri = list.uri().to_string();
        assert!(list_uri.contains("/namespaces/demo/pipelines?"));
        assert!(list_uri.contains("fieldSelector=metadata.name%3Dorders"));
        send.send_response(json_response(&json!({
                "apiVersion": "numaflow.numaproj.io/v1alpha1",
                "kind": "PipelineList",
                "metadata": {"resourceVersion": "7"},
                "items": []
        })));

        let (watch, send) = handle.next_request().await.expect("watch request");
        assert_eq!(watch.method(), Method::GET);
        let watch_uri = watch.uri().to_string();
        assert!(watch_uri.contains("fieldSelector=metadata.name%3Dorders"));
        assert!(watch_uri.contains("watch=true"));
        assert!(watch_uri.contains("resourceVersion=7"));
        let event = format!("{{\"type\":\"ADDED\",\"object\":{}}}\n", pipeline());
        send.send_response(Response::new(Body::from(event.into_bytes())));
    });

    let mut updates = cluster.watch_pipeline(&key()).await.unwrap();
    let update = tokio::time::timeout(Duration::from_secs(1), updates.next())
        .await
        .expect("watch update timed out")
        .expect("watch ended")
        .unwrap();

    assert_eq!(update.key, key());
    assert_eq!(update.spec.lifecycle.desired, DesiredPhase::Paused);
    server.await.unwrap();
}
