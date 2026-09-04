//! `nfctl` binary entry point.

use clap::Parser;

/// Operate Numaflow pipelines from the terminal.
#[derive(Debug, Parser)]
#[command(name = "nfctl", version, about, long_about = None)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
