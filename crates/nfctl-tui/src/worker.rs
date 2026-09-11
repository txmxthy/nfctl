//! The only task that touches the cluster. Requests in, replies out.

use std::sync::Arc;

use futures::StreamExt;
use nfctl_core::model::{
    Namespace, Pipeline, PipelineKey, Selector, TaggedLine, Timestamp, VertexName,
};
use nfctl_core::ports::ClusterPort;
use nfctl_core::service::logs::{TailHandle, TailOptions, tail};
use nfctl_core::service::{PipelineService, PipelineView};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerMessage {
    LoadPipelines(Option<Namespace>),
    LoadView(PipelineKey),
    StartLogs(PipelineKey, Option<VertexName>),
    StopLogs,
}

#[derive(Debug)]
pub enum WorkerReply {
    /// The list, and how long the cluster took to answer.
    Pipelines(Result<Vec<Pipeline>, String>, std::time::Duration),
    /// A pipeline's shape, before its numbers are in.
    Shape(Box<PipelineView>),
    View(Box<Result<PipelineView, String>>),
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
            WorkerMessage::LoadPipelines(ns) => {
                let started = std::time::Instant::now();
                let r = self
                    .service
                    .list(ns.as_ref())
                    .await
                    .map_err(|e| e.to_string());
                let _ = self
                    .tx
                    .send(WorkerReply::Pipelines(r, started.elapsed()))
                    .await;
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
                let selector = Selector::vertex_pods(&key.name, vertex.as_ref());
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
