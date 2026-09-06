use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Uri};
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use nfctl_core::model::{
    BufferInfo, BufferName, EdgeWatermark, PipelineHealth, PipelineKey, ReplicaErrors, Topology,
    VertexMetrics, VertexName,
};
use nfctl_core::ports::DaemonPort;
use nfctl_core::{Error, Result};
use serde::de::DeserializeOwned;

use crate::dialer::Dialer;
use crate::dto;

/// Timeouts and retry budget for daemon calls.
#[derive(Debug, Clone, Copy)]
pub struct ClientOptions {
    /// Per request, including connect. Numaflow's own client uses 1s; a laptop
    /// port-forward needs more.
    pub timeout: Duration,
    /// Extra attempts on transport failure or timeout (never on HTTP errors).
    pub retries: u8,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            retries: 2,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum CallError {
    #[error("{0}")]
    Transport(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("daemon returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("{0}")]
    Decode(String),
}

impl CallError {
    fn retryable(&self) -> bool {
        matches!(self, CallError::Transport(_) | CallError::Timeout(_))
    }
}

/// [`DaemonPort`] over the daemon's JSON API, bound to one pipeline.
#[derive(Clone)]
pub struct HttpDaemonClient {
    http: Client<Dialer, Empty<Bytes>>,
    /// `scheme://host` placeholder; the dialer decides the real destination.
    base: String,
    key: PipelineKey,
    /// Buffer names carry no from/to, so the topology supplies them.
    topology: Option<Topology>,
    opts: ClientOptions,
}

impl std::fmt::Debug for HttpDaemonClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDaemonClient")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl HttpDaemonClient {
    pub(crate) fn new(
        dialer: Dialer,
        secure: bool,
        key: PipelineKey,
        topology: Option<Topology>,
        opts: ClientOptions,
    ) -> Self {
        let http = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_mins(1))
            .pool_max_idle_per_host(1)
            .build(dialer);
        let base = if secure {
            "https://daemon".to_owned()
        } else {
            "http://daemon".to_owned()
        };
        Self {
            http,
            base,
            key,
            topology,
            opts,
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let mut attempt = 0u8;
        loop {
            match self.get_once(path).await {
                Ok(v) => return Ok(v),
                Err(e) if e.retryable() && attempt < self.opts.retries => {
                    attempt += 1;
                    tracing::debug!(path, attempt, error = %e, "retrying daemon call");
                }
                Err(e) => return Err(Error::daemon(e)),
            }
        }
    }

    async fn get_once<T: DeserializeOwned>(&self, path: &str) -> std::result::Result<T, CallError> {
        let uri: Uri = format!("{}{}", self.base, path)
            .parse()
            .map_err(|e| CallError::Transport(format!("{e}")))?;
        let req = Request::get(uri)
            .body(Empty::new())
            .map_err(|e| CallError::Transport(e.to_string()))?;
        let fut = async {
            let resp = self
                .http
                .request(req)
                .await
                .map_err(|e| CallError::Transport(e.to_string()))?;
            let status = resp.status();
            let body = resp
                .into_body()
                .collect()
                .await
                .map_err(|e| CallError::Transport(e.to_string()))?
                .to_bytes();
            if !status.is_success() {
                let message = serde_json::from_slice::<dto::GatewayErrorDto>(&body).map_or_else(
                    |_| String::from_utf8_lossy(&body).into_owned(),
                    |g| g.message,
                );
                return Err(CallError::Http {
                    status: status.as_u16(),
                    message,
                });
            }
            serde_json::from_slice(&body).map_err(|e| CallError::Decode(e.to_string()))
        };
        tokio::time::timeout(self.opts.timeout, fut)
            .await
            .unwrap_or(Err(CallError::Timeout(self.opts.timeout)))
    }

    fn path(&self, rest: &str) -> String {
        format!("/api/v1/pipelines/{}/{rest}", self.key.name)
    }

    /// Resolve a buffer's edge from its name using the topology.
    fn edge_for(&self, buffer: &str) -> (VertexName, VertexName) {
        let unknown =
            || VertexName::new("unknown").unwrap_or_else(|_| unreachable!("literal is valid"));
        let Some(t) = &self.topology else {
            return (unknown(), unknown());
        };
        // `<isb>-<pipeline>-<to>-<partition>`; the `to` vertex owns the buffer.
        let stem = buffer.rsplit_once('-').map_or(buffer, |(s, _)| s);
        let to = t
            .vertices()
            .iter()
            .map(|v| &v.name)
            .filter(|v| stem.ends_with(v.as_str()))
            .max_by_key(|v| v.as_str().len())
            .cloned();
        match to {
            Some(to) => {
                let from = t
                    .edges()
                    .iter()
                    .find(|e| e.to == to)
                    .map_or_else(unknown, |e| e.from.clone());
                (from, to)
            }
            None => (unknown(), unknown()),
        }
    }
}

#[async_trait]
impl DaemonPort for HttpDaemonClient {
    async fn buffers(&self) -> Result<Vec<BufferInfo>> {
        let d: dto::ListBuffersDto = self.get_json(&self.path("buffers")).await?;
        d.buffers
            .into_iter()
            .map(|b| {
                let (from, to) = self.edge_for(&b.buffer_name);
                dto::buffer_from(&b, from, to).map_err(Error::daemon)
            })
            .collect()
    }

    async fn buffer(&self, name: &BufferName) -> Result<BufferInfo> {
        let d: dto::GetBufferDto = self
            .get_json(&self.path(&format!("buffers/{name}")))
            .await?;
        let b = d.buffer.ok_or_else(|| Error::NotFound {
            kind: "buffer",
            name: name.to_string(),
        })?;
        let (from, to) = self.edge_for(&b.buffer_name);
        dto::buffer_from(&b, from, to).map_err(Error::daemon)
    }

    async fn vertex_metrics(&self, vertex: Option<&VertexName>) -> Result<Vec<VertexMetrics>> {
        let names: Vec<VertexName> = match (vertex, &self.topology) {
            (Some(v), _) => vec![v.clone()],
            (None, Some(t)) => t.vertices().iter().map(|v| v.name.clone()).collect(),
            (None, None) => {
                return Err(Error::Daemon(
                    "vertex metrics need a vertex name or a topology".into(),
                ));
            }
        };
        let mut out = Vec::new();
        for v in names {
            let d: dto::VertexMetricsListDto = self
                .get_json(&self.path(&format!("vertices/{v}/metrics")))
                .await?;
            for m in d.vertex_metrics {
                out.push(dto::metrics_from(&m).map_err(Error::daemon)?);
            }
        }
        Ok(out)
    }

    async fn watermarks(&self) -> Result<Vec<EdgeWatermark>> {
        let d: dto::WatermarksDto = self.get_json(&self.path("watermarks")).await?;
        d.pipeline_watermarks
            .into_iter()
            .map(|w| dto::watermark_from(w).map_err(Error::daemon))
            .collect()
    }

    async fn health(&self) -> Result<PipelineHealth> {
        let d: dto::GetStatusDto = self.get_json(&self.path("status")).await?;
        Ok(dto::health_from(d.status.unwrap_or_default()))
    }

    async fn vertex_errors(&self, vertex: &VertexName) -> Result<Vec<ReplicaErrors>> {
        let d: dto::GetErrorsDto = self
            .get_json(&self.path(&format!("vertices/{vertex}/errors")))
            .await?;
        Ok(d.errors.into_iter().map(dto::errors_from).collect())
    }
}
