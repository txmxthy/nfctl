//! Runtime data that only the per-pipeline daemon knows.

use serde::{Deserialize, Serialize};

use super::{BufferName, ContainerName, Timestamp, VertexName};

/// A ratio in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Fraction(f64);

impl Fraction {
    /// `None` if outside `0.0..=1.0` or NaN.
    #[must_use]
    pub fn new(v: f64) -> Option<Self> {
        (v.is_finite() && (0.0..=1.0).contains(&v)).then_some(Self(v))
    }

    /// Build from a percentage (`80` → `0.8`).
    #[must_use]
    pub fn from_percent(p: f64) -> Option<Self> {
        Self::new(p / 100.0)
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn percent(self) -> f64 {
        self.0 * 100.0
    }
}

impl TryFrom<f64> for Fraction {
    type Error = String;
    fn try_from(v: f64) -> Result<Self, String> {
        Self::new(v).ok_or_else(|| format!("{v} is not a fraction in 0..=1"))
    }
}

impl From<Fraction> for f64 {
    fn from(f: Fraction) -> f64 {
        f.0
    }
}

/// One inter-step buffer (an edge partition) as seen by the daemon.
/// `None` means the daemon reported "unknown", which is distinct from zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferInfo {
    pub name: BufferName,
    /// Every vertex feeding the target buffer, sorted and deduplicated.
    pub sources: Vec<VertexName>,
    pub to: VertexName,
    pub pending: Option<i64>,
    pub ack_pending: Option<i64>,
    pub total: Option<i64>,
    pub length: Option<i64>,
    pub usage: Option<Fraction>,
    pub usage_limit: Option<Fraction>,
    pub is_full: Option<bool>,
}

/// A value over the daemon's lookback windows.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Windows<T> {
    pub m1: Option<T>,
    pub m5: Option<T>,
    pub m15: Option<T>,
    /// The vertex's configured (or auto-tuned) lookback.
    pub default: Option<T>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VertexMetrics {
    pub vertex: VertexName,
    /// Messages per second.
    pub rate: Windows<f64>,
    pub pending: Windows<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeWatermark {
    pub from: VertexName,
    pub to: VertexName,
    pub enabled: bool,
    /// One per partition; `None` when not yet available.
    pub per_partition: Vec<Option<Timestamp>>,
}

/// Health as computed by the daemon (buffer-usage EWMA with debounce).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Healthy,
    Warning,
    Critical,
    #[default]
    Unknown,
    Inactive,
    Deleting,
    Unhealthy,
}

impl Health {
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "healthy" => Health::Healthy,
            "warning" => Health::Warning,
            "critical" => Health::Critical,
            "inactive" => Health::Inactive,
            "deleting" => Health::Deleting,
            "unhealthy" => Health::Unhealthy,
            _ => Health::Unknown,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Healthy => "healthy",
            Health::Warning => "warning",
            Health::Critical => "critical",
            Health::Unknown => "unknown",
            Health::Inactive => "inactive",
            Health::Deleting => "deleting",
            Health::Unhealthy => "unhealthy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineHealth {
    pub status: Health,
    pub message: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerError {
    pub container: ContainerName,
    pub at: Option<Timestamp>,
    pub code: String,
    pub message: String,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaErrors {
    pub replica: String,
    pub errors: Vec<ContainerError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_bounds() {
        assert!(Fraction::new(0.0).is_some());
        assert!(Fraction::new(1.0).is_some());
        assert!(Fraction::new(1.01).is_none());
        assert!(Fraction::new(-0.1).is_none());
        assert!(Fraction::new(f64::NAN).is_none());
        assert_eq!(Fraction::from_percent(80.0).map(Fraction::get), Some(0.8));
    }
}
