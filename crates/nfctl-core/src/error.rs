use crate::model::TopologyError;

/// Every failure `nfctl` can report. Adapter failures are boxed as `source`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("{kind} `{name}` not found")]
    NotFound { kind: &'static str, name: String },

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("cluster: {0}")]
    Cluster(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("daemon: {0}")]
    Daemon(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// A resource read from the cluster does not satisfy the domain invariants.
    #[error("invalid {kind} `{name}`: {reason}")]
    Invalid {
        kind: &'static str,
        name: String,
        reason: String,
    },

    #[error("invalid topology: {0}")]
    Topology(#[from] TopologyError),

    /// Bad arguments that clap could not catch (e.g. an invalid resource name).
    #[error("{0}")]
    Usage(String),

    /// The command exists but is not implemented yet. See `docs/roadmap.md`.
    #[error("`{0}` is not implemented yet (see docs/roadmap.md)")]
    Unimplemented(&'static str),
}

impl Error {
    /// Process exit code. 1 generic, 2 usage (clap), 3 check failed, 4 unimplemented.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Unimplemented(_) => 4,
            Error::Usage(_) => 2,
            Error::Invalid { .. } | Error::Topology(_) => 3,
            _ => 1,
        }
    }

    /// Wrap an adapter error as a cluster failure.
    pub fn cluster(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Error::Cluster(Box::new(e))
    }

    /// Wrap an adapter error as a daemon failure.
    pub fn daemon(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Error::Daemon(Box::new(e))
    }
}
