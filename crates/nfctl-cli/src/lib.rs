//! The `nfctl` command tree and its renderers. `main.rs` is only the composition
//! root; everything here is testable against the in-memory fakes.

mod cli;
mod commands;
mod output;

pub use cli::{Cli, Command, Globals, OutputFormat};
pub use commands::{Context, run, run_offline};
