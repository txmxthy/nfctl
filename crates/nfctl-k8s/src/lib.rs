//! Kubernetes adapter for `nfctl`: implements [`nfctl_core::ports::ClusterPort`]
//! with kube-rs. CRD wire shapes live in [`dto`] and are converted into domain
//! types at this boundary (anti-corruption layer).

mod client;
mod cluster;
pub mod dto;

pub use client::{ClientOptions, connect, current_context};
pub use cluster::KubeCluster;
