//! Multi-pod log tailing that survives pod churn.
//!
//! A supervisor task watches pods matching a selector and keeps one tail task
//! per `(pod, container)`. Tails are keyed so a deleted pod's tail is aborted,
//! and a tail that ends early (container restart) is reopened from the last
//! timestamp it saw. Output is one merged stream of tagged lines.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio_stream::wrappers::ReceiverStream;

use crate::model::{
    ContainerName, LogLine, LogOptions, Namespace, PodEvent, PodName, PodPhase, PodRef, Selector,
    TaggedLine, Timestamp, labels,
};
use crate::ports::ClusterPort;
use crate::{Error, Result};

/// Which containers of each pod to tail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ContainerSelect {
    /// The main container plus the pod's default (user) container, if any.
    #[default]
    Default,
    Named(Vec<ContainerName>),
    /// Every non-init container.
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailOptions {
    pub containers: ContainerSelect,
    pub since: Option<Timestamp>,
    pub tail_lines: Option<u32>,
    /// Pause before reopening a tail that ended.
    pub resume_delay: Duration,
    /// Output channel capacity; backpressure to the consumer beyond this.
    pub buffer: usize,
}

impl Default for TailOptions {
    fn default() -> Self {
        Self {
            containers: ContainerSelect::Default,
            since: None,
            tail_lines: None,
            resume_delay: Duration::from_secs(1),
            buffer: 1024,
        }
    }
}

/// Pick the containers to tail on a pod.
#[must_use]
pub fn select_containers(pod: &PodRef, sel: &ContainerSelect) -> Vec<ContainerName> {
    let find = |n: &str| {
        pod.containers
            .iter()
            .find(|c| c.name.as_str() == n)
            .map(|c| c.name.clone())
    };
    match sel {
        ContainerSelect::All => pod
            .containers
            .iter()
            .filter(|c| !c.is_init)
            .map(|c| c.name.clone())
            .collect(),
        ContainerSelect::Named(names) => names.iter().filter_map(|n| find(n.as_str())).collect(),
        ContainerSelect::Default => {
            let mut v: Vec<ContainerName> = find(labels::MAIN_CONTAINER).into_iter().collect();
            if let Some(d) = &pod.default_container
                && !v.contains(d)
                && find(d.as_str()).is_some()
            {
                v.push(d.clone());
            }
            v
        }
    }
}

/// Drop this to stop the supervisor and every tail.
#[derive(Debug)]
pub struct TailHandle(tokio::task::JoinHandle<()>);

impl Drop for TailHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

type Key = (PodName, ContainerName);

/// Start following logs for pods matching `selector`. Returns the merged stream
/// and a handle whose drop tears everything down.
pub async fn tail(
    cluster: Arc<dyn ClusterPort>,
    ns: Namespace,
    selector: Selector,
    opts: TailOptions,
) -> Result<(BoxStream<'static, TaggedLine>, TailHandle)> {
    let events = cluster.watch_pods(&ns, &selector).await?;
    let (tx, rx) = mpsc::channel(opts.buffer);
    let sup = Supervisor {
        cluster,
        ns,
        opts,
        tx,
        tails: HashMap::new(),
        set: JoinSet::new(),
        seen: HashSet::new(),
    };
    let handle = tokio::spawn(sup.run(events));
    Ok((Box::pin(ReceiverStream::new(rx)), TailHandle(handle)))
}

/// One-shot: current backlog of every matching pod, no watch, no follow.
pub async fn snapshot(
    cluster: &dyn ClusterPort,
    ns: &Namespace,
    selector: &Selector,
    opts: &TailOptions,
) -> Result<Vec<TaggedLine>> {
    let mut out = Vec::new();
    for pod in cluster.list_pods(ns, selector).await? {
        for container in select_containers(&pod, &opts.containers) {
            let lo = LogOptions {
                follow: false,
                since: opts.since,
                tail_lines: opts.tail_lines,
            };
            let mut s = cluster.tail_logs(ns, &pod.name, &container, &lo).await?;
            while let Some(line) = s.next().await {
                out.push(TaggedLine {
                    pod: pod.name.clone(),
                    container: container.clone(),
                    line: line?,
                });
            }
        }
    }
    Ok(out)
}

struct Supervisor {
    cluster: Arc<dyn ClusterPort>,
    ns: Namespace,
    opts: TailOptions,
    tx: mpsc::Sender<TaggedLine>,
    tails: HashMap<Key, AbortHandle>,
    set: JoinSet<Key>,
    /// Pods seen since the last `Resync`, to garbage-collect after `ResyncDone`.
    seen: HashSet<PodName>,
}

