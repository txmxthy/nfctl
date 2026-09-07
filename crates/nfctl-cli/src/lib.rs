//! The `nfctl` command tree and its renderers. `main.rs` is only the composition
//! root; everything here is testable against the in-memory fakes.

mod cli;
mod commands;
pub mod complete;
pub mod map;
mod output;

pub use cli::{Cli, Command, Globals, OutputFormat};
pub use commands::{Context, Output, run, run_offline};
