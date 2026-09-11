//! Tab completion of resource names. The shell runs `nfctl` with `COMPLETE=<shell>`
//! and the whole command line after `--`; clap asks the completer attached to
//! the positional for candidates. Names come from the fixture on the line, a
//! short-lived cache, or one bounded call to the cluster. A completer has
//! nowhere to report errors, so every failure degrades to an empty list — and
//! nothing on this path may reach the terminal, not even from a child process
//! the kube client spawns (see `Hushed`, and ADR 0008).

use std::ffi::{OsStr, OsString};
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use clap_complete::CompletionCandidate;
use nfctl_core::model::{IsbService, MonoVertex, Pipeline};
use nfctl_core::ports::ClusterPort as _;
use serde::{Deserialize, Serialize};

/// Cached resource names stay fresh this long.
const CACHE_TTL: Duration = Duration::from_secs(30);
/// Budget for the whole live lookup: connect and three lists.
const LIVE_TIMEOUT: Duration = Duration::from_millis(1500);

// ---- what is on the command line ------------------------------------------

/// The parts of the line being completed that change which names are right.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Line {
    pub context: Option<String>,
    pub namespace: Option<String>,
    pub fixture: Option<PathBuf>,
    /// Bare words after the binary: subcommands and positionals, in order.
    pub words: Vec<String>,
}

/// Flags that take a value, so the value is not mistaken for a positional.
const VALUE_FLAGS: [&str; 10] = [
    "-n",
    "--namespace",
    "--context",
    "--fixture",
    "-o",
    "--output",
    "--request-timeout",
    "--daemon-url",
    "-c",
    "--container",
];

/// Scan the line after `--`. Understands `--flag value`, `--flag=value` and `-nvalue`.
pub fn scan<I: IntoIterator<Item = OsString>>(args: I) -> Line {
    let mut line = Line::default();
    let mut args = args.into_iter().map(|a| a.to_string_lossy().into_owned());
    let _bin = args.next();
    let mut set = |flag: &str, value: String| match flag {
        "-n" | "--namespace" => line.namespace = Some(value),
        "--context" => line.context = Some(value),
        "--fixture" => line.fixture = Some(PathBuf::from(value)),
        _ => {}
    };
    while let Some(arg) = args.next() {
        if let Some((flag, value)) = arg.split_once('=')
            && flag.starts_with('-')
        {
            set(flag, value.to_owned());
        } else if VALUE_FLAGS.contains(&arg.as_str()) {
            if let Some(value) = args.next() {
                set(&arg, value);
            }
        } else if arg.len() > 2
            && let Some(rest) = arg.strip_prefix("-n")
        {
            set("-n", rest.to_owned());
        } else if !arg.starts_with('-') {
            line.words.push(arg);
        }
    }
    line
}

/// The line being completed, with the environment filling what it leaves out.
fn current_line() -> Line {
    let mut args = std::env::args_os();
    let after: Vec<OsString> = args.by_ref().skip_while(|a| a != "--").skip(1).collect();
    let mut line = scan(after);
    if line.context.is_none() {
        line.context = std::env::var("NFCTL_CONTEXT").ok();
    }
    if line.namespace.is_none() {
        line.namespace = std::env::var("NFCTL_NAMESPACE").ok();
    }
    if line.fixture.is_none() {
        line.fixture = std::env::var_os("NFCTL_FIXTURE").map(PathBuf::from);
    }
    line
}

// ---- the catalog -----------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub namespace: String,
    pub name: String,
    pub phase: String,
    #[serde(default)]
    pub vertices: Vec<String>,
}

/// Every name a positional can take, from one source.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
// `try_join!` expands to pinning code the lint counts as unsafe.
#[allow(clippy::unsafe_derive_deserialize)]
pub struct Catalog {
    pub pipelines: Vec<Entry>,
    pub monovertices: Vec<Entry>,
    pub isbs: Vec<Entry>,
}

