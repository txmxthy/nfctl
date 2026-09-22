use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Uri};
use http_body_util::{BodyExt, Empty, LengthLimitError, Limited};
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
    /// Maximum bytes accepted from one daemon response body.
    pub max_response_body: usize,
    /// How long a whole-pipeline metrics answer is reused before asking
    /// again. See `METRICS_TTL`.
    pub metrics_ttl: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            retries: 2,
            max_response_body: 8 * 1024 * 1024,
            metrics_ttl: METRICS_TTL,
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
    #[error("daemon response body exceeds {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("{0}")]
    Decode(String),
}

impl CallError {
    fn retryable(&self) -> bool {
        matches!(self, CallError::Transport(_) | CallError::Timeout(_))
    }
}

/// Which daemon flavour: the URL layout differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    Pipeline,
    /// One daemon per `MonoVertex`; metrics and status paths carry no name.
    MonoVertex,
}

/// Connections kept open. A view asks four questions at once, so it wants
/// four; more only opens more port-forwards, which cost more than they save.
const POOL: usize = 4;

/// The last whole-pipeline metrics answer, with when it was given.
type Recent = Arc<Mutex<Option<(Instant, Vec<VertexMetrics>)>>>;

/// How long a whole-pipeline metrics answer is reused.
///
/// The daemon has no endpoint for every vertex at once, so this is one call a
/// vertex: eleven round trips for an eleven-vertex pipeline, and on a distant
/// cluster that is seconds. Everything else a view needs is one call each, so
/// metrics alone are asked for less often than the rest.
///
/// Five seconds costs a reader nothing. The rates are already averages over
/// one, five and fifteen minutes, so a two-second refresh was redrawing the
/// same numbers; pending is a gauge, and five seconds behind on a queue depth
/// changes no decision anyone makes from this tool.
///
/// Numaflow's UI server does have one endpoint for all of them
/// (`/namespaces/{ns}/pipelines/{p}/vertices/metrics`), but it is a separate
/// deployment that can have its own authentication in front of it, and it
/// only loops over the same per-vertex call inside the cluster. Depending on
/// it would trade this tool's two dependencies, the Kubernetes API and a
/// pipeline's own daemon, for three.
const METRICS_TTL: Duration = Duration::from_secs(5);

/// [`DaemonPort`] over the daemon's JSON API, bound to one pipeline or `MonoVertex`.
#[derive(Clone)]
pub struct HttpDaemonClient {
    flavour: Flavour,
    http: Client<Dialer, Empty<Bytes>>,
    /// `scheme://host` placeholder; the dialer decides the real destination.
    base: String,
    key: PipelineKey,
    /// Buffer names carry no from/to, so the topology supplies them.
    topology: Option<Topology>,
    opts: ClientOptions,
    /// The last whole-pipeline metrics answer, and when it was given.
    metrics: Recent,
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
        flavour: Flavour,
        key: PipelineKey,
        topology: Option<Topology>,
        opts: ClientOptions,
    ) -> Self {
        let http = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_mins(1))
            .pool_max_idle_per_host(POOL)
            .build(dialer);
        let base = if secure {
            "https://daemon".to_owned()
        } else {
            "http://daemon".to_owned()
        };
        Self {
            flavour,
            http,
            base,
            key,
            topology,
            opts,
            metrics: Arc::new(Mutex::new(None)),
        }
    }

    /// The last whole-pipeline metrics answer, while it is inside the window.
    fn recent_metrics(&self) -> Option<Vec<VertexMetrics>> {
        let held = self.metrics.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, got) = held.as_ref()?;
        (at.elapsed() < self.opts.metrics_ttl).then(|| got.clone())
    }

    fn remember_metrics(&self, got: &[VertexMetrics]) {
        *self.metrics.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), got.to_vec()));
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

    #[tracing::instrument(level = "info", skip(self), fields(path))]
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
            let body = Limited::new(resp.into_body(), self.opts.max_response_body)
                .collect()
                .await
                .map_err(|e| {
                    if e.downcast_ref::<LengthLimitError>().is_some() {
                        CallError::BodyTooLarge {
                            limit: self.opts.max_response_body,
                        }
                    } else {
                        CallError::Transport(e.to_string())
                    }
                })?
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
        match self.flavour {
            Flavour::Pipeline => format!("/api/v1/pipelines/{}/{rest}", self.key.name),
            Flavour::MonoVertex => format!("/api/v1/{rest}"),
        }
    }

    fn not_for_monovertex(&self, what: &'static str) -> Result<()> {
        match self.flavour {
            Flavour::Pipeline => Ok(()),
            Flavour::MonoVertex => Err(Error::Daemon(
                format!("{what}: a MonoVertex has no inter-step buffers").into(),
            )),
        }
    }

    /// Resolve a buffer's incoming edges from its name using the topology.
    fn ends_for(&self, buffer: &str) -> (Vec<VertexName>, VertexName) {
        let unknown =
            || VertexName::new("unknown").unwrap_or_else(|_| unreachable!("literal is valid"));
        let Some(t) = &self.topology else {
            return (vec![unknown()], unknown());
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
                let mut sources: Vec<_> = t
                    .edges()
                    .iter()
                    .filter(|e| e.to == to)
                    .map(|e| e.from.clone())
                    .collect();
                if sources.is_empty() {
                    sources.push(unknown());
                }
                (sources, to)
            }
            None => (vec![unknown()], unknown()),
        }
    }
}

