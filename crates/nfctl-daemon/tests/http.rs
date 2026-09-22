#![allow(clippy::unwrap_used)]
//! The JSON layer over plain HTTP, against a mock daemon.

use std::time::Duration;

use nfctl_core::model::{
    Edge, Namespace, OnFull, PipelineKey, PipelineName, ScaleSpec, Topology, Vertex, VertexKind,
    VertexName,
};
use nfctl_core::ports::DaemonConnector;
use nfctl_daemon::{ClientOptions, DirectConnector};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn key() -> PipelineKey {
    PipelineKey::new(
        Namespace::new("ns").unwrap(),
        PipelineName::new("p").unwrap(),
    )
}

fn connector(server: &MockServer, retries: u8) -> DirectConnector {
    let opts = ClientOptions {
        timeout: Duration::from_millis(300),
        retries,
        ..ClientOptions::default()
    };
    DirectConnector::new(&server.uri(), None, opts).unwrap()
}

#[tokio::test]
async fn happy_path_health_and_metrics() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/p/status"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"status":{"status":"warning","message":"buffer filling","code":"D2"}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/p/vertices/cat/metrics"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"vertexMetrics":[{"vertex":"cat","processingRates":{"1m":12.5},"pendings":{"1m":"4"}}]}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let d = connector(&server, 0).connect(&key(), None).await.unwrap();
    let h = d.health().await.unwrap();
    assert_eq!(h.code, "D2");
    let m = d
        .vertex_metrics(Some(&VertexName::new("cat").unwrap()))
        .await
        .unwrap();
    assert_eq!(m[0].rate.m1, Some(12.5));
    assert_eq!(m[0].pending.m1, Some(4));
}

#[tokio::test]
async fn http_errors_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/p/status"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_raw(r#"{"code":5,"message":"nope"}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let d = connector(&server, 3).connect(&key(), None).await.unwrap();
    let err = d.health().await.unwrap_err();
    assert!(err.to_string().contains("404"), "{err}");
    assert!(err.to_string().contains("nope"), "{err}");
}

#[tokio::test]
async fn timeouts_are_retried_up_to_budget() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/p/status"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(3)
        .mount(&server)
        .await;
    let d = connector(&server, 2).connect(&key(), None).await.unwrap();
    let err = d.health().await.unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
}

#[tokio::test]
async fn malformed_json_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/p/buffers"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{not json", "application/json"))
        .mount(&server)
        .await;
    let d = connector(&server, 0).connect(&key(), None).await.unwrap();
    assert!(d.buffers().await.is_err());
}

#[tokio::test]
async fn a_fan_in_buffer_keeps_every_source() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/p/buffers"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"buffers":[{"bufferName":"default-p-cat-0","pendingCount":"7"}]}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let v = |name: &str| VertexName::new(name).unwrap();
    let vertex = |name: &str, kind| Vertex {
        name: v(name),
        kind,
        partitions: 1,
        scale: ScaleSpec::default(),
        image: None,
    };
    let edge = |from: &str, to: &str| Edge {
        from: v(from),
        to: v(to),
        conditions: None,
        on_full: OnFull::default(),
    };
    let topology = Topology::new(
        vec![
            vertex("a", VertexKind::Source),
            vertex("b", VertexKind::Source),
            vertex("cat", VertexKind::Sink),
        ],
        vec![edge("b", "cat"), edge("a", "cat")],
    )
    .unwrap();
    let d = connector(&server, 0)
        .connect(&key(), Some(&topology))
        .await
        .unwrap();

    let buffers = d.buffers().await.unwrap();
    assert_eq!(buffers[0].sources, [v("a"), v("b")]);
    assert_eq!(buffers[0].to, v("cat"));
}

#[test]
fn rejects_bad_urls() {
    assert!(DirectConnector::new("ftp://x", None, ClientOptions::default()).is_err());
    assert!(DirectConnector::new("https://", None, ClientOptions::default()).is_err());
    assert!(
        DirectConnector::new(
            "https://daemon.example:4327",
            None,
            ClientOptions::default()
        )
        .is_ok()
    );
}

/// Connecting twice hands back the same client, so the connection pool it
/// holds survives a refresh instead of being rebuilt with it.
#[tokio::test]
async fn a_second_connect_reuses_the_client() {
    let server = MockServer::start().await;
    let c = connector(&server, 0);
    let a = c.connect(&key(), None).await.unwrap();
    let b = c.connect(&key(), None).await.unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b), "same client");

    // A different pipeline gets its own.
    let other = PipelineKey::new(
        Namespace::new("ns").unwrap(),
        PipelineName::new("q").unwrap(),
    );
    let d = c.connect(&other, None).await.unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a, &d), "one client per pipeline");
}
