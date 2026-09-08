use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::engine::ArgValueCompleter;

use crate::complete;

/// Operate Numaflow pipelines from the terminal.
#[derive(Debug, Parser)]
#[command(name = "nfctl", version, about, long_about = None, propagate_version = true)]
pub struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Args)]
pub struct Globals {
    /// Namespace. Without it, lists span every namespace and a bare name resolves across them
    #[arg(short, long, global = true, env = "NFCTL_NAMESPACE", add = ArgValueCompleter::new(complete::namespaces))]
    pub namespace: Option<String>,

    /// All namespaces (already the default without -n; kept for muscle memory)
    #[arg(short = 'A', long, global = true, conflicts_with = "namespace")]
    pub all_namespaces: bool,

    /// kubeconfig context
    #[arg(long, global = true, env = "NFCTL_CONTEXT")]
    pub context: Option<String>,

    /// Per-request timeout in seconds
    #[arg(long, global = true, default_value_t = 20, value_name = "SECS")]
    pub request_timeout: u64,

    /// Output format
    #[arg(short, long, global = true, default_value = "table", value_enum)]
    pub output: OutputFormat,

    /// Talk to the pipeline daemon at this URL instead of port-forwarding
    #[arg(long, global = true, env = "NFCTL_DAEMON_URL", value_name = "URL")]
    pub daemon_url: Option<String>,

    /// Disable colour (also honours the `NO_COLOR` environment variable)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Answer from this fixture file instead of a cluster (demos, tests, screenshots)
    #[arg(long, global = true, env = "NFCTL_FIXTURE", value_name = "FILE", conflicts_with_all = ["context", "daemon_url"])]
    pub fixture: Option<String>,
}

impl Globals {
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Wide,
    Json,
    Yaml,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List pipelines
    #[command(alias = "list")]
    Ls,

    /// Show one pipeline
    Get {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
    },

    /// Render a pipeline's topology
    Dag {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Diagram format (`-o json|yaml` prints the topology model instead)
        #[arg(short, long, default_value = "ascii", value_enum)]
        format: DagFormat,
        /// Fit the ASCII render to this many columns (default: terminal width)
        #[arg(short, long, value_name = "COLS")]
        width: Option<usize>,
        /// Draw every shard (`name-0`, `name-1`, ...) instead of one `name ×N` node
        #[arg(long)]
        expand_shards: bool,
    },

    /// Logs from every pod of a pipeline (or one vertex), tagged by pod and container
    Logs {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Restrict to one vertex
        #[arg(add = ArgValueCompleter::new(complete::vertices))]
        vertex: Option<String>,
        /// Container(s) to read; default is `numa` plus the pod's default container
        #[arg(short, long = "container", value_name = "NAME")]
        containers: Vec<String>,
        /// Every non-init container
        #[arg(long, conflicts_with = "containers")]
        all_containers: bool,
        /// Keep following; new pods are picked up and restarts resumed
        #[arg(short, long)]
        follow: bool,
        /// Only lines newer than this, e.g. `10m`, `2h`
        #[arg(long, value_name = "DURATION")]
        since: Option<String>,
        /// Last N lines of each container's backlog
        #[arg(long, value_name = "N")]
        tail: Option<u32>,
        /// Prefix each line with its timestamp
        #[arg(long)]
        timestamps: bool,
    },

    /// Phase, health, rates, pending and buffer usage, refreshed live
    Top {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Refresh interval in seconds
        #[arg(short, long, default_value_t = 2, value_name = "SECS")]
        interval: u64,
        /// Render once and exit
        #[arg(long)]
        once: bool,
    },

    /// Phase, health, rates, pending and buffer usage, once
    Status {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
    },

    /// Inter-step buffer services
    Isb {
        #[command(subcommand)]
        command: IsbCommand,
    },

