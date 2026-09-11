use std::sync::{Arc, Mutex};

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
#[derive(Clone)]
pub struct PortForwardConnector {
    client: Client,
    cluster: Arc<dyn ClusterPort>,
    tls: TlsConnector,
    opts: ClientOptions,
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
        }
    }
}

#[async_trait]
impl DaemonConnector for PortForwardConnector {
    #[tracing::instrument(level = "info", skip_all)]
    async fn connect(&self, key: &PipelineKey) -> Result<Box<dyn DaemonPort>> {
        let topology = self
            .cluster
            .get_pipeline(key)
            .await
            .ok()
            .map(|p| p.spec.topology);
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), key.namespace.as_str());
        let dialer = Dialer::PodForward {
            pods,
            selector: Selector::daemon_pods(&key.name),
            tls: self.tls.clone(),
            cached_pod: Arc::new(Mutex::new(None)),
        };
        Ok(Box::new(HttpDaemonClient::new(
            dialer,
            true,
            Flavour::Pipeline,
            key.clone(),
            topology,
            self.opts,
        )))
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn connect_monovertex(&self, key: &MonoVertexKey) -> Result<Box<dyn DaemonPort>> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), key.namespace.as_str());
        let dialer = Dialer::PodForward {
            pods,
            selector: Selector::monovertex_daemon_pods(&key.name),
            tls: self.tls.clone(),
            cached_pod: Arc::new(Mutex::new(None)),
        };
        let pkey = PipelineKey::new(key.namespace.clone(), key.name.clone());
        Ok(Box::new(HttpDaemonClient::new(
            dialer,
            true,
            Flavour::MonoVertex,
            pkey,
            None,
            self.opts,
        )))
    }
}

/// Talks to a daemon at a fixed URL (in-cluster, or an existing forward).
#[derive(Debug, Clone)]
pub struct DirectConnector {
    host: String,
    port: u16,
    secure: bool,
    topology: Option<Topology>,
    opts: ClientOptions,
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
            host,
            port,
            secure,
            topology,
            opts,
        })
    }
}

#[async_trait]
impl DaemonConnector for DirectConnector {
    async fn connect(&self, key: &PipelineKey) -> Result<Box<dyn DaemonPort>> {
        let tls = self
            .secure
            .then(|| TlsConnector::from(crate::tls::insecure_client_config()));
        let dialer = Dialer::Direct {
            host: self.host.clone(),
            port: self.port,
            tls,
        };
        Ok(Box::new(HttpDaemonClient::new(
            dialer,
            self.secure,
            Flavour::Pipeline,
            key.clone(),
            self.topology.clone(),
            self.opts,
        )))
    }

    async fn connect_monovertex(&self, key: &MonoVertexKey) -> Result<Box<dyn DaemonPort>> {
        let tls = self
            .secure
            .then(|| TlsConnector::from(crate::tls::insecure_client_config()));
        let dialer = Dialer::Direct {
            host: self.host.clone(),
            port: self.port,
            tls,
        };
        let pkey = PipelineKey::new(key.namespace.clone(), key.name.clone());
        Ok(Box::new(HttpDaemonClient::new(
            dialer,
            self.secure,
            Flavour::MonoVertex,
            pkey,
            None,
            self.opts,
        )))
    }
}
