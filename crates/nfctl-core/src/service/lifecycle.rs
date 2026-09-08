//! Pause, resume, recycle, wait: the procedures that need more than one call.

use std::time::Duration;

use futures::StreamExt;
use serde::Serialize;

use crate::model::{
    DesiredPhase, Namespace, Pipeline, PipelineKey, PipelinePhase, PodName, ResumeStrategy,
    Selector, Timestamp, VertexName,
};
use crate::ports::{ClusterPort, DaemonConnector};
use crate::{Error, Result};

/// Block until the pipeline reports `phase`, or `timeout` elapses.
pub async fn wait_for_phase(
    cluster: &dyn ClusterPort,
    key: &PipelineKey,
    phase: PipelinePhase,
    timeout: Duration,
) -> Result<Pipeline> {
    let wait = async {
        let mut stream = cluster.watch_pipeline(key).await?;
        while let Some(item) = stream.next().await {
            let p = item?;
            if p.status.phase == phase {
                return Ok(p);
            }
        }
        // Watch ended without reaching the phase: report what we last knew.
        let p = cluster.get_pipeline(key).await?;
        if p.status.phase == phase {
            Ok(p)
        } else {
            Err(Error::Timeout(timeout))
        }
    };
    tokio::time::timeout(timeout, wait)
        .await
        .unwrap_or(Err(Error::Timeout(timeout)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PauseReport {
    pub phase: PipelinePhase,
    /// `Some(true)` when every buffer is empty; `None` when the daemon could not say.
    pub drained: Option<bool>,
    pub drained_on_pause: bool,
    pub dry_run: bool,
}

/// Set desired phase Paused and, with `wait`, follow it to `Paused` and report drain.
pub async fn pause(
    cluster: &dyn ClusterPort,
    daemons: &dyn DaemonConnector,
    key: &PipelineKey,
    wait: Option<Duration>,
    dry_run: bool,
) -> Result<PauseReport> {
    cluster
        .set_lifecycle(key, DesiredPhase::Paused, None, dry_run)
        .await?;
    if dry_run {
        let p = cluster.get_pipeline(key).await?;
        return Ok(PauseReport {
            phase: p.status.phase,
            drained: None,
            drained_on_pause: false,
            dry_run,
        });
    }
    let p = match wait {
        Some(t) => wait_for_phase(cluster, key, PipelinePhase::Paused, t).await?,
        None => cluster.get_pipeline(key).await?,
    };
    let drained = match daemons.connect(key).await {
        Ok(d) => d.buffers().await.ok().filter(|b| !b.is_empty()).map(|b| {
            b.iter()
                .all(|x| x.pending == Some(0) && x.ack_pending == Some(0))
        }),
        Err(_) => None,
    };
    Ok(PauseReport {
        phase: p.status.phase,
        drained,
        drained_on_pause: p.status.drained_on_pause,
        dry_run,
    })
}

/// Set desired phase Running with a resume strategy.
pub async fn resume(
    cluster: &dyn ClusterPort,
    key: &PipelineKey,
    strategy: ResumeStrategy,
    dry_run: bool,
) -> Result<()> {
    cluster
        .set_lifecycle(key, DesiredPhase::Running, Some(strategy), dry_run)
        .await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RecycleReport {
    /// One vertex: its pods were deleted and the controller replaces them.
    Pods {
        vertex: VertexName,
        deleted: Vec<PodName>,
    },
    /// Whole pipeline: paused (drained), then resumed with the fast strategy.
    Pipeline {
        paused_at: Timestamp,
        drained: Option<bool>,
    },
}

/// Restart a vertex (delete its pods) or a whole pipeline (pause, drain, resume).
pub async fn recycle(
    cluster: &dyn ClusterPort,
    daemons: &dyn DaemonConnector,
    key: &PipelineKey,
    vertex: Option<&VertexName>,
    wait: Duration,
    all_at_once: bool,
    dry_run: bool,
) -> Result<RecycleReport> {
    if let Some(v) = vertex {
        let p = cluster.get_pipeline(key).await?;
        if p.spec.topology.vertex(v).is_none() {
            return Err(Error::NotFound {
                kind: "vertex",
                name: format!("{key}/{v}"),
            });
        }
        let selector = Selector::vertex_pods(&key.name, Some(v));
        let deleted = if all_at_once {
            cluster
                .delete_pods(&key.namespace, &selector, dry_run)
                .await?
        } else {
            rolling_restart(cluster, &key.namespace, &selector, wait, dry_run).await?
        };
        return Ok(RecycleReport::Pods {
            vertex: v.clone(),
            deleted,
        });
    }
    let report = pause(cluster, daemons, key, Some(wait), dry_run).await?;
    let paused_at = Timestamp::now();
    resume(cluster, key, ResumeStrategy::Fast, dry_run).await?;
    Ok(RecycleReport::Pipeline {
        paused_at,
        drained: report.drained,
    })
}

const ROLL_POLL: Duration = Duration::from_millis(500);

/// Pods that are up: phase Running with every non-init container running.
fn running(pods: &[crate::model::PodRef]) -> usize {
    pods.iter()
        .filter(|p| p.phase == crate::model::PodPhase::Running)
        .filter(|p| p.containers.iter().all(|c| c.is_init || c.running))
        .count()
}

/// Delete the selected pods one at a time, waiting after each until the
/// selector again matches as many running pods as before the deletion.
/// `wait` bounds each step; a step that times out stops the roll with the pods
/// deleted so far in the error.
async fn rolling_restart(
    cluster: &dyn ClusterPort,
    ns: &Namespace,
    selector: &Selector,
    wait: Duration,
    dry_run: bool,
) -> Result<Vec<PodName>> {
    let mut pods = cluster.list_pods(ns, selector).await?;
    pods.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    let target = running(&pods);
    let mut deleted = Vec::with_capacity(pods.len());
    for pod in &pods {
        cluster.delete_pod(ns, &pod.name, dry_run).await?;
        deleted.push(pod.name.clone());
        if dry_run {
            continue;
        }
        let recovered = async {
            loop {
                let now = cluster.list_pods(ns, selector).await?;
                let gone = now.iter().all(|p| p.name != pod.name);
                if gone && running(&now) >= target {
                    return Ok::<(), Error>(());
                }
                tokio::time::sleep(ROLL_POLL).await;
            }
        };
        tokio::time::timeout(wait, recovered)
            .await
            .unwrap_or(Err(Error::Timeout(wait)))?;
    }
    Ok(deleted)
}

#[cfg(all(test, feature = "fake"))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::fake::{FakeCluster, FakeDaemons, sample_pipeline, sample_pod};
    use crate::model::PodEvent;
    use crate::service::NoDaemon;

    fn setup() -> (FakeCluster, PipelineKey) {
        let p = sample_pipeline("ns", "p", PipelinePhase::Running);
        let key = p.key.clone();
        (FakeCluster::with_pipelines(vec![p]), key)
    }

    #[tokio::test]
    async fn pause_then_resume_round_trip() {
        let (c, key) = setup();
        let r = pause(&c, &NoDaemon, &key, Some(Duration::from_secs(1)), false)
            .await
            .unwrap();
        assert_eq!(r.phase, PipelinePhase::Paused);
        assert_eq!(r.drained, None, "no daemon: unknown");
        assert_eq!(
            c.get_pipeline(&key).await.unwrap().spec.lifecycle.desired,
            DesiredPhase::Paused
        );

        resume(&c, &key, ResumeStrategy::Slow, false).await.unwrap();
        let p = c.get_pipeline(&key).await.unwrap();
        assert_eq!(p.status.phase, PipelinePhase::Running);
        assert_eq!(p.meta.resume_strategy, Some(ResumeStrategy::Slow));
    }

    #[tokio::test]
    async fn dry_run_changes_nothing() {
        let (c, key) = setup();
        let r = pause(&c, &NoDaemon, &key, None, true).await.unwrap();
        assert!(r.dry_run);
        assert_eq!(
            c.get_pipeline(&key).await.unwrap().status.phase,
            PipelinePhase::Running
        );
    }

    #[tokio::test]
    async fn wait_times_out_when_phase_never_arrives() {
        let (c, key) = setup();
        let err = wait_for_phase(&c, &key, PipelinePhase::Failed, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Timeout(_)), "{err}");
    }

    #[tokio::test]
    async fn recycle_vertex_deletes_its_pods_and_rejects_unknown_vertex() {
        let (c, key) = setup();
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        let v = VertexName::new("cat").unwrap();
        let r = recycle(
            &c,
            &NoDaemon,
            &key,
            Some(&v),
            Duration::from_secs(1),
            true,
            false,
        )
        .await
        .unwrap();
        assert!(matches!(r, RecycleReport::Pods { ref deleted, .. } if deleted.len() == 1));
        let ghost = VertexName::new("ghost").unwrap();
        assert!(matches!(
            recycle(
                &c,
                &NoDaemon,
                &key,
                Some(&ghost),
                Duration::from_secs(1),
                true,
                false
            )
            .await
            .unwrap_err(),
            Error::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn recycle_pipeline_pauses_and_resumes_fast() {
        let (c, key) = setup();
        let r = recycle(
            &c,
            &FakeDaemons::default(),
            &key,
            None,
            Duration::from_secs(1),
            false,
            false,
        )
        .await
        .unwrap();
        assert!(matches!(r, RecycleReport::Pipeline { .. }));
        let p = c.get_pipeline(&key).await.unwrap();
        assert_eq!(p.status.phase, PipelinePhase::Running);
        assert_eq!(p.meta.resume_strategy, Some(ResumeStrategy::Fast));
    }

    #[tokio::test]
    async fn recycle_vertex_rolls_one_pod_at_a_time() {
        let (c, key) = setup();
        c.emit(&PodEvent::Applied(sample_pod("p-cat-0-abc", false)));
        c.emit(&PodEvent::Applied(sample_pod("p-cat-1-def", false)));
        let v = VertexName::new("cat").unwrap();
        let r = recycle(
            &c,
            &NoDaemon,
            &key,
            Some(&v),
            Duration::from_secs(2),
            false,
            false,
        )
        .await
        .unwrap();
        let RecycleReport::Pods { deleted, .. } = r else {
            panic!("expected pods");
        };
        assert_eq!(
            deleted.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["p-cat-0-abc", "p-cat-1-def"]
        );
        // Every pod was replaced, none of the originals remain.
        let names: Vec<String> = c
            .list_pods(&key.namespace, &Selector::vertex_pods(&key.name, Some(&v)))
            .await
            .unwrap()
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        assert_eq!(names, ["p-cat-0-abc-r", "p-cat-1-def-r"]);
    }
}