    /// Pause a pipeline: sources stop, buffers drain, pods scale to zero
    Pause {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Block until the phase is Paused and report whether buffers drained
        #[arg(short, long)]
        wait: bool,
        /// Give up waiting after this many seconds
        #[arg(long, default_value_t = 120, value_name = "SECS")]
        timeout: u64,
        /// Validate on the server without changing anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Resume a paused pipeline
    Resume {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// `fast` restores the replica counts from before the pause; `slow` starts at the minimum
        #[arg(short, long, default_value = "fast", value_enum)]
        strategy: Strategy,
        /// Validate on the server without changing anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Restart one vertex (its pods, one at a time) or a whole pipeline (pause, drain, resume)
    Recycle {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Restrict to one vertex
        #[arg(add = ArgValueCompleter::new(complete::vertices))]
        vertex: Option<String>,
        /// Give up waiting (for the pause, or for each replaced pod) after this many seconds
        #[arg(long, default_value_t = 120, value_name = "SECS")]
        timeout: u64,
        /// Delete every pod of the vertex at once instead of one at a time
        #[arg(long)]
        all_at_once: bool,
        /// Show what would happen without changing anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Block until a pipeline reaches a phase
    Wait {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Phase to wait for
        #[arg(short, long, value_enum)]
        phase: Phase,
        /// Give up after this many seconds
        #[arg(long, default_value_t = 300, value_name = "SECS")]
        timeout: u64,
    },

    /// Apply a Pipeline manifest, refusing changes that need delete-and-recreate
    Apply {
        /// Manifest file (`-` for stdin)
        #[arg(short, long, value_name = "FILE")]
        file: String,
        /// Report what would change and exit 3 on a blocking change, without applying
        #[arg(long)]
        check: bool,
        /// Server-side dry run (still runs the checks)
        #[arg(long)]
        dry_run: bool,
    },

    /// Set a vertex's replica count
    Scale {
        /// Pipeline name
        #[arg(add = ArgValueCompleter::new(complete::pipelines))]
        name: String,
        /// Vertex name
        #[arg(add = ArgValueCompleter::new(complete::vertices))]
        vertex: String,
        /// Replica count
        replicas: u32,
        /// Validate on the server without changing anything
        #[arg(long)]
        dry_run: bool,
    },

    #[command(about = "MonoVertex operations")]
    Mvtx {
        #[command(subcommand)]
        command: MvtxCommand,
    },

    /// Interactive terminal UI
    Tui {
        /// Refresh interval in seconds
        #[arg(short, long, default_value_t = 2, value_name = "SECS")]
        interval: u64,
    },

    /// Every command and option in one tree: the whole surface, for pruning
    Map,

    /// Generate shell completions
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Strategy {
    Fast,
    Slow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Phase {
    Running,
    Paused,
    Pausing,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DagFormat {
    Ascii,
    Mermaid,
    Dot,
}

#[derive(Debug, Subcommand)]
pub enum MvtxCommand {
    #[command(alias = "list", about = "List MonoVertices")]
    Ls,
    #[command(about = "Show one MonoVertex")]
    Get {
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        name: String,
    },
    #[command(about = "Phase, health, rate and pending from the MonoVertex daemon")]
    Status {
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        name: String,
    },
    #[command(about = "Logs from the MonoVertex pods")]
    Logs {
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        name: String,
        /// Keep following
        #[arg(short, long)]
        follow: bool,
        /// Every non-init container (default: `numa` plus the pod's default container)
        #[arg(long)]
        all_containers: bool,
        /// Last N lines of each container's backlog
        #[arg(long, value_name = "N")]
        tail: Option<u32>,
    },
    /// Pause: set desired phase Paused
    Pause {
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Resume: set desired phase Running and hand replicas back to the autoscaler
    Resume {
        #[arg(add = ArgValueCompleter::new(complete::monovertices))]
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum IsbCommand {
    /// List ISB services
    Ls,
    /// Inspect one ISB service
    Inspect {
        #[arg(add = ArgValueCompleter::new(complete::isbs))]
        name: String,
    },
}

impl Command {
    /// The roadmap name of a command that is still a stub. Every command is
    /// implemented today; this stays so a future preview command exits 4 cleanly.
    #[must_use]
    pub fn stub_name(&self) -> Option<&'static str> {
        None
    }
}
