//! Numaflow daemon adapter: implements [`nfctl_core::ports::DaemonPort`] over the
//! daemon's grpc-gateway JSON API, reached either through a Kubernetes
//! port-forward to the daemon pod or directly by URL.
//!
//! One HTTP client serves both: a custom connector ([`dialer::Dialer`]) either
//! opens an authenticated port-forward and wraps it in TLS, or dials TCP.
//! Port-forwarded daemon certificates are ephemeral and bypass verification
//! within that transport; direct HTTPS uses the platform trust store.

mod client;
mod connector;
mod dialer;
pub(crate) mod dto;
mod tls;

pub use client::{ClientOptions, HttpDaemonClient};
pub use connector::{DirectConnector, PortForwardConnector};
