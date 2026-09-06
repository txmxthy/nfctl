#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Snapshot tests of every renderer against the in-memory fakes.

use std::sync::Arc;

use clap::Parser;
use nfctl_cli::{Cli, Context, run};
use nfctl_core::fake::{FakeCluster, FakeDaemons, sample_pipeline};
use nfctl_core::model::{Namespace, PipelinePhase, Timestamp};
use nfctl_core::service::PipelineService;
use time::OffsetDateTime;

fn ctx() -> Context {
    let mut paused = sample_pipeline("demo", "paused-pipeline", PipelinePhase::Paused);
    paused.status.message = Some("waiting for buffers to drain".to_owned());
    let cluster = FakeCluster::with_pipelines(vec![
        sample_pipeline("demo", "simple-pipeline", PipelinePhase::Running),
        paused,
        sample_pipeline("other", "elsewhere", PipelinePhase::Failed),
    ]);
    Context {
        service: PipelineService::new(Arc::new(cluster), Arc::new(FakeDaemons::default())),
        default_namespace: Namespace::new("demo").unwrap(),
        // 3 days, 4 hours after the epoch the fixtures were created at.
        now: Timestamp::new(OffsetDateTime::UNIX_EPOCH + time::Duration::hours(76)),
        terminal_width: Some(100),
    }
}

async fn out(args: &[&str]) -> String {
    let cli = Cli::parse_from(std::iter::once("nfctl").chain(args.iter().copied()));
    run(&cli, &ctx()).await.unwrap()
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
    let err = run(&cli, &ctx()).await.unwrap_err();
    assert!(matches!(err, nfctl_core::Error::NotFound { .. }), "{err}");
    assert_eq!(err.exit_code(), 1);
}

#[tokio::test]
async fn bad_name_is_rejected_before_any_call() {
    let cli = Cli::parse_from(["nfctl", "get", "Not_Valid"]);
    let err = run(&cli, &ctx()).await.unwrap_err();
    assert!(matches!(err, nfctl_core::Error::Usage(_)), "{err}");
    assert_eq!(err.exit_code(), 2);
}