impl Catalog {
    fn build(pipelines: &[Pipeline], monovertices: &[MonoVertex], isbs: &[IsbService]) -> Self {
        Self {
            pipelines: pipelines
                .iter()
                .map(|p| Entry {
                    namespace: p.key.namespace.to_string(),
                    name: p.key.name.to_string(),
                    phase: p.status.phase.as_str().to_owned(),
                    vertices: p
                        .spec
                        .topology
                        .vertices()
                        .iter()
                        .map(|v| v.name.to_string())
                        .collect(),
                })
                .collect(),
            monovertices: monovertices
                .iter()
                .map(|m| Entry {
                    namespace: m.key.namespace.to_string(),
                    name: m.key.name.to_string(),
                    phase: m.phase.as_str().to_owned(),
                    vertices: Vec::new(),
                })
                .collect(),
            isbs: isbs
                .iter()
                .map(|i| Entry {
                    namespace: i.namespace.to_string(),
                    name: i.name.to_string(),
                    phase: format!("{:?}", i.phase),
                    vertices: Vec::new(),
                })
                .collect(),
        }
    }

    fn from_fixture(f: &nfctl_core::fake::Fixture) -> Self {
        Self::build(&f.pipelines, &f.monovertices, &f.isbs)
    }

    async fn from_cluster(context: Option<String>) -> nfctl_core::Result<Self> {
        let conn = nfctl_k8s::connect(&nfctl_k8s::ClientOptions {
            context,
            request_timeout: Some(LIVE_TIMEOUT),
        })
        .await?;
        let cluster = nfctl_k8s::KubeCluster::new(conn.client);
        let (pipelines, monovertices, isbs) = tokio::try_join!(
            cluster.list_pipelines(None),
            cluster.list_monovertices(None),
            cluster.list_isb(None),
        )?;
        Ok(Self::build(&pipelines, &monovertices, &isbs))
    }
}

/// `true` while a cache written at `modified` is still worth using at `now`.
pub fn fresh(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age < CACHE_TTL)
        || now < modified
}

fn cache_path(context: Option<&str>) -> Option<PathBuf> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    context.unwrap_or("").hash(&mut h);
    Some(crate::paths::cache_dir()?.join(format!("catalog-{:016x}.json", h.finish())))
}

fn read_cache(path: &PathBuf) -> Option<(Catalog, SystemTime)> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some((serde_json::from_str(&text).ok()?, modified))
}

fn write_cache(path: &PathBuf, catalog: &Catalog) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(catalog) {
        let _ = std::fs::write(path, text);
    }
}

// ---- keeping the terminal clean -------------------------------------------

/// Silences this process's stdin and stderr for as long as it is alive.
///
/// A kubeconfig context can authenticate through an exec credential plugin,
/// which kube runs as a child process with our own stdin and stderr when the
/// context does not say `interactiveMode: Never` (kube-client
/// `client::auth::auth_exec`). The child writes straight to the terminal, so a
/// plugin that cannot refresh silently prints its "log in again" text over the
/// prompt line on every Tab. No error handling of ours can catch that: the
/// bytes never pass through this process, and the lookup's timeout only stops
/// us waiting for a child that has already written. Pointing the two inherited
/// descriptors at `/dev/null` is the only place to intercept it, and it also
/// stops an interactive plugin reading the terminal behind the shell's back.
///
/// Scoped to the completion path deliberately: a normal run must keep showing
/// the plugin's message, because that is how the user learns to re-authenticate.
#[cfg(unix)]
#[derive(Debug)]
struct Hushed {
    stdin: Option<std::os::fd::OwnedFd>,
    stderr: Option<std::os::fd::OwnedFd>,
}

#[cfg(unix)]
impl Hushed {
    fn new() -> Self {
        use std::os::fd::AsFd as _;

        let null = || {
            std::fs::File::options()
                .write(true)
                .read(true)
                .open("/dev/null")
        };
        let stdin = std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .filter(|_| null().is_ok_and(|n| rustix::stdio::dup2_stdin(&n).is_ok()));
        let stderr = std::io::stderr()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .filter(|_| null().is_ok_and(|n| rustix::stdio::dup2_stderr(&n).is_ok()));
        Self { stdin, stderr }
    }
}

#[cfg(unix)]
impl Drop for Hushed {
    fn drop(&mut self) {
        if let Some(fd) = self.stdin.take() {
            let _ = rustix::stdio::dup2_stdin(&fd);
        }
        if let Some(fd) = self.stderr.take() {
            let _ = rustix::stdio::dup2_stderr(&fd);
        }
    }
}

