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

    /// Render a pipeline's topology (preview: not implemented yet, see docs/roadmap.md)
    Dag { name: String },

    /// Tail logs across a pipeline's pods (preview: not implemented yet, see docs/roadmap.md)
    Logs {
        name: String,
        vertex: Option<String>,
    },

    /// Live phase, rates, pending and buffer usage (preview: not implemented yet, see docs/roadmap.md)
    Top { name: String },

    /// Status fused from the CRD and the daemon (preview: not implemented yet, see docs/roadmap.md)
    Status { name: String },

    /// Inter-step buffer services (preview: not implemented yet, see docs/roadmap.md)
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
            Command::Dag { .. } => "dag",
            Command::Logs { .. } => "logs",
            Command::Top { .. } => "top",
            Command::Status { .. } => "status",
            Command::Isb { .. } => "isb",
            Command::Pause { .. } => "pause",
            Command::Resume { .. } => "resume",
            Command::Recycle { .. } => "recycle",
            Command::Wait { .. } => "wait",
            Command::Apply => "apply",
            Command::Scale { .. } => "scale",
            Command::Mvtx => "mvtx",
            Command::Tui => "tui",
            Command::Ls | Command::Get { .. } | Command::Completions { .. } => return None,
        })
    }
}
