use serde::{Deserialize, Serialize};

use super::{IsbName, Namespace};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IsbPhase {
    #[default]
    Unknown,
    Pending,
    Running,
    Failed,
    Deleting,
}

impl IsbPhase {
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "Pending" => IsbPhase::Pending,
            "Running" => IsbPhase::Running,
            "Failed" => IsbPhase::Failed,
            "Deleting" => IsbPhase::Deleting,
            _ => IsbPhase::Unknown,
        }
    }
}

/// An `InterStepBufferService`. `JetStream` is the only backend in current Numaflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsbService {
    /// Older fixtures omit it; they predate cross-namespace listing.
    #[serde(default = "Namespace::default_ns")]
    pub namespace: Namespace,
    pub name: IsbName,
    pub version: String,
    pub replicas: u32,
    pub persistent: bool,
    pub phase: IsbPhase,
    /// `Ready` condition true and phase Running/Deleting.
    pub healthy: bool,
}