/// Other platforms inherit the noise; only unix can re-point a descriptor.
#[cfg(not(unix))]
#[derive(Debug)]
struct Hushed;

#[cfg(not(unix))]
impl Hushed {
    fn new() -> Self {
        Self
    }
}

fn live(context: Option<String>) -> Option<Catalog> {
    // Held for the whole lookup: the credential plugin runs somewhere inside it.
    let _hushed = Hushed::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(async {
        tokio::time::timeout(LIVE_TIMEOUT, Catalog::from_cluster(context))
            .await
            .ok()?
            .ok()
    })
}

/// Fixture on the line, else a fresh cache, else the cluster, else a stale
/// cache, else nothing. Resolved once per process.
fn catalog(line: &Line) -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        if let Some(path) = &line.fixture {
            return std::fs::read_to_string(path)
                .ok()
                .and_then(|t| nfctl_core::fake::Fixture::parse(&t).ok())
                .map(|f| Catalog::from_fixture(&f))
                .unwrap_or_default();
        }
        let path = cache_path(line.context.as_deref());
        let cached = path.as_ref().and_then(read_cache);
        if let Some((c, modified)) = &cached
            && fresh(*modified, SystemTime::now())
        {
            return c.clone();
        }
        if let Some(c) = live(line.context.clone()) {
            if let Some(p) = &path {
                write_cache(p, &c);
            }
            return c;
        }
        cached.map(|(c, _)| c).unwrap_or_default()
    })
}

// ---- completers -------------------------------------------------------------

fn candidates<'a>(
    entries: impl Iterator<Item = &'a Entry>,
    current: &OsStr,
    namespace: Option<&str>,
) -> Vec<CompletionCandidate> {
    let prefix = current.to_string_lossy();
    entries
        .filter(|e| namespace.is_none_or(|ns| e.namespace == ns || e.namespace.is_empty()))
        .filter(|e| e.name.starts_with(prefix.as_ref()))
        .map(|e| {
            let help = if e.namespace.is_empty() {
                e.phase.clone()
            } else {
                format!("{} · {}", e.phase, e.namespace)
            };
            CompletionCandidate::new(&e.name).help(Some(help.into()))
        })
        .collect()
}

pub fn pipelines(current: &OsStr) -> Vec<CompletionCandidate> {
    let line = current_line();
    candidates(
        catalog(&line).pipelines.iter(),
        current,
        line.namespace.as_deref(),
    )
}

pub fn monovertices(current: &OsStr) -> Vec<CompletionCandidate> {
    let line = current_line();
    candidates(
        catalog(&line).monovertices.iter(),
        current,
        line.namespace.as_deref(),
    )
}

pub fn isbs(current: &OsStr) -> Vec<CompletionCandidate> {
    let line = current_line();
    candidates(
        catalog(&line).isbs.iter(),
        current,
        line.namespace.as_deref(),
    )
}

/// Vertices of the pipeline named earlier on the line (`nfctl logs <pipeline> <Tab>`).
pub fn vertices(current: &OsStr) -> Vec<CompletionCandidate> {
    let line = current_line();
    let Some(pipeline) = line.words.get(1) else {
        return Vec::new();
    };
    let prefix = current.to_string_lossy();
    catalog(&line)
        .pipelines
        .iter()
        .filter(|p| &p.name == pipeline)
        .filter(|p| line.namespace.as_deref().is_none_or(|ns| p.namespace == ns))
        .flat_map(|p| p.vertices.iter())
        .filter(|v| v.starts_with(prefix.as_ref()))
        .map(CompletionCandidate::new)
        .collect()
}

/// Namespaces seen on any pipeline or `MonoVertex`.
pub fn namespaces(current: &OsStr) -> Vec<CompletionCandidate> {
    let line = current_line();
    let c = catalog(&line);
    let prefix = current.to_string_lossy();
    let mut seen: Vec<&str> = c
        .pipelines
        .iter()
        .chain(&c.monovertices)
        .map(|e| e.namespace.as_str())
        .filter(|ns| ns.starts_with(prefix.as_ref()))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    seen.into_iter().map(CompletionCandidate::new).collect()
}

