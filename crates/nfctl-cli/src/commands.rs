use clap::CommandFactory;
use nfctl_core::model::{Namespace, PipelineKey, PipelineName, Timestamp};
use nfctl_core::service::PipelineService;
use nfctl_core::{Error, Result};

use crate::cli::{Cli, Command, OutputFormat};
use crate::output;

/// Everything a command needs besides its arguments. Built once in `main`.
#[derive(Debug, Clone)]
pub struct Context {
    pub service: PipelineService,
    pub default_namespace: Namespace,
    /// Injected so "AGE" columns are deterministic in tests.
    pub now: Timestamp,
}

fn invalid_arg(what: &'static str, value: &str, e: impl std::fmt::Display) -> Error {
    Error::Usage(format!("invalid {what} `{value}`: {e}"))
}

impl Context {
    fn namespace(&self, cli: &Cli) -> Result<Namespace> {
        match &cli.globals.namespace {
            Some(ns) => Namespace::new(ns).map_err(|e| invalid_arg("namespace", ns, e)),
            None => Ok(self.default_namespace.clone()),
        }
    }
}

/// Commands that never touch a cluster: stubs and completions. `None` means the
/// command needs a [`Context`]; call [`run`].
#[must_use]
pub fn run_offline(cli: &Cli) -> Option<Result<String>> {
    if let Some(name) = cli.command.stub_name() {
        return Some(Err(Error::Unimplemented(name)));
    }
    match &cli.command {
        Command::Completions { shell } => {
            let mut buf = Vec::new();
            clap_complete::generate(*shell, &mut Cli::command(), "nfctl", &mut buf);
            Some(Ok(String::from_utf8_lossy(&buf).into_owned()))
        }
        _ => None,
    }
}

/// Execute a parsed command against a cluster and return what to print.
pub async fn run(cli: &Cli, ctx: &Context) -> Result<String> {
    if let Some(offline) = run_offline(cli) {
        return offline;
    }
    let fmt = cli.globals.output;
    match &cli.command {
        Command::Ls => {
            let ns = if cli.globals.all_namespaces {
                None
            } else {
                Some(ctx.namespace(cli)?)
            };
            let pipelines = ctx.service.list(ns.as_ref()).await?;
            output::pipelines(&pipelines, fmt, cli.globals.all_namespaces, ctx.now)
        }
        Command::Get { name } => {
            let ns = ctx.namespace(cli)?;
            let name = PipelineName::new(name).map_err(|e| invalid_arg("pipeline", name, e))?;
            let p = ctx.service.get(&PipelineKey::new(ns, name)).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Yaml => output::serialised(&p, fmt),
                OutputFormat::Table | OutputFormat::Wide => {
                    output::pipelines(std::slice::from_ref(&p), OutputFormat::Wide, false, ctx.now)
                }
            }
        }
        // Handled by `run_offline`; listed so the match stays exhaustive.
        Command::Completions { .. }
        | Command::Dag { .. }
        | Command::Logs { .. }
        | Command::Top { .. }
        | Command::Status { .. }
        | Command::Isb { .. }
        | Command::Pause { .. }
        | Command::Resume { .. }
        | Command::Recycle { .. }
        | Command::Wait { .. }
        | Command::Apply
        | Command::Scale { .. }
        | Command::Mvtx
        | Command::Tui => unreachable!("handled by run_offline"),
    }
}
