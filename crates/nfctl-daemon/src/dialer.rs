//! The connector behind the HTTP client: how bytes reach the daemon.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, Portforwarder};
use nfctl_core::model::Selector;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tracing::Instrument as _;

/// Numaflow's daemon port.
pub const DAEMON_PORT: u16 = 4327;

/// Supertrait so a byte stream can be boxed as one trait object.
pub trait AsyncIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncIo for T {}

type BoxIo = Pin<Box<dyn AsyncIo>>;

#[derive(Debug, thiserror::Error)]
pub enum DialError {
    #[error("no running daemon pod matching `{0}`")]
    NoDaemonPod(String),
    #[error("port-forward: {0}")]
    PortForward(#[source] kube::Error),
    #[error("port-forward stream for port {0} was not available")]
    NoStream(u16),
    #[error("tls handshake: {0}")]
    Tls(#[source] io::Error),
    #[error("connect: {0}")]
    Io(#[source] io::Error),
    #[error("{0}")]
    BadUrl(String),
}

enum Io {
    Tls(Box<TlsStream<BoxIo>>),
    Plain(BoxIo),
}

/// One connection. Owns the port-forward session (if any) so it is torn down
/// with the connection.
pub struct Forwarded {
    io: Io,
    forwarder: Option<Portforwarder>,
}

impl Drop for Forwarded {
    fn drop(&mut self) {
        if let Some(pf) = &self.forwarder {
            pf.abort();
        }
    }
}

impl AsyncRead for Forwarded {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.io {
            Io::Tls(s) => Pin::new(s).poll_read(cx, buf),
            Io::Plain(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Forwarded {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.io {
            Io::Tls(s) => Pin::new(s).poll_write(cx, buf),
            Io::Plain(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.io {
            Io::Tls(s) => Pin::new(s).poll_flush(cx),
            Io::Plain(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.io {
            Io::Tls(s) => Pin::new(s).poll_shutdown(cx),
            Io::Plain(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

impl Connection for Forwarded {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

/// Where the daemon is and how to get a byte stream to it.
#[derive(Clone)]
pub enum Dialer {
    /// Resolve the daemon pod by label, port-forward, TLS.
    PodForward {
        pods: Api<Pod>,
        selector: Selector,
        tls: TlsConnector,
        /// Last pod that worked; cleared on failure so a rescheduled daemon is found.
        cached_pod: Arc<Mutex<Option<String>>>,
    },
    /// TCP to `host:port`, TLS when `tls` is set.
    Direct {
        host: String,
        port: u16,
        tls: Option<TlsConnector>,
    },
}

impl std::fmt::Debug for Dialer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dialer::PodForward { selector, .. } => write!(f, "PodForward({selector})"),
            Dialer::Direct { host, port, tls } => {
                write!(f, "Direct({host}:{port}, tls={})", tls.is_some())
            }
        }
    }
}

impl Dialer {
    async fn resolve_pod(
        pods: &Api<Pod>,
        selector: &Selector,
        cache: &Mutex<Option<String>>,
    ) -> Result<String, DialError> {
        if let Some(name) = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Ok(name);
        }
        let sel = selector.to_string();
        let list = pods
            .list(&ListParams::default().labels(&sel))
            .await
            .map_err(DialError::PortForward)?;
        let running = list.items.into_iter().find(|p| {
            p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running")
                && p.metadata.name.is_some()
        });
        let name = running
            .and_then(|p| p.metadata.name)
            .ok_or_else(|| DialError::NoDaemonPod(sel.clone()))?;
        *cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(name.clone());
        Ok(name)
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn dial(self) -> Result<TokioIo<Forwarded>, DialError> {
        match self {
            Dialer::PodForward {
                pods,
                selector,
                tls,
                cached_pod,
            } => {
                let server_name = ServerName::try_from("localhost")
                    .map_err(|e| DialError::BadUrl(e.to_string()))?;
                let name = {
                    let span = tracing::info_span!("resolve daemon pod");
                    Self::resolve_pod(&pods, &selector, &cached_pod)
                        .instrument(span)
                        .await?
                };
                let attempt = async {
                    let mut pf = pods
                        .portforward(&name, &[DAEMON_PORT])
                        .instrument(tracing::info_span!("port-forward"))
                        .await
                        .map_err(DialError::PortForward)?;
                    let raw: BoxIo = Box::pin(
                        pf.take_stream(DAEMON_PORT)
                            .ok_or(DialError::NoStream(DAEMON_PORT))?,
                    );
                    let io = tls
                        .connect(server_name, raw)
                        .instrument(tracing::info_span!("tls handshake"))
                        .await
                        .map_err(DialError::Tls)?;
                    Ok::<_, DialError>(Forwarded {
                        io: Io::Tls(Box::new(io)),
                        forwarder: Some(pf),
                    })
                };
                match attempt.await {
                    Ok(f) => Ok(TokioIo::new(f)),
                    Err(e) => {
                        // Drop the cached pod; the next dial re-resolves.
                        *cached_pod
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                        Err(e)
                    }
                }
            }
            Dialer::Direct { host, port, tls } => {
                let server_name = ServerName::try_from(host.clone())
                    .map_err(|e| DialError::BadUrl(e.to_string()))?;
                let tcp = TcpStream::connect((host.as_str(), port))
                    .await
                    .map_err(DialError::Io)?;
                let raw: BoxIo = Box::pin(tcp);
                let io = match tls {
                    Some(tls) => Io::Tls(Box::new(
                        tls.connect(server_name, raw)
                            .await
                            .map_err(DialError::Tls)?,
                    )),
                    None => Io::Plain(raw),
                };
                Ok(TokioIo::new(Forwarded {
                    io,
                    forwarder: None,
                }))
            }
        }
    }
}

impl tower_service::Service<http::Uri> for Dialer {
    type Response = TokioIo<Forwarded>;
    type Error = DialError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    /// The URI's host is a placeholder; the dialer already knows where to go.
    fn call(&mut self, _uri: http::Uri) -> Self::Future {
        Box::pin(self.clone().dial())
    }
}