impl Supervisor {
    async fn run(mut self, mut events: BoxStream<'static, Result<PodEvent>>) {
        loop {
            tokio::select! {
                ev = events.next() => match ev {
                    Some(Ok(ev)) => self.on_event(ev),
                    Some(Err(_)) => {} // the adapter's watcher re-lists on its own
                    None => break,
                },
                Some(done) = self.set.join_next(), if !self.set.is_empty() => {
                    if let Ok(key) = done {
                        self.tails.remove(&key);
                    }
                }
                () = self.tx.closed() => break,
            }
        }
        self.set.abort_all();
    }

    fn on_event(&mut self, ev: PodEvent) {
        match ev {
            PodEvent::Applied(pod) => {
                self.seen.insert(pod.name.clone());
                if matches!(pod.phase, PodPhase::Succeeded | PodPhase::Failed) {
                    self.stop_pod(&pod.name);
                    return;
                }
                for c in select_containers(&pod, &self.opts.containers) {
                    let running = pod.containers.iter().any(|s| s.name == c && s.running);
                    let key = (pod.name.clone(), c);
                    if running && !self.tails.contains_key(&key) {
                        self.spawn(key);
                    }
                }
            }
            PodEvent::Deleted(pod) => self.stop_pod(&pod.name),
            PodEvent::Resync => self.seen.clear(),
            PodEvent::ResyncDone => {
                let gone: Vec<PodName> = self
                    .tails
                    .keys()
                    .map(|(p, _)| p.clone())
                    .filter(|p| !self.seen.contains(p))
                    .collect();
                for p in gone {
                    self.stop_pod(&p);
                }
            }
        }
    }

    fn stop_pod(&mut self, pod: &PodName) {
        self.tails.retain(|(p, _), h| {
            if p == pod {
                h.abort();
                false
            } else {
                true
            }
        });
    }

    fn spawn(&mut self, key: Key) {
        let task = TailTask {
            cluster: Arc::clone(&self.cluster),
            ns: self.ns.clone(),
            key: key.clone(),
            since: self.opts.since,
            tail_lines: self.opts.tail_lines,
            resume_delay: self.opts.resume_delay,
            tx: self.tx.clone(),
        };
        let h = self.set.spawn(task.run());
        self.tails.insert(key, h);
    }
}

struct TailTask {
    cluster: Arc<dyn ClusterPort>,
    ns: Namespace,
    key: Key,
    since: Option<Timestamp>,
    tail_lines: Option<u32>,
    resume_delay: Duration,
    tx: mpsc::Sender<TaggedLine>,
}

const MAX_OPEN_FAILURES: u32 = 5;

impl TailTask {
    async fn run(mut self) -> Key {
        let mut last_ts: Option<Timestamp> = None;
        let mut boundary = HashMap::<String, usize>::new();
        let mut failures = 0u32;
        loop {
            let lo = LogOptions {
                follow: true,
                since: self.since,
                tail_lines: self.tail_lines,
            };
            let (pod, container) = &self.key;
            match self.cluster.tail_logs(&self.ns, pod, container, &lo).await {
                Err(e) => {
                    failures += 1;
                    if failures >= MAX_OPEN_FAILURES {
                        let _ = self
                            .tx
                            .send(self.tagged(LogLine {
                                at: None,
                                text: format!("<tail failed: {e}>"),
                            }))
                            .await;
                        return self.key;
                    }
                    tokio::time::sleep(self.resume_delay * failures).await;
                }
                Ok(mut stream) => {
                    failures = 0;
                    let replay_at = last_ts;
                    let mut replay = boundary.clone();
                    while let Some(item) = stream.next().await {
                        let Ok(line) = item else { break };
                        // `since` is inclusive: skip matching entries from the
                        // last timestamp, but keep new entries at that instant.
                        if line.at == replay_at
                            && let Some(count) = replay.get_mut(&line.text)
                            && *count > 0
                        {
                            *count -= 1;
                            continue;
                        }
                        if let Some(at) = line.at {
                            match last_ts {
                                Some(last) if at > last => {
                                    last_ts = Some(at);
                                    boundary.clear();
                                    boundary.insert(line.text.clone(), 1);
                                }
                                Some(last) if at == last => {
                                    *boundary.entry(line.text.clone()).or_default() += 1;
                                }
                                None => {
                                    last_ts = Some(at);
                                    boundary.insert(line.text.clone(), 1);
                                }
                                Some(_) => {}
                            }
                        }
                        if self.tx.send(self.tagged(line)).await.is_err() {
                            return self.key;
                        }
                    }
                    // Stream ended: container restarted or the connection dropped. Resume.
                    self.since = last_ts.or(self.since);
                    self.tail_lines = None;
                    tokio::time::sleep(self.resume_delay).await;
                }
            }
        }
    }

