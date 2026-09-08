#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Snapshot tests of every renderer against the in-memory fakes.

use std::sync::Arc;

use clap::Parser;
use futures::StreamExt;
use nfctl_cli::{Cli, Context, Output, run};
use nfctl_core::fake::{
    FakeCluster, FakeDaemon, FakeDaemons, LogScript, sample_monovertex, sample_pipeline, sample_pod,
};
use nfctl_core::model::{
    BufferInfo, BufferName, Fraction, Health, IsbName, IsbPhase, IsbService, LogLine,
    MonoVertexPhase, Namespace, PipelineHealth, PipelinePhase, PodEvent, Timestamp, VertexMetrics,
    VertexName, Windows,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::service::PipelineService;
use time::OffsetDateTime;

fn ctx() -> Context {
    ctx_with(FakeCluster::default())
}

fn ctx_with(cluster: FakeCluster) -> Context {
    ctx_with_daemon(cluster, FakeDaemon::default())
}

fn ctx_with_daemon(cluster: FakeCluster, daemon: FakeDaemon) -> Context {
    cluster.add_monovertices([sample_monovertex("demo", "mono", MonoVertexPhase::Running)]);
    cluster.add_isbs([IsbService {
        namespace: Namespace::new("demo").unwrap(),
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
        service: PipelineService::new(cluster, Arc::new(FakeDaemons::one(daemon))),
        default_namespace: Namespace::new("demo").unwrap(),
        // 3 days, 4 hours after the epoch the fixtures were created at.
        now: Timestamp::new(OffsetDateTime::UNIX_EPOCH + time::Duration::hours(76)),
        terminal_width: Some(100),
        colour: false,
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

#[tokio::test]
async fn lifecycle_commands_against_fake() {
    let c = FakeCluster::default();
    c.emit(&PodEvent::Applied(sample_pod(
        "simple-pipeline-cat-0-abcd",
        false,
    )));
    let ctx = ctx_with(c.clone());
    let run_s = |args: &'static [&'static str]| {
        let ctx = ctx.clone();
        async move { out_ctx(&Cli::parse_from(args), &ctx).await }
    };
    assert_eq!(
        run_s(&["nfctl", "pause", "simple-pipeline", "--dry-run"]).await,
        "demo/simple-pipeline: pause accepted (dry run)\n"
    );
    assert_eq!(
        run_s(&["nfctl", "pause", "simple-pipeline", "--wait"]).await,
        "demo/simple-pipeline: Paused; drain state unknown\n"
    );
    assert_eq!(
        run_s(&["nfctl", "wait", "simple-pipeline", "--phase", "paused"]).await,
        "demo/simple-pipeline: Paused\n"
    );
    assert_eq!(
        run_s(&["nfctl", "resume", "simple-pipeline", "--strategy", "slow"]).await,
        "demo/simple-pipeline: resume (slow)\n"
    );
    insta::assert_snapshot!(
        "recycle_vertex",
        run_s(&["nfctl", "recycle", "simple-pipeline", "cat"]).await
    );
    assert_eq!(
        run_s(&["nfctl", "recycle", "simple-pipeline"]).await,
        "demo/simple-pipeline: paused, resumed (fast)\n"
    );
    assert_eq!(
        run_s(&["nfctl", "scale", "simple-pipeline", "cat", "3"]).await,
        "demo/simple-pipeline/cat: replicas=3\n"
    );
    assert_eq!(c.scale_calls().len(), 1);
    let err = run(
        &Cli::parse_from([
            "nfctl",
            "wait",
            "simple-pipeline",
            "--phase",
            "failed",
            "--timeout",
            "0",
        ]),
        &ctx,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, nfctl_core::Error::Timeout(_)), "{err}");
}

#[tokio::test]
async fn apply_check_blocks_and_warns() {
    let c = FakeCluster::default();
    let ctx = ctx_with(c.clone());
    let dir = std::env::temp_dir().join(format!("nfctl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: &str| {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p.to_string_lossy().into_owned()
    };

    // Same topology, different ISB: blocked.
    let mut isb_change = sample_pipeline("demo", "simple-pipeline", PipelinePhase::Running);
    isb_change.spec.isb = IsbName::new("other").unwrap();
    let f1 = write("isb.yaml", "isb-change");
    c.register_manifest("isb-change", isb_change);
    let err = run(
        &Cli::parse_from(["nfctl", "apply", "-f", &f1, "--check"]),
        &ctx,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.exit_code(), 3);
    insta::assert_snapshot!("apply_check_block", err.to_string());

    // New pipeline: fine, and applied.
    let f2 = write("new.yaml", "brand-new");
    c.register_manifest(
        "brand-new",
        sample_pipeline("demo", "brand-new", PipelinePhase::Unknown),
    );
    insta::assert_snapshot!(
        "apply_create",
        out_ctx(&Cli::parse_from(["nfctl", "apply", "-f", &f2]), &ctx).await
    );
    assert!(
        c.list_pipelines(None)
            .await
            .unwrap()
            .iter()
            .any(|p| p.key.name.as_str() == "brand-new")
    );

    // Unchanged existing pipeline with --check: no changes reported.
    let f3 = write("same.yaml", "same");
    c.register_manifest(
        "same",
        sample_pipeline("demo", "simple-pipeline", PipelinePhase::Running),
    );
    assert_eq!(
        out_ctx(
            &Cli::parse_from(["nfctl", "apply", "-f", &f3, "--check"]),
            &ctx
        )
        .await,
        "demo/simple-pipeline: no topology changes\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn mvtx_commands() {
    insta::assert_snapshot!("mvtx_ls", out(&["mvtx", "ls", "-o", "wide"]).await);
    insta::assert_snapshot!("mvtx_status", out(&["mvtx", "status", "mono"]).await);
    assert_eq!(out(&["mvtx", "pause", "mono"]).await, "demo/mono: pause\n");
    let ctx = ctx();
    assert_eq!(
        out_ctx(&Cli::parse_from(["nfctl", "mvtx", "pause", "mono"]), &ctx).await,
        "demo/mono: pause\n"
    );
    assert!(
        out_ctx(&Cli::parse_from(["nfctl", "mvtx", "get", "mono"]), &ctx)
            .await
            .contains("Paused")
    );
    assert_eq!(
        out_ctx(&Cli::parse_from(["nfctl", "mvtx", "resume", "mono"]), &ctx).await,
        "demo/mono: resume\n"
    );
}

#[tokio::test]
async fn bare_names_resolve_across_namespaces_unless_ambiguous() {
    let c = FakeCluster::default();
    c.add_pipelines([
        sample_pipeline("team-a", "shared-name", PipelinePhase::Running),
        sample_pipeline("team-b", "shared-name", PipelinePhase::Paused),
    ]);
    let ctx = ctx_with(c);
    // `elsewhere` lives only in `other`, so no -n is needed.
    assert!(
        out_ctx(&Cli::parse_from(["nfctl", "get", "elsewhere"]), &ctx)
            .await
            .contains("Failed")
    );
    let err = run(&Cli::parse_from(["nfctl", "get", "shared-name"]), &ctx)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, nfctl_core::Error::Usage(_)), "{err}");
    assert!(
        err.to_string().contains("team-a/shared-name") && err.to_string().contains("pass -n"),
        "{err}"
    );
    assert!(
        out_ctx(
            &Cli::parse_from(["nfctl", "get", "shared-name", "-n", "team-b"]),
            &ctx
        )
        .await
        .contains("Paused")
    );
    // Lists span the cluster by default and show the namespace column.
    let all = out_ctx(&Cli::parse_from(["nfctl", "ls"]), &ctx).await;
    assert!(all.starts_with("NAMESPACE") && all.contains("team-a") && all.contains("other"));
}
