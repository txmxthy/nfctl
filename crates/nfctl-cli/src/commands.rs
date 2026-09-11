use std::sync::Arc;
use std::time::Duration;

use clap::CommandFactory;
use futures::StreamExt;
use futures::stream::BoxStream;
use nfctl_core::model::{
    ContainerName, IsbName, MonoVertexKey, Namespace, PipelineKey, PipelineName, Selector,
    TaggedLine, Timestamp, VertexName,
};
use nfctl_core::model::{PipelinePhase, ResumeStrategy};
use nfctl_core::ports::ClusterPort;
use nfctl_core::service::logs::{ContainerSelect, TailOptions, snapshot, tail};
use nfctl_core::service::{PipelineService, lifecycle};
use nfctl_core::{Error, Result};

use crate::cli::{Cli, Command, DagFormat, IsbCommand, MvtxCommand, OutputFormat, Phase, Strategy};
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
    /// ANSI colour in diagrams: a terminal, and neither `--no-color` nor `NO_COLOR`.
    pub colour: bool,
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
    /// The namespace for manifests that do not name one: the kubeconfig default.
    fn namespace(&self, cli: &Cli) -> Result<Namespace> {
        match &cli.globals.namespace {
            Some(ns) => Namespace::new(ns).map_err(|e| invalid_arg("namespace", ns, e)),
            None => Ok(self.default_namespace.clone()),
        }
    }

    /// `Some` only when `-n` was given. Lists span the cluster otherwise: with a
    /// namespace per pipeline, the kubeconfig's namespace is rarely the right one.
    fn scope(cli: &Cli) -> Result<Option<Namespace>> {
        match &cli.globals.namespace {
            Some(ns) => Namespace::new(ns)
                .map(Some)
                .map_err(|e| invalid_arg("namespace", ns, e)),
            None => Ok(None),
        }
    }

    /// A pipeline by name: in `-n` when given, else wherever it uniquely exists.
    async fn resolve_pipeline(&self, cli: &Cli, name: &str) -> Result<PipelineKey> {
        let name = PipelineName::new(name).map_err(|e| invalid_arg("pipeline", name, e))?;
        if let Some(ns) = Self::scope(cli)? {
            return Ok(PipelineKey::new(ns, name));
        }
        let found: Vec<PipelineKey> = self
            .service
            .list(None)
            .await?
            .into_iter()
            .filter(|p| p.key.name == name)
            .map(|p| p.key)
            .collect();
        unique("pipeline", &name, found)
    }

    async fn resolve_monovertex(&self, cli: &Cli, name: &str) -> Result<MonoVertexKey> {
        let name = PipelineName::new(name).map_err(|e| invalid_arg("monovertex", name, e))?;
        if let Some(namespace) = Self::scope(cli)? {
            return Ok(MonoVertexKey { namespace, name });
        }
        let found: Vec<MonoVertexKey> = self
            .cluster
            .list_monovertices(None)
            .await?
            .into_iter()
            .filter(|m| m.key.name == name)
            .map(|m| m.key)
            .collect();
        unique("monovertex", &name, found)
    }
}

