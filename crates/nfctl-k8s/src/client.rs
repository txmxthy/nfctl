use std::time::Duration;

use kube::config::KubeConfigOptions;
use kube::{Client, Config};
use nfctl_core::model::Namespace;
use nfctl_core::{Error, Result};

/// How to reach a cluster. Mirrors the CLI's global flags.
#[derive(Debug, Clone, Default)]
pub struct ClientOptions {
    /// kubeconfig context; `None` uses the current one.
    pub context: Option<String>,
    /// Applied to connect/read/write; watches and log streams override read.
    pub request_timeout: Option<Duration>,
}

/// A connected client plus the context's default namespace.
#[derive(Clone)]
pub struct Connected {
    pub client: Client,
    pub default_namespace: Namespace,
}

impl std::fmt::Debug for Connected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connected")
            .field("default_namespace", &self.default_namespace)
            .finish_non_exhaustive()
    }
}

/// Load kubeconfig (or in-cluster config) and connect.
#[tracing::instrument(level = "info", skip_all, fields(context = opts.context.as_deref()))]
pub async fn connect(opts: &ClientOptions) -> Result<Connected> {
    let mut config = match &opts.context {
        Some(ctx) => {
            let kco = KubeConfigOptions {
                context: Some(ctx.clone()),
                ..Default::default()
            };
            Config::from_kubeconfig(&kco)
                .await
                .map_err(Error::cluster)?
        }
        None => Config::infer().await.map_err(Error::cluster)?,
    };
    if let Some(t) = opts.request_timeout {
        config.connect_timeout = Some(t);
        config.read_timeout = Some(t);
        config.write_timeout = Some(t);
    }
    let default_namespace = Namespace::new(config.default_namespace.clone())
        .unwrap_or_else(|_| Namespace::default_ns());
    let client = {
        let _build = tracing::info_span!("client build").entered();
        Client::try_from(config).map_err(Error::cluster)?
    };
    Ok(Connected {
        client,
        default_namespace,
    })
}

/// The context the kubeconfig would use when none is named, for saying which
/// cluster is on screen. `None` when there is no kubeconfig to read, which is
/// the in-cluster case.
#[must_use]
pub fn current_context() -> Option<String> {
    kube::config::Kubeconfig::read().ok()?.current_context
}
