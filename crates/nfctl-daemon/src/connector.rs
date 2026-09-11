use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::Api;
use nfctl_core::model::{MonoVertexKey, PipelineKey, Selector, Topology};
use nfctl_core::ports::{ClusterPort, DaemonConnector, DaemonPort};
use nfctl_core::{Error, Result};
use tokio_rustls::TlsConnector;

use crate::client::{ClientOptions, Flavour, HttpDaemonClient};
use crate::dialer::{DAEMON_PORT, Dialer};

/// Reaches each pipeline's daemon through a port-forward to its pod.
///
/// One client per pipeline, kept. Building it is the expensive part of a
/// refresh: finding the daemon pod, opening the port-forward and shaking
/// hands with a self-signed certificate cost more than every question that
/// follows. The client holds a connection pool for a minute, and its dialer
/// drops a pod that stops answering, so a kept client re-dials by itself
/// rather than going stale.
#[derive(Clone)]
pub struct PortForwardConnector {
    client: Client,
    cluster: Arc<dyn ClusterPort>,
    tls: TlsConnector,
    opts: ClientOptions,
    kept: Arc<Mutex<HashMap<Kept, Arc<dyn DaemonPort>>>>,
}

/// What a kept client answers for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Kept {
    Pipeline(PipelineKey),
    MonoVertex(MonoVertexKey),
}

impl std::fmt::Debug for PortForwardConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PortForwardConnector")
    }
}

impl PortForwardConnector {
    /// `cluster` is used to fetch the topology so buffers can be attributed to edges.
    #[must_use]
    pub fn new(client: Client, cluster: Arc<dyn ClusterPort>, opts: ClientOptions) -> Self {
        Self {
            client,
            cluster,
            tls: TlsConnector::from(crate::tls::insecure_client_config()),
            opts,
            kept: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn kept(&self, at: &Kept) -> Option<Arc<dyn DaemonPort>> {
        kept_in(&self.kept, at)
    }

    fn keep(&self, at: Kept, port: &Arc<dyn DaemonPort>) {
        keep_in(&self.kept, at, port);
    }
}

type Keep = Arc<Mutex<HashMap<Kept, Arc<dyn DaemonPort>>>>;

fn kept_in(keep: &Keep, at: &Kept) -> Option<Arc<dyn DaemonPort>> {
    keep.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(at)
        .map(Arc::clone)
}

fn keep_in(keep: &Keep, at: Kept, port: &Arc<dyn DaemonPort>) {
    keep.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(at, Arc::clone(port));
}

#[async_trait]
impl DaemonConnector for PortForwardConnector {
    #[tracing::instrument(level = "info", skip_all)]
    async fn connect(
        &self,
        key: &PipelineKey,
        topology: Option<&Topology>,
    ) -> Result<Arc<dyn DaemonPort>> {
        let at = Kept::Pipeline(key.clone());
        if let Some(kept) = self.kept(&at) {
            return Ok(kept);
        }
        // Only read the pipeline when the caller has not already got it.
        let topology = match topology {
            Some(t) => Some(t.clone()),
            None => self
                .cluster
                .get_pipeline(key)
                .await
                .ok()
                .map(|p| p.spec.topology),
        };
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), key.namespace.as_str());
        let dialer = Dialer::PodForward {
            pods,
            selector: Selector::daemon_pods(&key.name),
            tls: self.tls.clone(),
            cached_pod: Arc::new(Mutex::new(None)),
        };
        let port: Arc<dyn DaemonPort> = Arc::new(HttpDaemonClient::new(
            dialer,
            true,
            Flavour::Pipeline,
            key.clone(),
            topology,
            self.opts,
        ));
        self.keep(at, &port);
        Ok(port)
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn connect_monovertex(&self, key: &MonoVertexKey) -> Result<Arc<dyn DaemonPort>> {
        let at = Kept::MonoVertex(key.clone());
        if let Some(kept) = self.kept(&at) {
            return Ok(kept);
        }
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), key.namespace.as_str());
        let dialer = Dialer::PodForward {
            pods,
            selector: Selector::monovertex_daemon_pods(&key.name),
            tls: self.tls.clone(),
            cached_pod: Arc::new(Mutex::new(None)),
        };
        let pkey = PipelineKey::new(key.namespace.clone(), key.name.clone());
        let port: Arc<dyn DaemonPort> = Arc::new(HttpDaemonClient::new(
            dialer,
            true,
            Flavour::MonoVertex,
            pkey,
            None,
            self.opts,
        ));
        self.keep(at, &port);
        Ok(port)
    }
}