/// The shell snippet that registers dynamic completion (what
/// `COMPLETE=<shell> nfctl` prints), for `nfctl completions <shell>`.
pub fn registration(shell: clap_complete::Shell) -> Option<String> {
    let shells = clap_complete::env::Shells::builtins();
    let completer = shells.completer(&shell.to_string())?;
    // The completer is `nfctl` from PATH, not this binary's path, so the
    // snippet can live in dotfiles that move between machines.
    let mut buf = Vec::new();
    completer
        .write_registration("COMPLETE", "nfctl", "nfctl", "nfctl", &mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Line {
        scan(s.split(' ').map(OsString::from))
    }

    #[test]
    fn scanner_finds_flags_and_positionals() {
        let l = line("nfctl -n demo --context=prod logs linear cat");
        assert_eq!(l.namespace.as_deref(), Some("demo"));
        assert_eq!(l.context.as_deref(), Some("prod"));
        assert_eq!(l.words, ["logs", "linear", "cat"]);
        let l = line("nfctl -ndemo get f");
        assert_eq!(l.namespace.as_deref(), Some("demo"));
        assert_eq!(l.words, ["get", "f"]);
    }

    #[test]
    fn scanner_skips_flag_values() {
        let l = line("nfctl logs linear -c udf --fixture demo.yaml cat");
        assert_eq!(l.words, ["logs", "linear", "cat"]);
        assert_eq!(
            l.fixture.as_deref(),
            Some(std::path::Path::new("demo.yaml"))
        );
    }

    #[test]
    fn candidates_filter_by_prefix_and_namespace() {
        let entries = [
            Entry {
                namespace: "a".into(),
                name: "fanout".into(),
                phase: "Running".into(),
                vertices: vec![],
            },
            Entry {
                namespace: "b".into(),
                name: "fast".into(),
                phase: "Paused".into(),
                vertices: vec![],
            },
            Entry {
                namespace: "a".into(),
                name: "linear".into(),
                phase: "Running".into(),
                vertices: vec![],
            },
        ];
        let names = |ns: Option<&str>, cur: &str| -> Vec<String> {
            candidates(entries.iter(), OsStr::new(cur), ns)
                .iter()
                .map(|c| c.get_value().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(names(None, "fa"), ["fanout", "fast"]);
        assert_eq!(names(Some("a"), "fa"), ["fanout"]);
        assert_eq!(names(None, "z"), Vec::<String>::new());
        let help = candidates(entries.iter(), OsStr::new("fan"), None)
            .remove(0)
            .get_help()
            .map(ToString::to_string);
        assert_eq!(help.as_deref(), Some("Running · a"));
    }

    /// Points stderr at a file, checks `Hushed` swallows writes while alive and
    /// that the descriptor is the original one again after it drops.
    #[cfg(unix)]
    #[test]
    fn hushed_restores_stderr_on_drop() {
        use std::io::Write as _;
        use std::os::fd::AsFd as _;

        // Writing to the handle, not `eprint!`: libtest captures the macro
        // above the descriptor, which is the layer under test.
        let write = |s: &str| std::io::stderr().write_all(s.as_bytes()).unwrap();
        let path = std::env::temp_dir().join(format!("nfctl-hushed-{}.log", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let saved = std::io::stderr().as_fd().try_clone_to_owned().unwrap();
        rustix::stdio::dup2_stderr(&file).unwrap();
        {
            let _hushed = Hushed::new();
            write("during-the-lookup");
        }
        write("after-the-lookup");
        rustix::stdio::dup2_stderr(&saved).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(
            !text.contains("during-the-lookup"),
            "stderr was not silenced"
        );
        assert!(text.contains("after-the-lookup"), "stderr was not restored");
    }

    #[test]
    fn cache_freshness_uses_the_ttl() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        assert!(fresh(t0, t0 + Duration::from_secs(29)));
        assert!(!fresh(t0, t0 + Duration::from_secs(31)));
        assert!(
            fresh(t0, t0 - Duration::from_secs(5)),
            "clock went backwards"
        );
    }
}
