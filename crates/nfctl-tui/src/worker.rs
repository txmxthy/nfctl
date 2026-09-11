//! The only task that touches the cluster. Requests in, replies out.

use std::sync::Arc;

use futures::StreamExt;
use nfctl_core::model::{
    MonoVertexKey, Namespace, PipelineKey, Selector, TaggedLine, Timestamp, VertexName, Workload,
    WorkloadKey, WorkloadKind,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::service::logs::{TailHandle, TailOptions, tail};
use nfctl_core::service::{MonoVertexView, PipelineService, PipelineView};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerMessage {
    LoadWorkloads(Option<Namespace>),
    LoadView(PipelineKey),
    LoadMonoVertex(MonoVertexKey),
    StartLogs(WorkloadKey, Option<VertexName>),
    StopLogs,
}

#[derive(Debug)]
pub enum WorkerReply {
    /// The list, and how long the cluster took to answer.
    Workloads(Result<Vec<Workload>, String>, std::time::Duration),
    /// A pipeline's shape, before its numbers are in.
    Shape(Box<PipelineView>),
    View(Box<Result<PipelineView, String>>),
    MonoVertex(Box<Result<MonoVertexView, String>>),
    Log(TaggedLine),
    LogsFailed(String),
}

pub struct Worker {
    cluster: Arc<dyn ClusterPort>,
    service: PipelineService,
    tx: mpsc::Sender<WorkerReply>,
    logs: Option<(TailHandle, tokio::task::JoinHandle<()>)>,
}

impl Worker {
    pub fn spawn(
        cluster: Arc<dyn ClusterPort>,
        service: PipelineService,
    ) -> (mpsc::Sender<WorkerMessage>, mpsc::Receiver<WorkerReply>) {
        let (req_tx, mut req_rx) = mpsc::channel::<WorkerMessage>(64);
        let (rep_tx, rep_rx) = mpsc::channel::<WorkerReply>(1024);
        let mut w = Worker {
            cluster,
            service,
            tx: rep_tx,
            logs: None,
        };
        tokio::spawn(async move {
            while let Some(msg) = req_rx.recv().await {
                w.handle(msg).await;
            }
            w.stop_logs();
        });
        (req_tx, rep_rx)
    }

    async fn handle(&mut self, msg: WorkerMessage) {
        match msg {
            WorkerMessage::LoadWorkloads(ns) => {
                let started = std::time::Instant::now();
                let r = self
                    .service
                    .list_workloads(ns.as_ref())
                    .await
                    .map_err(|e| e.to_string());
                let _ = self
                    .tx
                    .send(WorkerReply::Workloads(r, started.elapsed()))
                    .await;
            }
            WorkerMessage::LoadMonoVertex(key) => {
                // A MonoVertex has no shape to draw before its numbers: one
                // read of the CRD and two daemon calls are the whole view.
                let r = self
                    .service
                    .monovertex_view(&key, Timestamp::now())
                    .await
                    .map_err(|e| e.to_string());
                let _ = self.tx.send(WorkerReply::MonoVertex(Box::new(r))).await;
            }
            WorkerMessage::LoadView(key) => {
                // The shape is one call to the cluster and the numbers are
                // several to a daemon that may be far away, so the shape is
                // sent as soon as it lands and the panel has a graph to draw
                // while the rest is still coming.
                let now = Timestamp::now();
                let shape = match self.service.shape(&key, now).await {
                    Ok(shape) => shape,
                    Err(e) => {
                        let _ = self
                            .tx
                            .send(WorkerReply::View(Box::new(Err(e.to_string()))))
                            .await;
                        return;
                    }
                };
                let _ = self
                    .tx
                    .send(WorkerReply::Shape(Box::new(shape.clone())))
                    .await;
                let full = self.service.numbers(shape, now).await;
                let _ = self.tx.send(WorkerReply::View(Box::new(Ok(full)))).await;
            }
            WorkerMessage::StartLogs(key, vertex) => {
                self.stop_logs();
                let selector = match key.kind {
                    WorkloadKind::Pipeline => Selector::vertex_pods(&key.name, vertex.as_ref()),
                    WorkloadKind::MonoVertex => Selector::monovertex_pods(&key.name),
                };
                let opts = TailOptions {
                    tail_lines: Some(50),
                    ..TailOptions::default()
                };
                match tail(
                    Arc::clone(&self.cluster),
                    key.namespace.clone(),
                    selector,
                    opts,
                )
                .await
                {
                    Ok((mut lines, handle)) => {
                        let tx = self.tx.clone();
                        let pump = tokio::spawn(async move {
                            while let Some(l) = lines.next().await {
                                if tx.send(WorkerReply::Log(l)).await.is_err() {
                                    break;
                                }
                            }
                        });
                        self.logs = Some((handle, pump));
                    }
                    Err(e) => {
                        let _ = self.tx.send(WorkerReply::LogsFailed(e.to_string())).await;
                    }
                }
            }
            WorkerMessage::StopLogs => self.stop_logs(),
        }
    }

    fn stop_logs(&mut self) {
        if let Some((handle, pump)) = self.logs.take() {
            pump.abort();
            drop(handle);
        }
    }
}
