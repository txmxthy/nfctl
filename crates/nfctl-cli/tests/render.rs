#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Snapshot tests of every renderer against the in-memory fakes.

use std::sync::Arc;

use clap::Parser;
use futures::StreamExt;
use nfctl_cli::{Cli, Context, Output, run};
use nfctl_core::fake::{
    FakeCluster, FakeDaemon, FakeDaemons, LogScript, sample_pipeline, sample_pod,
};
use nfctl_core::model::{
    BufferInfo, BufferName, Fraction, Health, IsbName, IsbPhase, IsbService, LogLine, Namespace,
    PipelineHealth, PipelinePhase, PodEvent, Timestamp, VertexMetrics, VertexName, Windows,
};
use nfctl_core::service::PipelineService;
use time::OffsetDateTime;

fn ctx() -> Context {
    ctx_with(FakeCluster::default())
}

fn ctx_with(cluster: FakeCluster) -> Context {
    ctx_with_daemon(cluster, FakeDaemon::default())
}

fn ctx_with_daemon(cluster: FakeCluster, daemon: FakeDaemon) -> Context {
    cluster.add_isbs([IsbService {
        name: IsbName::new("default").unwrap(),
        version: "2.10.3".into(),
        replicas: 3,
        persistent: true,
        phase: IsbPhase::Running,
        healthy: true,
    }]);
    let mut paused = sample_pipeline("demo", "paused-pipeline", PipelinePhase::Paused);
    paused.status.message = Some("waiting for buffers to drain".to_owned());
    cluster.add_pipelines([
        sample_pipeline("demo", "simple-pipeline", PipelinePhase::Running),
        paused,
        sample_pipeline("other", "elsewhere", PipelinePhase::Failed),
    ]);
    let cluster: Arc<dyn nfctl_core::ports::ClusterPort> = Arc::new(cluster);
    Context {
        cluster: Arc::clone(&cluster),
        service: PipelineService::new(cluster, Arc::new(FakeDaemons(daemon))),
        default_namespace: Namespace::new("demo").unwrap(),
        // 3 days, 4 hours after the epoch the fixtures were created at.
        now: Timestamp::new(OffsetDateTime::UNIX_EPOCH + time::Duration::hours(76)),
        terminal_width: Some(100),
    }
}

async fn out(args: &[&str]) -> String {
    let cli = Cli::parse_from(std::iter::once("nfctl").chain(args.iter().copied()));
    run(&cli, &ctx()).await.unwrap().collect().await
}

#[tokio::test]
async fn logs_snapshot_and_follow() {
    let c = FakeCluster::default();
    let pod = sample_pod("simple-pipeline-cat-0-abcd", true);
    let at = Some(Timestamp::new(
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(5),
    ));
    c.emit(&PodEvent::Applied(pod.clone()));
    for ctr in ["numa", "udf"] {
        let line = LogLine {
            at,
            text: format!("hello from {ctr}"),
        };
        c.script(
            &pod.name,
            &ctr.try_into().unwrap(),
            LogScript {
                lines: vec![line.clone()],
                hang: false,
            },
        );
        c.script(
            &pod.name,
            &ctr.try_into().unwrap(),
            LogScript {
                lines: vec![line],
                hang: true,
            },
        );
    }
    let ctx = ctx_with(c);
    let cli = Cli::parse_from(["nfctl", "logs", "simple-pipeline", "--timestamps"]);
    insta::assert_snapshot!(
        "logs_snapshot",
        run(&cli, &ctx).await.unwrap().collect().await
    );

    let cli = Cli::parse_from(["nfctl", "logs", "simple-pipeline", "-f", "-c", "udf"]);
    let Output::Lines(mut lines) = run(&cli, &ctx).await.unwrap() else {
        panic!("expected a stream")
    };
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, "simple-pipeline-cat-0-abcd/udf hello from udf");
}

#[tokio::test]
async fn ls_table() {
    insta::assert_snapshot!(out(&["ls"]).await);
}

#[tokio::test]
async fn ls_all_namespaces_wide() {
    insta::assert_snapshot!(out(&["ls", "-A", "-o", "wide"]).await);
}