/// Exactly one match, or a usage error naming the namespaces to choose from.
fn unique<K: std::fmt::Display>(
    kind: &'static str,
    name: &PipelineName,
    mut found: Vec<K>,
) -> Result<K> {
    match found.len() {
        0 => Err(Error::NotFound {
            kind,
            name: name.to_string(),
        }),
        1 => Ok(found.remove(0)),
        _ => Err(Error::Usage(format!(
            "{kind} `{name}` exists in more than one namespace ({}); pass -n",
            found
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
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
        Command::Map => {
            let node = crate::map::describe(&Cli::command());
            Some(match cli.globals.output {
                OutputFormat::Json | OutputFormat::Yaml => {
                    output::serialised(&node, cli.globals.output)
                }
                OutputFormat::Table | OutputFormat::Wide => Ok(crate::map::render(&node)),
            })
        }
        Command::Completions { shell } => Some(
            crate::complete::registration(*shell)
                .ok_or_else(|| Error::Usage(format!("no dynamic completion for {shell}"))),
        ),
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
    if let Some(out) = run_logs(cli, ctx).await? {
        return Ok(out);
    }
    if let Command::Mvtx { command } = &cli.command {
        return run_mvtx(cli, ctx, command).await;
    }
    if let Command::Top {
        name,
        interval,
        once: false,
    } = &cli.command
    {
        let key = ctx.resolve_pipeline(cli, name).await?;
        let fmt = cli.globals.output;
        let service = ctx.service.clone();
        let every = Duration::from_secs((*interval).max(1));
        let frames = futures::stream::unfold((service, key), move |(service, key)| async move {
            let frame = match service.view(&key, Timestamp::now()).await {
                Ok(v) => output::view(&v, fmt).unwrap_or_else(|e| format!("error: {e}\n")),
                Err(e) => format!("error: {e}\n"),
            };
            tokio::time::sleep(every).await;
            // Clear screen + home, then the frame (no trailing newline: the printer adds one).
            Some((
                format!("\x1b[2J\x1b[H{}", frame.trim_end_matches('\n')),
                (service, key),
            ))
        });
        return Ok(Output::Lines(Box::pin(frames)));
    }
    run_text(cli, ctx).await.map(Output::Text)
}

/// Commands whose output is a single string.
async fn run_text(cli: &Cli, ctx: &Context) -> Result<String> {
    let fmt = cli.globals.output;
    match &cli.command {
        Command::Ls => {
            let ns = Context::scope(cli)?;
            let pipelines = ctx.service.list(ns.as_ref()).await?;
            output::pipelines(&pipelines, fmt, ns.is_none(), ctx.now)
        }
        Command::Get { name } => {
            let p = ctx
                .service
                .get(&ctx.resolve_pipeline(cli, name).await?)
                .await?;
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
            expand_shards,
        } => {
            let p = ctx
                .service
                .get(&ctx.resolve_pipeline(cli, name).await?)
                .await?;
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
                    let opts = nfctl_graph::RenderOptions {
                        width: width.or(ctx.terminal_width),
                        collapse_shards: !expand_shards,
                        colour: ctx.colour,
                    };
                    Ok(nfctl_graph::render_with(&p.spec.topology, format, opts))
                }
            }
        }
        // Handled by `run_offline`; listed so the match stays exhaustive.
        Command::Status { name }
        | Command::Top {
            name, once: true, ..
        } => {
            let key = ctx.resolve_pipeline(cli, name).await?;
            let v = ctx.service.view(&key, ctx.now).await?;
            output::view(&v, fmt)
        }
        Command::Isb { command } => run_isb(cli, ctx, command).await,
        Command::Pause { .. }
        | Command::Resume { .. }
        | Command::Recycle { .. }
        | Command::Wait { .. }
        | Command::Apply { .. }
        | Command::Scale { .. } => run_lifecycle(cli, ctx).await,
        // Handled above; listed so the match stays exhaustive.
        Command::Completions { .. }
        | Command::Logs { .. }
        | Command::Top { .. }
        | Command::Tui { .. }
        | Command::Map
        | Command::Mvtx { .. } => unreachable!("handled before run_text"),
    }
}

async fn run_isb(cli: &Cli, ctx: &Context, command: &IsbCommand) -> Result<String> {
    let fmt = cli.globals.output;
    let ns = Context::scope(cli)?;
    match command {
        IsbCommand::Ls => output::isbs(&ctx.service.list_isb(ns.as_ref()).await?, fmt),
        IsbCommand::Inspect { name } => {
            let name = IsbName::new(name).map_err(|e| invalid_arg("isbsvc", name, e))?;
            let mut found: Vec<_> = ctx
                .service
                .list_isb(ns.as_ref())
                .await?
                .into_iter()
                .filter(|i| i.name == name)
                .collect();
            let isb = match found.len() {
                0 => {
                    return Err(Error::NotFound {
                        kind: "isbsvc",
                        name: name.to_string(),
                    });
                }
                1 => found.remove(0),
                _ => {
                    let nss: Vec<String> = found.iter().map(|i| i.namespace.to_string()).collect();
                    return Err(Error::Usage(format!(
                        "isbsvc `{name}` exists in {} namespaces ({}); pass -n",
                        nss.len(),
                        nss.join(", ")
                    )));
                }
            };
            // A pipeline references its ISB by name within its own namespace.
            let users: Vec<_> = ctx
                .service
                .list(Some(&isb.namespace))
                .await?
                .into_iter()
                .filter(|p| p.spec.isb == name)
                .collect();
            output::isb_detail(&isb, &users, fmt)
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn run_lifecycle(cli: &Cli, ctx: &Context) -> Result<String> {
    let fmt = cli.globals.output;
    let cluster = ctx.cluster.as_ref();
    let daemons = ctx.service.daemons();
    match &cli.command {
        Command::Pause {
            name,
            wait,
            timeout,
            dry_run,
        } => {
            let key = ctx.resolve_pipeline(cli, name).await?;
            let wait = wait.then(|| Duration::from_secs(*timeout));
            let r = lifecycle::pause(cluster, daemons, &key, wait, *dry_run).await?;
            if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
                return output::serialised(&r, fmt);
            }
            let drained = match r.drained {
                Some(true) => "buffers drained",
                Some(false) => "buffers still draining",
                None => "drain state unknown",
            };
            Ok(match (r.dry_run, r.phase) {
                (true, _) => format!("{key}: pause accepted (dry run)\n"),
                (false, PipelinePhase::Paused) => format!("{key}: Paused; {drained}\n"),
                (false, phase) => format!(
                    "{key}: desired Paused, currently {}; {drained}\n",
                    phase.as_str()
                ),
            })
        }
        Command::Resume {
            name,
            strategy,
            dry_run,
        } => {
            let key = ctx.resolve_pipeline(cli, name).await?;
            let strategy = match strategy {
                Strategy::Fast => ResumeStrategy::Fast,
                Strategy::Slow => ResumeStrategy::Slow,
            };
            lifecycle::resume(cluster, &key, strategy, *dry_run).await?;
            Ok(format!(
                "{key}: resume ({}){}\n",
                strategy.as_str(),
                if *dry_run { " accepted (dry run)" } else { "" }
            ))
        }
        Command::Recycle {
            name,
            vertex,
            timeout,
            all_at_once,
            dry_run,
        } => {
            let key = ctx.resolve_pipeline(cli, name).await?;
            let vertex = vertex
                .as_deref()
                .map(|v| VertexName::new(v).map_err(|e| invalid_arg("vertex", v, e)))
                .transpose()?;
            let r = lifecycle::recycle(
                cluster,
                daemons,
                &key,
                vertex.as_ref(),
                Duration::from_secs(*timeout),
                *all_at_once,
                *dry_run,
            )
            .await?;
            if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
                return output::serialised(&r, fmt);
            }
            let suffix = if *dry_run { " (dry run)" } else { "" };
            Ok(match r {
                lifecycle::RecycleReport::Pods { vertex, deleted } => {
                    let how = if *all_at_once { "" } else { ", one at a time" };
                    let mut s = format!(
                        "{key}/{vertex}: restarted {} pod(s){how}{suffix}\n",
                        deleted.len()
                    );
                    for p in deleted {
                        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("  {p}\n"));
                    }
                    s
                }
                lifecycle::RecycleReport::Pipeline { drained, .. } => format!(
                    "{key}: paused{}, resumed (fast){suffix}\n",
                    match drained {
                        Some(true) => " and drained",
                        Some(false) => " (buffers not empty)",
                        None => "",
                    }
                ),
            })
        }
        Command::Wait {
            name,
            phase,
            timeout,
        } => {
            let key = ctx.resolve_pipeline(cli, name).await?;
            let phase = match phase {
                Phase::Running => PipelinePhase::Running,
                Phase::Paused => PipelinePhase::Paused,
                Phase::Pausing => PipelinePhase::Pausing,
                Phase::Failed => PipelinePhase::Failed,
            };
            let p = lifecycle::wait_for_phase(cluster, &key, phase, Duration::from_secs(*timeout))
                .await?;
            Ok(format!("{key}: {}\n", p.status.phase.as_str()))
        }
        Command::Scale {
            name,
            vertex,
            replicas,
            dry_run,
        } => {
            let key = ctx.resolve_pipeline(cli, name).await?;
            let vertex = VertexName::new(vertex).map_err(|e| invalid_arg("vertex", vertex, e))?;
            cluster
                .scale_vertex(&key, &vertex, *replicas, *dry_run)
                .await?;
            Ok(format!(
                "{key}/{vertex}: replicas={replicas}{}\n",
                if *dry_run { " (dry run)" } else { "" }
            ))
        }
        Command::Apply {
            file,
            check,
            dry_run,
        } => run_apply(cli, ctx, file, *check, *dry_run).await,
        _ => unreachable!("run_lifecycle is only called for lifecycle commands"),
    }
}

async fn run_apply(
    cli: &Cli,
    ctx: &Context,
    file: &str,
    check: bool,
    dry_run: bool,
) -> Result<String> {
    let fmt = cli.globals.output;
    let text = if file == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
            .map_err(|e| Error::Usage(format!("reading stdin: {e}")))?;
        s
    } else {
        std::fs::read_to_string(file).map_err(|e| Error::Usage(format!("reading `{file}`: {e}")))?
    };
    let ns = ctx.namespace(cli)?;
    let new = ctx.cluster.parse_manifest(&text, &ns)?;
    let live = match ctx.cluster.get_pipeline(&new.key).await {
        Ok(p) => Some(p),
        Err(Error::NotFound { .. }) => None,
        Err(e) => return Err(e),
    };
    let backlog = match (&live, ctx.service.daemons().connect(&new.key, None).await) {
        (Some(_), Ok(d)) => d
            .buffers()
            .await
            .ok()
            .map(|b| b.iter().filter_map(|x| x.pending).sum::<i64>()),
        _ => None,
    };
    let report = nfctl_core::service::check(live.as_ref(), &new, backlog);
    if check {
        if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
            let s = output::serialised(&report, fmt)?;
            return if report.ok() {
                Ok(s)
            } else {
                Err(Error::CheckFailed(s))
            };
        }
        let s = output::apply_report(&new.key, &report, live.is_some());
        return if report.ok() {
            Ok(s)
        } else {
            Err(Error::CheckFailed(s))
        };
    }
    if !report.ok() {
        return Err(Error::CheckFailed(output::apply_report(
            &new.key,
            &report,
            live.is_some(),
        )));
    }
    let applied = ctx.cluster.apply_manifest(&text, &ns, dry_run).await?;
    let mut s = output::apply_report(&new.key, &report, live.is_some());
    let _ = std::fmt::Write::write_fmt(
        &mut s,
        format_args!(
            "{}: {}{}\n",
            applied.key,
            if live.is_some() {
                "configured"
            } else {
                "created"
            },
            if dry_run { " (server dry run)" } else { "" }
        ),
    );
    Ok(s)
}

