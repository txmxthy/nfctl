//! Composition root: parse args, connect adapters, run, print, exit.
#![allow(clippy::print_stderr)]

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use nfctl_cli::{Cli, Context, run, run_offline};
use nfctl_core::service::{NoDaemon, PipelineService};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(&cli).await {
        Ok(out) => {
            let mut stdout = std::io::stdout().lock();
            // A closed pipe (`| head`) is not an error worth reporting.
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.flush();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nfctl: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

async fn execute(cli: &Cli) -> nfctl_core::Result<String> {
    if let Some(offline) = run_offline(cli) {
        return offline;
    }
    let conn = nfctl_k8s::connect(&nfctl_k8s::ClientOptions {
        context: cli.globals.context.clone(),
        request_timeout: Some(cli.globals.timeout()),
    })
    .await?;
    let cluster = Arc::new(nfctl_k8s::KubeCluster::new(conn.client));
    let ctx = Context {
        service: PipelineService::new(cluster, Arc::new(NoDaemon)),
        default_namespace: conn.default_namespace,
        now: nfctl_core::model::Timestamp::now(),
    };
    run(cli, &ctx).await
}
