//! Domain model, ports and services for `nfctl`.
//!
//! This crate has no I/O. Adapters (`nfctl-k8s`, `nfctl-daemon`) implement the
//! port traits defined here; the CLI and TUI call the services.

pub mod error;

pub use error::Error;

/// Crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;