/// `logs`: snapshot or follow. `None` when the command is something else.
async fn run_logs(cli: &Cli, ctx: &Context) -> Result<Option<Output>> {
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
        let PipelineKey {
            namespace: ns,
            name,
        } = ctx.resolve_pipeline(cli, name).await?;
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
            return Ok(Some(Output::Lines(Box::pin(stream))));
        }
        let lines = snapshot(ctx.cluster.as_ref(), &ns, &selector, &opts).await?;
        if lines.is_empty() {
            return Ok(Some(Output::Text("No pods found.\n".to_owned())));
        }
        return Ok(Some(Output::Text(
            lines
                .iter()
                .map(|l| format_line(l, timestamps) + "\n")
                .collect(),
        )));
    }
    if let Command::Tui { interval } = &cli.command {
        let ns = Context::scope(cli)?;
        nfctl_tui::run(
            Arc::clone(&ctx.cluster),
            ctx.service.clone(),
            ns,
            Duration::from_secs((*interval).max(1)),
            nfctl_tui::Palette::detect(cli.globals.no_color),
            cli.globals.timings,
        )
        .await
        .map_err(|e| Error::Cluster(Box::new(e)))?;
        return Ok(Some(Output::Text(String::new())));
    }
    Ok(None)
}

async fn run_mvtx(cli: &Cli, ctx: &Context, command: &MvtxCommand) -> Result<Output> {
    let fmt = cli.globals.output;
    let cluster = ctx.cluster.as_ref();
    match command {
        MvtxCommand::Ls => {
            let ns = Context::scope(cli)?;
            let mut items = cluster.list_monovertices(ns.as_ref()).await?;
            items.sort_by(|a, b| {
                (&a.key.namespace, &a.key.name).cmp(&(&b.key.namespace, &b.key.name))
            });
            output::monovertices(&items, fmt, ns.is_none(), ctx.now).map(Output::Text)
        }
        MvtxCommand::Get { name } => {
            let key = ctx.resolve_monovertex(cli, name).await?;
            let m = cluster.get_monovertex(&key).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Yaml => output::serialised(&m, fmt),
                OutputFormat::Table | OutputFormat::Wide => output::monovertices(
                    std::slice::from_ref(&m),
                    OutputFormat::Wide,
                    false,
                    ctx.now,
                ),
            }
            .map(Output::Text)
        }
        MvtxCommand::Status { name } => {
            let key = ctx.resolve_monovertex(cli, name).await?;
            let m = cluster.get_monovertex(&key).await?;
            let mut warnings = Vec::new();
            let (health, metrics) = match ctx.service.daemons().connect_monovertex(&key).await {
                Ok(d) => (
                    d.health()
                        .await
                        .map_err(|e| warnings.push(format!("health: {e}")))
                        .ok(),
                    d.vertex_metrics(None)
                        .await
                        .map_err(|e| warnings.push(format!("metrics: {e}")))
                        .ok(),
                ),
                Err(e) => {
                    warnings.push(format!("daemon unavailable: {e}"));
                    (None, None)
                }
            };
            output::monovertex_status(&m, health.as_ref(), metrics.as_deref(), &warnings, fmt)
                .map(Output::Text)
        }
        MvtxCommand::Logs {
            name,
            follow,
            all_containers,
            tail: tail_lines,
        } => run_mvtx_logs(cli, ctx, name, *follow, *all_containers, *tail_lines).await,
        MvtxCommand::Pause { name, dry_run } => {
            let key = ctx.resolve_monovertex(cli, name).await?;
            cluster
                .set_monovertex_lifecycle(&key, nfctl_core::model::DesiredPhase::Paused, *dry_run)
                .await?;
            Ok(Output::Text(format!(
                "{key}: pause{}\n",
                if *dry_run { " accepted (dry run)" } else { "" }
            )))
        }
        MvtxCommand::Resume { name, dry_run } => {
            let key = ctx.resolve_monovertex(cli, name).await?;
            cluster
                .set_monovertex_lifecycle(&key, nfctl_core::model::DesiredPhase::Running, *dry_run)
                .await?;
            Ok(Output::Text(format!(
                "{key}: resume{}\n",
                if *dry_run { " accepted (dry run)" } else { "" }
            )))
        }
    }
}

async fn run_mvtx_logs(
    cli: &Cli,
    ctx: &Context,
    name: &str,
    follow: bool,
    all_containers: bool,
    tail_lines: Option<u32>,
) -> Result<Output> {
    let key = ctx.resolve_monovertex(cli, name).await?;
    let cluster = ctx.cluster.as_ref();
    let selector = Selector::monovertex_pods(&key.name);
    let containers = if all_containers {
        ContainerSelect::All
    } else {
        ContainerSelect::Default
    };
    let opts = TailOptions {
        containers,
        tail_lines,
        ..TailOptions::default()
    };
    if follow {
        let (lines, handle) = tail(Arc::clone(&ctx.cluster), key.namespace, selector, opts).await?;
        let stream = lines.map(move |l| {
            let _keep = &handle;
            format_line(&l, false)
        });
        return Ok(Output::Lines(Box::pin(stream)));
    }
    let lines = snapshot(cluster, &key.namespace, &selector, &opts).await?;
    if lines.is_empty() {
        return Ok(Output::Text("No pods found.\n".to_owned()));
    }
    Ok(Output::Text(
        lines.iter().map(|l| format_line(l, false) + "\n").collect(),
    ))
}
