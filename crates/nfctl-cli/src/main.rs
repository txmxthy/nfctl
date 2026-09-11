//! Composition root: parse args, connect adapters, run, print, exit.
#![allow(clippy::print_stderr)]

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{CommandFactory as _, Parser};
use futures::StreamExt;
use nfctl_cli::{Cli, Context, Output, run, run_offline};
use nfctl_core::ports::DaemonConnector;
use nfctl_core::service::PipelineService;

fn main() -> ExitCode {
    // Shell completion requests are answered here and exit; nothing may have
    // written to stdout yet. Runs outside the async runtime because a
    // completer that queries the cluster builds its own.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
    let cli = Cli::parse();
    nfctl_cli::timing::install(cli.globals.timings, cli.owns_the_terminal());
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        eprintln!("nfctl: cannot start the async runtime");
        return ExitCode::FAILURE;
    };
    rt.block_on(async_main(&cli))
}

async fn async_main(cli: &Cli) -> ExitCode {
    match execute(cli).await {
        Ok(Output::Text(out)) => {
            let mut stdout = std::io::stdout().lock();
            // A closed pipe (`| head`) is not an error worth reporting.
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.flush();
            ExitCode::SUCCESS
        }
        Ok(Output::Lines(mut lines)) => {
            let mut stdout = std::io::stdout().lock();
            while let Some(l) = lines.next().await {
                if writeln!(stdout, "{l}")
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    break;
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nfctl: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

async fn execute(cli: &Cli) -> nfctl_core::Result<Output> {
    if let Some(offline) = run_offline(cli) {
        return offline.map(Output::Text);
    }
    if let Some(path) = &cli.globals.fixture {
        let text = std::fs::read_to_string(path)
            .map_err(|e| nfctl_core::Error::Usage(format!("reading fixture `{path}`: {e}")))?;
        let fixture = nfctl_core::fake::Fixture::parse(&text)
            .map_err(|e| nfctl_core::Error::Usage(format!("fixture `{path}`: {e}")))?;
        let cluster: Arc<dyn nfctl_core::ports::ClusterPort> =
            Arc::new(nfctl_core::fake::FakeCluster::from_fixture(&fixture));
        let daemons: Arc<dyn DaemonConnector> =
            Arc::new(nfctl_core::fake::FakeDaemons::from_fixture(&fixture));
        let default_namespace = fixture
            .pipelines
            .first()
            .map_or_else(nfctl_core::model::Namespace::default_ns, |p| {
                p.key.namespace.clone()
            });
        let ctx = Context {
            cluster: Arc::clone(&cluster),
            service: PipelineService::new(cluster, daemons),
            default_namespace,
            now: nfctl_core::model::Timestamp::now(),
            terminal_width: terminal_size::terminal_size().map(|(w, _)| usize::from(w.0)),
            colour: colour_enabled(cli),
        };
        return run(cli, &ctx).await;
    }
    let conn = nfctl_k8s::connect(&nfctl_k8s::ClientOptions {
        context: cli.globals.context.clone(),
        request_timeout: Some(cli.globals.timeout()),
    })
    .await?;
    let cluster: Arc<dyn nfctl_core::ports::ClusterPort> =
        Arc::new(nfctl_k8s::KubeCluster::new(conn.client.clone()));
    let daemon_opts = nfctl_daemon::ClientOptions::default();
    let daemons: Arc<dyn DaemonConnector> = match &cli.globals.daemon_url {
        Some(url) => Arc::new(nfctl_daemon::DirectConnector::new(url, None, daemon_opts)?),
        None => Arc::new(nfctl_daemon::PortForwardConnector::new(
            conn.client,
            Arc::clone(&cluster),
            daemon_opts,
        )),
    };
    let ctx = Context {
        cluster: Arc::clone(&cluster),
        service: PipelineService::new(cluster, daemons),
        default_namespace: conn.default_namespace,
        now: nfctl_core::model::Timestamp::now(),
        terminal_width: terminal_size::terminal_size().map(|(w, _)| usize::from(w.0)),
        colour: colour_enabled(cli),
    };
    run(cli, &ctx).await
}

/// Colour only on a terminal, and only when neither `--no-color` nor the
/// `NO_COLOR` environment variable (<https://no-color.org>) says otherwise.
fn colour_enabled(cli: &Cli) -> bool {
    use std::io::IsTerminal as _;
    let env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    std::io::stdout().is_terminal() && !cli.globals.no_color && !env
}