/// Talks to a daemon at a fixed URL (in-cluster, or an existing forward).
///
/// Keeps one client per pipeline for the same reason the port-forwarding
/// connector does: the client holds the connection pool, so a kept one saves
/// the handshake on every refresh.
#[derive(Clone)]
pub struct DirectConnector {
    host: String,
    port: u16,
    secure: bool,
    topology: Option<Topology>,
    opts: ClientOptions,
    kept: Arc<Mutex<HashMap<Kept, Arc<dyn DaemonPort>>>>,
}

impl DirectConnector {
    /// `url` is `http://host[:port]` or `https://host[:port]`.
    pub fn new(url: &str, topology: Option<Topology>, opts: ClientOptions) -> Result<Self> {
        let uri: http::Uri = url
            .parse()
            .map_err(|e| Error::Usage(format!("invalid daemon url `{url}`: {e}")))?;
        let secure = match uri.scheme_str() {
            Some("https") => true,
            Some("http") => false,
            _ => {
                return Err(Error::Usage(format!(
                    "daemon url `{url}` must start with http:// or https://"
                )));
            }
        };
        let host = uri
            .host()
            .ok_or_else(|| Error::Usage(format!("daemon url `{url}` has no host")))?
            .to_owned();
        let port = uri.port_u16().unwrap_or(DAEMON_PORT);
        Ok(Self {
            kept: Arc::new(Mutex::new(HashMap::new())),
            host,
            port,
            secure,
            topology,
            opts,
        })
    }
}

impl std::fmt::Debug for DirectConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectConnector")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("secure", &self.secure)
            .finish_non_exhaustive()
    }
}

impl DirectConnector {
    fn kept(&self, at: &Kept) -> Option<Arc<dyn DaemonPort>> {
        kept_in(&self.kept, at)
    }

    fn keep(&self, at: Kept, port: &Arc<dyn DaemonPort>) {
        keep_in(&self.kept, at, port);
    }
}

#[async_trait]
impl DaemonConnector for DirectConnector {
    async fn connect(
        &self,
        key: &PipelineKey,
        topology: Option<&Topology>,
    ) -> Result<Arc<dyn DaemonPort>> {
        let at = Kept::Pipeline(key.clone());
        if let Some(kept) = self.kept(&at) {
            return Ok(kept);
        }
        let tls = self
            .secure
            .then(|| TlsConnector::from(crate::tls::insecure_client_config()));
        let dialer = Dialer::Direct {
            host: self.host.clone(),
            port: self.port,
            tls,
        };
        let port: Arc<dyn DaemonPort> = Arc::new(HttpDaemonClient::new(
            dialer,
            self.secure,
            Flavour::Pipeline,
            key.clone(),
            topology.cloned().or_else(|| self.topology.clone()),
            self.opts,
        ));
        self.keep(at, &port);
        Ok(port)
    }

    async fn connect_monovertex(&self, key: &MonoVertexKey) -> Result<Arc<dyn DaemonPort>> {
        let at = Kept::MonoVertex(key.clone());
        if let Some(kept) = self.kept(&at) {
            return Ok(kept);
        }
        let tls = self
            .secure
            .then(|| TlsConnector::from(crate::tls::insecure_client_config()));
        let dialer = Dialer::Direct {
            host: self.host.clone(),
            port: self.port,
            tls,
        };
        let pkey = PipelineKey::new(key.namespace.clone(), key.name.clone());
        let port: Arc<dyn DaemonPort> = Arc::new(HttpDaemonClient::new(
            dialer,
            self.secure,
            Flavour::MonoVertex,
            pkey,
            None,
            self.opts,
        ));
        self.keep(at, &port);
        Ok(port)
    }
}
