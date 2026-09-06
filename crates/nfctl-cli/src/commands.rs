use std::sync::Arc;
use std::time::Duration;

use clap::CommandFactory;
use futures::StreamExt;
use futures::stream::BoxStream;
use nfctl_core::model::{
    ContainerName, Namespace, PipelineKey, PipelineName, Selector, TaggedLine, Timestamp,
    VertexName,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::service::PipelineService;
use nfctl_core::service::logs::{ContainerSelect, TailOptions, snapshot, tail};
use nfctl_core::{Error, Result};

use crate::cli::{Cli, Command, DagFormat, OutputFormat};
use crate::output;

/// What a command produces: a finished string, or lines as they arrive.
pub enum Output {
    Text(String),
    Lines(BoxStream<'static, String>),
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Output::Text(s) => f.debug_tuple("Text").field(s).finish(),
            Output::Lines(_) => f.write_str("Lines(..)"),
        }
    }
}

impl Output {
    /// Collect everything into one string (tests, and callers that do not stream).
    pub async fn collect(self) -> String {
        match self {
            Output::Text(s) => s,
            Output::Lines(mut lines) => {
                let mut out = String::new();
                while let Some(l) = lines.next().await {
                    out.push_str(&l);
                    out.push('\n');
                }
                out
            }
        }
    }
}

/// Everything a command needs besides its arguments. Built once in `main`.
#[derive(Clone)]
pub struct Context {
    pub cluster: Arc<dyn ClusterPort>,
    pub service: PipelineService,
    pub default_namespace: Namespace,
    /// Injected so "AGE" columns are deterministic in tests.
    pub now: Timestamp,
    /// Columns available for ASCII diagrams; `None` when not a terminal.
    pub terminal_width: Option<usize>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("default_namespace", &self.default_namespace)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
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

/// Parse `10m`, `2h`, `30s`, `1d`.
fn parse_duration(s: &str) -> Result<Duration> {
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = num
        .parse()
        .map_err(|_| Error::Usage(format!("invalid duration `{s}`")))?;
    let mult = match unit {
        "s" | "" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => {
            return Err(Error::Usage(format!(
                "invalid duration `{s}` (use s, m, h or d)"
            )));
        }
    };
    Ok(Duration::from_secs(n * mult))
}

fn format_line(l: &TaggedLine, timestamps: bool) -> String {
    let stamp = match (timestamps, l.line.at) {
        (true, Some(at)) => format!("{at} "),
        _ => String::new(),
    };
    format!("{}{}/{} {}", stamp, l.pod, l.container, l.line.text)
}

/// Execute a parsed command against a cluster.
pub async fn run(cli: &Cli, ctx: &Context) -> Result<Output> {
    if let Some(offline) = run_offline(cli) {
        return offline.map(Output::Text);
    }
    if let Command::Logs {
        name,
        vertex,
        containers,
        all_containers,
        follow,
        since,
        tail: tail_lines,
        timestamps,
    } = &cli.command
    {
        let ns = ctx.namespace(cli)?;
        let name = PipelineName::new(name).map_err(|e| invalid_arg("pipeline", name, e))?;
        let vertex = vertex
            .as_deref()
            .map(|v| VertexName::new(v).map_err(|e| invalid_arg("vertex", v, e)))
            .transpose()?;
        let selector = Selector::vertex_pods(&name, vertex.as_ref());
        let containers = if *all_containers {
            ContainerSelect::All
        } else if containers.is_empty() {
            ContainerSelect::Default
        } else {
            ContainerSelect::Named(
                containers
                    .iter()
                    .map(|c| ContainerName::new(c).map_err(|e| invalid_arg("container", c, e)))
                    .collect::<Result<Vec<_>>>()?,
            )
        };
        let since = since
            .as_deref()
            .map(parse_duration)
            .transpose()?
            .map(|d| Timestamp::new(ctx.now.get() - d));
        let opts = TailOptions {
            containers,
            since,
            tail_lines: *tail_lines,
            ..TailOptions::default()
        };
        let timestamps = *timestamps;
        if *follow {
            let (lines, handle) = tail(Arc::clone(&ctx.cluster), ns, selector, opts).await?;
            // The handle rides along with the stream so the tails live exactly as long as it.
            let stream = lines.map(move |l| {
                let _keep = &handle;
                format_line(&l, timestamps)
            });
            return Ok(Output::Lines(Box::pin(stream)));
        }
        let lines = snapshot(ctx.cluster.as_ref(), &ns, &selector, &opts).await?;
        if lines.is_empty() {
            return Ok(Output::Text("No pods found.\n".to_owned()));
        }
        return Ok(Output::Text(
            lines
                .iter()
                .map(|l| format_line(l, timestamps) + "\n")
                .collect(),
        ));
    }
    run_text(cli, ctx).await.map(Output::Text)
}

/// Commands whose output is a single string.
async fn run_text(cli: &Cli, ctx: &Context) -> Result<String> {
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
        Command::Dag {
            name,
            format,
            width,
        } => {
            let ns = ctx.namespace(cli)?;
            let name = PipelineName::new(name).map_err(|e| invalid_arg("pipeline", name, e))?;
            let p = ctx.service.get(&PipelineKey::new(ns, name)).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Yaml => {
                    output::serialised(&p.spec.topology, fmt)
                }
                OutputFormat::Table | OutputFormat::Wide => {
                    let format = match format {
                        DagFormat::Ascii => nfctl_graph::Format::Ascii,
                        DagFormat::Mermaid => nfctl_graph::Format::Mermaid,
                        DagFormat::Dot => nfctl_graph::Format::Dot,
                    };
                    let width = width.or(ctx.terminal_width);
                    Ok(nfctl_graph::render(&p.spec.topology, format, width))
                }
            }
        }
        // Handled by `run_offline`; listed so the match stays exhaustive.
        Command::Completions { .. }
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
        | Command::Tui => unreachable!("handled before run_text"),
    }
}