#[tokio::test]
async fn ls_json() {
    insta::assert_snapshot!(out(&["ls", "-o", "json"]).await);
}

#[tokio::test]
async fn ls_empty_namespace() {
    insta::assert_snapshot!(out(&["ls", "-n", "empty"]).await);
}

#[tokio::test]
async fn get_table_and_yaml() {
    insta::assert_snapshot!("get_table", out(&["get", "simple-pipeline"]).await);
    insta::assert_snapshot!(
        "get_yaml",
        out(&["get", "simple-pipeline", "-o", "yaml"]).await
    );
}

#[tokio::test]
async fn dag_formats() {
    insta::assert_snapshot!("dag_ascii", out(&["dag", "simple-pipeline"]).await);
    insta::assert_snapshot!(
        "dag_mermaid",
        out(&["dag", "simple-pipeline", "-f", "mermaid"]).await
    );
    insta::assert_snapshot!(
        "dag_json",
        out(&["dag", "simple-pipeline", "-o", "json"]).await
    );
}

fn live_daemon() -> FakeDaemon {
    let v = |s: &str| VertexName::new(s).unwrap();
    FakeDaemon {
        buffers: vec![BufferInfo {
            name: BufferName::new("default-simple-pipeline-cat-0").unwrap(),
            from: v("in"),
            to: v("cat"),
            pending: Some(42),
            ack_pending: Some(3),
            total: Some(45),
            length: Some(30000),
            usage: Fraction::new(0.0015),
            usage_limit: Fraction::new(0.8),
            is_full: Some(false),
        }],
        metrics: vec![VertexMetrics {
            vertex: v("cat"),
            rate: Windows {
                m1: Some(120.5),
                m5: Some(118.0),
                m15: None,
                default: Some(119.0),
            },
            pending: Windows {
                m1: Some(42),
                m5: None,
                m15: None,
                default: Some(42),
            },
        }],
        watermarks: vec![],
        health: Some(PipelineHealth {
            status: Health::Healthy,
            message: "Pipeline data flow is healthy".into(),
            code: "D1".into(),
        }),
    }
}

#[tokio::test]
async fn status_with_daemon_and_without() {
    let live = ctx_with_daemon(FakeCluster::default(), live_daemon());
    let cli = Cli::parse_from(["nfctl", "status", "simple-pipeline"]);
    insta::assert_snapshot!("status_table", out_ctx(&cli, &live).await);
    let cli = Cli::parse_from(["nfctl", "top", "simple-pipeline", "--once", "-o", "json"]);
    insta::assert_snapshot!("status_json", out_ctx(&cli, &live).await);

    // Daemon reports nothing usable: the CRD half still renders, with a warning.
    let cli = Cli::parse_from(["nfctl", "status", "paused-pipeline"]);
    insta::assert_snapshot!("status_degraded", out_ctx(&cli, &ctx()).await);
}

#[tokio::test]
async fn isb_commands() {
    insta::assert_snapshot!("isb_ls", out(&["isb", "ls"]).await);
    insta::assert_snapshot!("isb_inspect", out(&["isb", "inspect", "default"]).await);
    let cli = Cli::parse_from(["nfctl", "isb", "inspect", "missing"]);
    assert!(matches!(
        run(&cli, &ctx()).await.err().unwrap(),
        nfctl_core::Error::NotFound { .. }
    ));
}

async fn out_ctx(cli: &Cli, ctx: &Context) -> String {
    run(cli, ctx).await.unwrap().collect().await
}

#[tokio::test]
async fn get_missing_is_not_found() {
    let cli = Cli::parse_from(["nfctl", "get", "ghost"]);
    let err = run(&cli, &ctx()).await.err().unwrap();
    assert!(matches!(err, nfctl_core::Error::NotFound { .. }), "{err}");
    assert_eq!(err.exit_code(), 1);
}

#[tokio::test]
async fn bad_name_is_rejected_before_any_call() {
    let cli = Cli::parse_from(["nfctl", "get", "Not_Valid"]);
    let err = run(&cli, &ctx()).await.err().unwrap();
    assert!(matches!(err, nfctl_core::Error::Usage(_)), "{err}");
    assert_eq!(err.exit_code(), 2);
}
