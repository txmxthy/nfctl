//! Composition root: parse args, connect adapters, run, print, exit.
#![allow(clippy::print_stderr)]

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use futures::StreamExt;
use nfctl_cli::{Cli, Context, Output, run, run_offline};
use nfctl_core::service::{NoDaemon, PipelineService};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(&cli).await {
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
    let conn = nfctl_k8s::connect(&nfctl_k8s::ClientOptions {
        context: cli.globals.context.clone(),
        request_timeout: Some(cli.globals.timeout()),
    })
    .await?;
    let cluster: Arc<dyn nfctl_core::ports::ClusterPort> =
        Arc::new(nfctl_k8s::KubeCluster::new(conn.client));
    let ctx = Context {
        cluster: Arc::clone(&cluster),
        service: PipelineService::new(cluster, Arc::new(NoDaemon)),
        default_namespace: conn.default_namespace,
        now: nfctl_core::model::Timestamp::now(),
        terminal_width: terminal_size::terminal_size().map(|(w, _)| usize::from(w.0)),
    };
    run(cli, &ctx).await
}