#[async_trait]
impl DaemonPort for HttpDaemonClient {
    #[tracing::instrument(level = "info", skip_all)]
    async fn buffers(&self) -> Result<Vec<BufferInfo>> {
        self.not_for_monovertex("buffers")?;
        let d: dto::ListBuffersDto = self.get_json(&self.path("buffers")).await?;
        d.buffers
            .into_iter()
            .map(|b| {
                let (sources, to) = self.ends_for(&b.buffer_name);
                dto::buffer_from(&b, sources, to).map_err(Error::daemon)
            })
            .collect()
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn buffer(&self, name: &BufferName) -> Result<BufferInfo> {
        self.not_for_monovertex("buffer")?;
        let d: dto::GetBufferDto = self
            .get_json(&self.path(&format!("buffers/{name}")))
            .await?;
        let b = d.buffer.ok_or_else(|| Error::NotFound {
            kind: "buffer",
            name: name.to_string(),
        })?;
        let (sources, to) = self.ends_for(&b.buffer_name);
        dto::buffer_from(&b, sources, to).map_err(Error::daemon)
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn vertex_metrics(&self, vertex: Option<&VertexName>) -> Result<Vec<VertexMetrics>> {
        if self.flavour == Flavour::MonoVertex {
            let d: dto::MonoVertexMetricsDto = self.get_json(&self.path("metrics")).await?;
            let m = d.metrics.unwrap_or_default();
            let vm = dto::VertexMetricsDto {
                vertex: if m.mono_vertex.is_empty() {
                    self.key.name.to_string()
                } else {
                    m.mono_vertex
                },
                processing_rates: m.processing_rates,
                pendings: m.pendings,
            };
            return Ok(vec![dto::metrics_from(&vm).map_err(Error::daemon)?]);
        }
        let names: Vec<VertexName> = match (vertex, &self.topology) {
            (Some(v), _) => vec![v.clone()],
            (None, Some(t)) => t.vertices().iter().map(|v| v.name.clone()).collect(),
            (None, None) => {
                return Err(Error::Daemon(
                    "vertex metrics need a vertex name or a topology".into(),
                ));
            }
        };
        // Asked for the whole pipeline, and asked again inside the window:
        // the last answer will do. Only this call has the problem worth
        // avoiding, being one round trip a vertex.
        if vertex.is_none()
            && let Some(recent) = self.recent_metrics()
        {
            return Ok(recent);
        }
        // The daemon has no endpoint for every vertex at once, so it is one
        // call a vertex, in turn. Six at a time was tried and was worse: each
        // wants its own connection, and a connection here is a port-forward,
        // which costs more to open than the round trips it saves.
        let mut out = Vec::new();
        for v in names {
            let d: dto::VertexMetricsListDto = self
                .get_json(&self.path(&format!("vertices/{v}/metrics")))
                .await?;
            for m in d.vertex_metrics {
                out.push(dto::metrics_from(&m).map_err(Error::daemon)?);
            }
        }
        if vertex.is_none() {
            self.remember_metrics(&out);
        }
        Ok(out)
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn watermarks(&self) -> Result<Vec<EdgeWatermark>> {
        self.not_for_monovertex("watermarks")?;
        let d: dto::WatermarksDto = self.get_json(&self.path("watermarks")).await?;
        d.pipeline_watermarks
            .into_iter()
            .map(|w| dto::watermark_from(w).map_err(Error::daemon))
            .collect()
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn health(&self) -> Result<PipelineHealth> {
        let d: dto::GetStatusDto = self.get_json(&self.path("status")).await?;
        Ok(dto::health_from(d.status.unwrap_or_default()))
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn vertex_errors(&self, vertex: &VertexName) -> Result<Vec<ReplicaErrors>> {
        let rest = match self.flavour {
            Flavour::Pipeline => format!("vertices/{vertex}/errors"),
            Flavour::MonoVertex => format!("mono-vertices/{vertex}/errors"),
        };
        let d: dto::GetErrorsDto = self.get_json(&self.path(&rest)).await?;
        Ok(d.errors.into_iter().map(dto::errors_from).collect())
    }
}
