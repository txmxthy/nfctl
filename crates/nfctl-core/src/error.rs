/// Every failure `nfctl` can report. Adapters wrap their own errors as `source`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The command exists but is not implemented yet. See `docs/roadmap.md`.
    #[error("`{0}` is not implemented yet (see docs/roadmap.md)")]
    Unimplemented(&'static str),
}

impl Error {
    /// Process exit code for this error. 1 generic, 2 usage (clap), 3 check failed, 4 unimplemented.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Unimplemented(_) => 4,
        }
    }
}
