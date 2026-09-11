//! Where the time goes. `--timings` prints every span's duration as it
//! closes, so any command can be asked which step was slow without a
//! profiler; `NFCTL_LOG` takes over when more than the timings is wanted.
//!
//! Spans live at the boundaries that can block: building a kube client, each
//! cluster call, connecting to a daemon and each daemon call. Pure code is
//! not instrumented, because it has never been the answer.

use std::io::IsTerminal as _;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

/// The environment variable that overrides what is logged.
pub const LOG_ENV: &str = "NFCTL_LOG";

/// Install the subscriber for this process, if anything asked for one. Safe
/// to call once; a second call is ignored, which is what the tests want.
///
/// `owns_terminal` is the one command that draws over the whole screen. It
/// gets no subscriber while stderr is that same screen, because the lines
/// would land on top of what it drew; redirect stderr and it is written
/// there, which is how a TUI session is captured:
///
/// ```text
/// nfctl --timings tui 2> timings.log
/// ```
pub fn install(timings: bool, owns_terminal: bool) {
    if owns_terminal && std::io::stderr().is_terminal() {
        return;
    }
    let filter = match std::env::var(LOG_ENV) {
        Ok(spec) if !spec.trim().is_empty() => EnvFilter::new(spec),
        // Timings only: the spans themselves, nothing else.
        _ if timings => EnvFilter::new("nfctl_core=info,nfctl_k8s=info,nfctl_daemon=info"),
        _ => return,
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false)
        // Elapsed since start, not the wall clock: what matters is when in
        // the run a step happened and how long it took.
        .with_timer(tracing_subscriber::fmt::time::uptime())
        // Only the close event, which is the one carrying `time.busy`.
        .with_span_events(FmtSpan::CLOSE)
        .try_init();
}
