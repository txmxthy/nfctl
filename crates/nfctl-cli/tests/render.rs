#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Snapshot tests of every renderer against the in-memory fakes.

use std::sync::Arc;

use clap::Parser;
use futures::StreamExt;
use nfctl_cli::{Cli, Context, Output, run};
use nfctl_core::fake::{FakeCluster, FakeDaemons, LogScript, sample_pipeline, sample_pod};
use nfctl_core::model::{LogLine, Namespace, PipelinePhase, PodEvent, Timestamp};
use nfctl_core::service::PipelineService;
use time::OffsetDateTime;

fn ctx() -> Context {
    ctx_with(FakeCluster::default())
}

fn ctx_with(cluster: FakeCluster) -> Context {
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
        service: PipelineService::new(cluster, Arc::new(FakeDaemons::default())),
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
