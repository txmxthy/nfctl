use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Namespace (defaults to the kubeconfig context's namespace)
    #[arg(short, long, global = true, env = "NFCTL_NAMESPACE")]
    pub namespace: Option<String>,

    /// All namespaces
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
        name: String,
    },

    /// Render a pipeline's topology
    Dag {
        /// Pipeline name
        name: String,
        /// Diagram format (`-o json|yaml` prints the topology model instead)
        #[arg(short, long, default_value = "ascii", value_enum)]
        format: DagFormat,
        /// Fit the ASCII render to this many columns (default: terminal width)
        #[arg(short, long, value_name = "COLS")]
        width: Option<usize>,
    },

    /// Logs from every pod of a pipeline (or one vertex), tagged by pod and container
    Logs {
        /// Pipeline name
        name: String,
        /// Restrict to one vertex
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
        name: String,
    },

    /// Inter-step buffer services
    Isb {
        #[command(subcommand)]
        command: IsbCommand,
    },

    /// Pause a pipeline (preview: not implemented yet, see docs/roadmap.md)
    Pause { name: String },

    /// Resume a pipeline (preview: not implemented yet, see docs/roadmap.md)
    Resume { name: String },

    /// Restart a pipeline or one vertex safely (preview: not implemented yet, see docs/roadmap.md)
    Recycle {
        name: String,
        vertex: Option<String>,
    },

    /// Wait for a pipeline phase (preview: not implemented yet, see docs/roadmap.md)
    Wait { name: String },

    /// Apply a pipeline spec, checking for unsafe changes (preview: not implemented yet, see docs/roadmap.md)
    Apply,

    /// Scale a vertex (preview: not implemented yet, see docs/roadmap.md)
    Scale {
        name: String,
        vertex: String,
        replicas: u32,
    },

    #[command(about = "MonoVertex operations (preview: not implemented yet, see docs/roadmap.md)")]
    Mvtx,

    /// Interactive terminal UI (preview: not implemented yet, see docs/roadmap.md)
    Tui,

    /// Generate shell completions
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DagFormat {
    Ascii,
    Mermaid,
    Dot,
}

#[derive(Debug, Subcommand)]
pub enum IsbCommand {
    /// List ISB services
    Ls,
    /// Inspect one ISB service
    Inspect { name: String },
}

impl Command {
    /// The roadmap name of a command that is still a stub.
    #[must_use]
    pub fn stub_name(&self) -> Option<&'static str> {
        Some(match self {
            Command::Pause { .. } => "pause",
            Command::Resume { .. } => "resume",
            Command::Recycle { .. } => "recycle",
            Command::Wait { .. } => "wait",
            Command::Apply => "apply",
            Command::Scale { .. } => "scale",
            Command::Mvtx => "mvtx",
            Command::Tui => "tui",
            Command::Ls
            | Command::Get { .. }
            | Command::Dag { .. }
            | Command::Logs { .. }
            | Command::Top { .. }
            | Command::Status { .. }
            | Command::Isb { .. }
            | Command::Completions { .. } => {
                return None;
            }
        })
    }
}
