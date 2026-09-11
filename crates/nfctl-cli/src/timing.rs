//! Where the time goes. `--timings` prints every span's duration as it
//! closes, so any command can be asked which step was slow without a
//! profiler; `NFCTL_LOG` takes over when more than the timings is wanted.
//!
//! Spans live at the boundaries that can block: building a kube client, each
//! cluster call, connecting to a daemon and each daemon call. Pure code is
//! not instrumented, because it has never been the answer.

use std::fs::File;
use std::io::{IsTerminal as _, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

/// The environment variable that overrides what is logged.
pub const LOG_ENV: &str = "NFCTL_LOG";

/// A file the whole process shares. `tracing_subscriber` wants a writer it
/// can make on demand, and a `File` cannot be handed out twice.
#[derive(Clone)]
struct Shared(Arc<Mutex<File>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush()
    }
}

/// Install the subscriber for this process, if anything asked for one, and
/// say where it is writing when that is a file worth naming afterwards. Safe
/// to call once; a second call is ignored, which is what the tests want.
///
/// `owns_terminal` is the one command that draws over the whole screen. Its
/// timings cannot go to stderr while stderr is that same screen: the lines
/// land on top of what it drew. They go to a file instead, whose path the
/// caller prints once the screen is handed back. Redirect stderr and they go
/// there, like every other command's.
pub fn install(timings: bool, owns_terminal: bool) -> Option<PathBuf> {
    let filter = match std::env::var(LOG_ENV) {
        Ok(spec) if !spec.trim().is_empty() => EnvFilter::new(spec),
        // Timings only: the spans themselves, nothing else.
        _ if timings => EnvFilter::new("nfctl_core=info,nfctl_k8s=info,nfctl_daemon=info"),
        _ => return None,
    };
    let to_screen = owns_terminal && std::io::stderr().is_terminal();
    let file = to_screen.then(open_log).flatten();
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        // Elapsed since start, not the wall clock: what matters is when in
        // the run a step happened and how long it took.
        .with_timer(tracing_subscriber::fmt::time::uptime())
        // Only the close event, which is the one carrying `time.busy`.
        .with_span_events(FmtSpan::CLOSE);
    match file {
        Some((path, sink)) => {
            let _ = builder
                .with_writer(move || sink.clone())
                .with_ansi(false)
                .try_init();
            Some(path)
        }
        None if to_screen => None,
        None => {
            let _ = builder
                .with_writer(std::io::stderr)
                .with_ansi(std::io::stderr().is_terminal())
                .try_init();
            None
        }
    }
}

/// A file per run, so two sessions do not write over each other. Beside the
/// completion catalogue, which is somewhere a person can find again, unlike
/// the temporary directory.
fn open_log() -> Option<(PathBuf, Shared)> {
    let dir = crate::paths::cache_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("timings-{}.log", std::process::id()));
    let file = File::create(&path).ok()?;
    Some((path, Shared(Arc::new(Mutex::new(file)))))
}