    fn tagged(&self, line: LogLine) -> TaggedLine {
        TaggedLine {
            pod: self.key.0.clone(),
            container: self.key.1.clone(),
            line,
        }
    }
}

impl From<mpsc::error::SendError<TaggedLine>> for Error {
    fn from(_: mpsc::error::SendError<TaggedLine>) -> Self {
        Error::Cluster("log consumer went away".into())
    }
}

#[cfg(all(test, feature = "fake"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;
    use tokio::time::timeout;

    use super::*;
    use crate::fake::{FakeCluster, LogScript, sample_pod};
    use crate::model::PipelineName;

    fn ts(secs: i64) -> Timestamp {
        Timestamp::new(time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(secs))
    }

    fn line(secs: i64, text: &str) -> LogLine {
        LogLine {
            at: Some(ts(secs)),
            text: text.to_owned(),
        }
    }

    fn name<T: TryFrom<&'static str>>(s: &'static str) -> T {
        T::try_from(s).unwrap_or_else(|_| unreachable!())
    }

    async fn start(
        cluster: &FakeCluster,
        opts: TailOptions,
    ) -> (BoxStream<'static, TaggedLine>, TailHandle) {
        let sel = Selector::vertex_pods(&name::<PipelineName>("p"), None);
        tail(Arc::new(cluster.clone()), name("ns"), sel, opts)
            .await
            .unwrap()
    }

    async fn next_text(s: &mut BoxStream<'static, TaggedLine>) -> String {
        let l = timeout(Duration::from_secs(5), s.next())
            .await
            .expect("timed out")
            .expect("stream ended");
        format!("{}/{} {}", l.pod, l.container, l.line.text)
    }

    async fn settle() {
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn spawns_for_running_pods_and_tags_lines() {
        let c = FakeCluster::default();
        c.script(
            &name("p-cat-0-abc"),
            &name("numa"),
            LogScript {
                lines: vec![line(1, "hello")],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        let (mut s, _h) = start(&c, TailOptions::default()).await;
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa hello");
    }

    #[tokio::test]
    async fn default_selection_is_numa_plus_default_container() {
        let c = FakeCluster::default();
        c.emit(&PodEvent::Applied(sample_pod("p-udf-0-abc", true)));
        c.emit(&PodEvent::Applied(sample_pod("p-src-0-abc", false)));
        let (_s, _h) = start(&c, TailOptions::default()).await;
        settle().await;
        let mut keys: Vec<String> = c
            .tail_calls()
            .iter()
            .map(|t| format!("{}/{}", t.pod, t.container))
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            ["p-src-0-abc/numa", "p-udf-0-abc/numa", "p-udf-0-abc/udf"]
        );
    }

    #[tokio::test]
    async fn pending_pods_and_repeat_applies_do_not_spawn() {
        let c = FakeCluster::default();
        let mut pending = sample_pod("p-cat-0-abc", false);
        pending.containers[0].running = false;
        let (_s, _h) = start(&c, TailOptions::default()).await;
        c.emit(&PodEvent::Applied(pending));
        settle().await;
        assert!(
            c.tail_calls().is_empty(),
            "spawned for a non-running container"
        );

        c.script(
            &name("p-cat-0-abc"),
            &name("numa"),
            LogScript {
                lines: vec![],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        settle().await;
        assert_eq!(c.tail_calls().len(), 1, "double spawn");
    }

    #[tokio::test]
    async fn eof_resumes_from_last_timestamp_and_dedupes_boundary() {
        let c = FakeCluster::default();
        let pod: PodName = name("p-cat-0-abc");
        let ctr: ContainerName = name("numa");
        c.script(
            &pod,
            &ctr,
            LogScript {
                lines: vec![line(1, "a"), line(2, "b")],
                hang: false,
            },
        );
        // Server replays the inclusive boundary line; it must be suppressed.
        c.script(
            &pod,
            &ctr,
            LogScript {
                lines: vec![line(2, "b"), line(3, "c")],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        let opts = TailOptions {
            tail_lines: Some(10),
            resume_delay: Duration::from_millis(10),
            ..Default::default()
        };
        let (mut s, _h) = start(&c, opts).await;
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa a");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa b");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa c");
        let calls = c.tail_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].opts.tail_lines, Some(10));
        assert_eq!(calls[1].opts.since, Some(ts(2)));
        assert_eq!(
            calls[1].opts.tail_lines, None,
            "resume must not re-apply tail_lines"
        );
    }

    #[tokio::test]
    async fn one_stream_preserves_distinct_lines_with_the_same_timestamp() {
        let c = FakeCluster::default();
        c.script(
            &name("p-cat-0-abc"),
            &name("numa"),
            LogScript {
                lines: vec![line(1, "first"), line(1, "second")],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));

        let (mut s, _h) = start(&c, TailOptions::default()).await;
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa first");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa second");
    }

    #[tokio::test]
    async fn resume_dedupes_only_matching_boundary_entries() {
        let c = FakeCluster::default();
        let pod: PodName = name("p-cat-0-abc");
        let ctr: ContainerName = name("numa");
        c.script(
            &pod,
            &ctr,
            LogScript {
                lines: vec![
                    line(1, "before"),
                    line(2, "boundary-a"),
                    line(2, "boundary-b"),
                ],
                hang: false,
            },
        );
        c.script(
            &pod,
            &ctr,
            LogScript {
                lines: vec![
                    line(2, "boundary-a"),
                    line(2, "boundary-b"),
                    line(2, "new-at-boundary"),
                    line(3, "after"),
                ],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        let opts = TailOptions {
            resume_delay: Duration::from_millis(10),
            ..Default::default()
        };

        let (mut s, _h) = start(&c, opts).await;
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa before");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa boundary-a");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa boundary-b");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa new-at-boundary");
        assert_eq!(next_text(&mut s).await, "p-cat-0-abc/numa after");
    }

    #[tokio::test]
    async fn deleted_pod_stops_its_tail_and_resync_gc_removes_unseen() {
        let c = FakeCluster::default();
        for p in ["p-a-0-x", "p-b-0-y"] {
            c.script(
                &name(p),
                &name("numa"),
                LogScript {
                    lines: vec![],
                    hang: true,
                },
            );
        }
        c.emit(&PodEvent::Applied(sample_pod("p-a-0-x", false)));
        c.emit(&PodEvent::Applied(sample_pod("p-b-0-y", false)));
        let (mut s, _h) = start(&c, TailOptions::default()).await;
        settle().await;
        assert_eq!(c.tail_calls().len(), 2);

        c.emit(&PodEvent::Deleted(sample_pod("p-a-0-x", false)));
        settle().await;
        // A relist that only lists pod b must not respawn a.
        c.emit(&PodEvent::Resync);
        c.emit(&PodEvent::Applied(sample_pod("p-b-0-y", false)));
        c.emit(&PodEvent::ResyncDone);
        settle().await;
        assert_eq!(
            c.tail_calls().len(),
            2,
            "unexpected respawn after delete/resync"
        );

        // A relist that omits b garbage-collects its tail; b re-appearing later respawns it.
        c.emit(&PodEvent::Resync);
        c.emit(&PodEvent::ResyncDone);
        settle().await;
        c.script(
            &name("p-b-0-y"),
            &name("numa"),
            LogScript {
                lines: vec![line(9, "back")],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-b-0-y", false)));
        assert_eq!(next_text(&mut s).await, "p-b-0-y/numa back");
        assert_eq!(c.tail_calls().len(), 3);
    }

    #[tokio::test]
    async fn dropping_the_handle_stops_the_supervisor() {
        let c = FakeCluster::default();
        c.script(
            &name("p-a-0-x"),
            &name("numa"),
            LogScript {
                lines: vec![line(1, "x")],
                hang: true,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-a-0-x", false)));
        let (mut s, h) = start(&c, TailOptions::default()).await;
        assert_eq!(next_text(&mut s).await, "p-a-0-x/numa x");
        drop(h);
        assert!(
            timeout(Duration::from_secs(1), s.next())
                .await
                .expect("stream should end")
                .is_none()
        );
    }

    #[tokio::test]
    async fn snapshot_reads_backlog_without_follow() {
        let c = FakeCluster::default();
        c.script(
            &name("p-a-0-x"),
            &name("numa"),
            LogScript {
                lines: vec![line(1, "one"), line(2, "two")],
                hang: false,
            },
        );
        c.emit(&PodEvent::Applied(sample_pod("p-a-0-x", false)));
        let sel = Selector::vertex_pods(&name::<PipelineName>("p"), None);
        let lines = snapshot(&c, &name("ns"), &sel, &TailOptions::default())
            .await
            .unwrap();
        assert_eq!(lines.len(), 2);
        assert!(!c.tail_calls()[0].opts.follow);
    }
}
