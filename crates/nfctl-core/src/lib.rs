//! Domain model, ports and services for `nfctl`.
//!
//! This crate has no I/O. Adapters (`nfctl-k8s`, `nfctl-daemon`) implement the
//! port traits in [`ports`]; the CLI and TUI call the [`service`] layer.

pub mod error;
pub mod model;
pub mod ports;
pub mod service;

#[cfg(feature = "fake")]
pub mod fake;

pub use error::Error;

/// Crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;
