//! `grpc-gateway` (`ProtoJSON`) wire shapes. Rules that shape these types:
//! 64-bit integers arrive as decimal strings, doubles as numbers (possibly
//! `"NaN"`), unset wrappers are absent, and empty containers are omitted.
//! Sentinels: `-1` watermark = not yet available; a negative rate = unknown.

use std::collections::HashMap;

use nfctl_core::model::{
    BufferInfo, BufferName, ContainerError, ContainerName, EdgeWatermark, Fraction, Health,
    PipelineHealth, ReplicaErrors, Timestamp, VertexMetrics, VertexName, Windows,
};
use serde::{Deserialize, Deserializer};

/// `Int64Value`: string, number or null.
fn de_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        N(i64),
        // A float here is the daemon's i64::MIN sentinel rendered as f64: unknown.
        F(serde::de::IgnoredAny),
        Null,
    }
    Ok(match Option::<Raw>::deserialize(d)? {
        Some(Raw::N(n)) => Some(n),
        Some(Raw::S(s)) => s.parse().ok(),
        None | Some(Raw::Null | Raw::F(_)) => None,
    })
}

/// `DoubleValue`: number, `"NaN"`/`"Infinity"` strings, or null.
fn de_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        N(f64),
        S(String),
        Null,
    }
    Ok(match Option::<Raw>::deserialize(d)? {
        None | Some(Raw::Null) => None,
        Some(Raw::N(n)) => Some(n),
        Some(Raw::S(s)) => s.parse().ok(),
    })
}

fn de_i64_map<'de, D: Deserializer<'de>>(d: D) -> Result<HashMap<String, Option<i64>>, D::Error> {
    #[derive(Deserialize)]
    struct W(#[serde(deserialize_with = "de_i64")] Option<i64>);
    let m: HashMap<String, W> = HashMap::deserialize(d)?;
    Ok(m.into_iter().map(|(k, v)| (k, v.0)).collect())
}

fn de_f64_map<'de, D: Deserializer<'de>>(d: D) -> Result<HashMap<String, Option<f64>>, D::Error> {
    #[derive(Deserialize)]
    struct W(#[serde(deserialize_with = "de_f64")] Option<f64>);
    let m: HashMap<String, W> = HashMap::deserialize(d)?;
    Ok(m.into_iter().map(|(k, v)| (k, v.0)).collect())
}

fn de_i64_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Option<i64>>, D::Error> {
    #[derive(Deserialize)]
    struct W(#[serde(deserialize_with = "de_i64")] Option<i64>);
    let v: Vec<W> = Vec::deserialize(d)?;
    Ok(v.into_iter().map(|w| w.0).collect())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BufferInfoDto {
    #[serde(default)]
    pub buffer_name: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub pending_count: Option<i64>,
    #[serde(default, deserialize_with = "de_i64")]
    pub ack_pending_count: Option<i64>,
    #[serde(default, deserialize_with = "de_i64")]
    pub total_messages: Option<i64>,
    #[serde(default, deserialize_with = "de_i64")]
    pub buffer_length: Option<i64>,
    #[serde(default, deserialize_with = "de_f64")]
    pub buffer_usage_limit: Option<f64>,
    #[serde(default, deserialize_with = "de_f64")]
    pub buffer_usage: Option<f64>,
    #[serde(default)]
    pub is_full: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListBuffersDto {
    #[serde(default)]
    pub buffers: Vec<BufferInfoDto>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct GetBufferDto {
    pub buffer: Option<BufferInfoDto>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VertexMetricsDto {
    #[serde(default)]
    pub vertex: String,
    #[serde(default, deserialize_with = "de_f64_map")]
    pub processing_rates: HashMap<String, Option<f64>>,
    #[serde(default, deserialize_with = "de_i64_map")]
    pub pendings: HashMap<String, Option<i64>>,
}

/// The `MonoVertex` daemon's metrics payload: same maps, different envelope.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MonoVertexMetricsInnerDto {
    #[serde(default)]
    pub mono_vertex: String,
    #[serde(default, deserialize_with = "de_f64_map")]
    pub processing_rates: HashMap<String, Option<f64>>,
    #[serde(default, deserialize_with = "de_i64_map")]
    pub pendings: HashMap<String, Option<i64>>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct MonoVertexMetricsDto {
    pub metrics: Option<MonoVertexMetricsInnerDto>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VertexMetricsListDto {
    #[serde(default)]
    pub vertex_metrics: Vec<VertexMetricsDto>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EdgeWatermarkDto {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default, deserialize_with = "de_i64_vec")]
    pub watermarks: Vec<Option<i64>>,
    #[serde(default)]
    pub is_watermark_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WatermarksDto {
    #[serde(default)]
    pub pipeline_watermarks: Vec<EdgeWatermarkDto>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct StatusDto {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub code: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct GetStatusDto {
    pub status: Option<StatusDto>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ContainerErrorDto {
    #[serde(default)]
    pub container: String,
    pub timestamp: Option<String>,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub details: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplicaErrorsDto {
    #[serde(default)]
    pub replica: String,
    #[serde(default)]
    pub container_errors: Vec<ContainerErrorDto>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct GetErrorsDto {
    #[serde(default)]
    pub errors: Vec<ReplicaErrorsDto>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct GatewayErrorDto {
    #[serde(default)]
    pub message: String,
}

// ---- conversions into the domain -------------------------------------------

/// Why a daemon payload cannot become a domain value.
#[derive(Debug, thiserror::Error)]
#[error("daemon payload: {0}")]
pub(crate) struct Malformed(pub String);

fn nonneg(v: Option<i64>) -> Option<i64> {
    v.filter(|n| *n >= 0)
}

fn windows_i64(m: &HashMap<String, Option<i64>>) -> Windows<i64> {
    let g = |k: &str| m.get(k).copied().flatten().and_then(|v| nonneg(Some(v)));
    Windows {
        m1: g("1m"),
        m5: g("5m"),
        m15: g("15m"),
        default: g("default"),
    }
}

fn windows_f64(m: &HashMap<String, Option<f64>>) -> Windows<f64> {
    let g = |k: &str| {
        m.get(k)
            .copied()
            .flatten()
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    Windows {
        m1: g("1m"),
        m5: g("5m"),
        m15: g("15m"),
        default: g("default"),
    }
}

/// Buffer names are `<isb>-<pipeline>-<vertex>-<partition>`; the daemon does not
/// send from/to, so the caller supplies them from the topology when it can.
pub(crate) fn buffer_from(
    d: &BufferInfoDto,
    from: VertexName,
    to: VertexName,
) -> Result<BufferInfo, Malformed> {
    Ok(BufferInfo {
        name: BufferName::new(&d.buffer_name)
            .map_err(|e| Malformed(format!("buffer `{}`: {e}", d.buffer_name)))?,
        from,
        to,
        pending: nonneg(d.pending_count),
        ack_pending: nonneg(d.ack_pending_count),
        total: nonneg(d.total_messages),
        length: nonneg(d.buffer_length),
        usage: d.buffer_usage.and_then(Fraction::new),
        usage_limit: d.buffer_usage_limit.and_then(Fraction::new),
        is_full: d.is_full,
    })
}

pub(crate) fn metrics_from(d: &VertexMetricsDto) -> Result<VertexMetrics, Malformed> {
    Ok(VertexMetrics {
        vertex: VertexName::new(&d.vertex)
            .map_err(|e| Malformed(format!("vertex `{}`: {e}", d.vertex)))?,
        rate: windows_f64(&d.processing_rates),
        pending: windows_i64(&d.pendings),
    })
}

pub(crate) fn watermark_from(d: EdgeWatermarkDto) -> Result<EdgeWatermark, Malformed> {
    let name = |s: &str| VertexName::new(s).map_err(|e| Malformed(format!("vertex `{s}`: {e}")));
    Ok(EdgeWatermark {
        from: name(&d.from)?,
        to: name(&d.to)?,
        enabled: d.is_watermark_enabled.unwrap_or(false),
        per_partition: d
            .watermarks
            .into_iter()
            .map(|w| {
                w.filter(|ms| *ms >= 0)
                    .and_then(|ms| Timestamp::from_unix_nanos(i128::from(ms) * 1_000_000))
            })
            .collect(),
    })
}

pub(crate) fn health_from(d: StatusDto) -> PipelineHealth {
    PipelineHealth {
        status: Health::parse_lenient(&d.status),
        message: d.message,
        code: d.code,
    }
}

pub(crate) fn errors_from(d: ReplicaErrorsDto) -> ReplicaErrors {
    ReplicaErrors {
        replica: d.replica,
        errors: d
            .container_errors
            .into_iter()
            .filter_map(|c| {
                Some(ContainerError {
                    container: ContainerName::new(&c.container).ok()?,
                    at: c
                        .timestamp
                        .as_deref()
                        .and_then(|t| Timestamp::parse_rfc3339(t).ok()),
                    code: c.code,
                    message: c.message,
                    details: c.details,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Captured from a real daemon (v1.8), names neutralised.
    const BUFFERS: &str = r#"{"buffers":[{"pipeline":"p", "bufferName":"default-p-cat-0", "pendingCount":"7", "ackPendingCount":"0", "totalMessages":"7", "bufferLength":"30000", "bufferUsageLimit":0.8, "bufferUsage":0.0002, "isFull":false}]}"#;
    const METRICS: &str = r#"{"vertexMetrics":[{"pipeline":"p", "vertex":"cat", "processingRates":{"15m":-9223372036854776000, "1m":4.5, "5m":"NaN", "default":4.2}, "pendings":{"15m":"0", "1m":"3", "5m":"-9223372036854775808", "default":"3"}}]}"#;
    const WATERMARKS: &str = r#"{"pipelineWatermarks":[{"pipeline":"p", "edge":"in-cat", "watermarks":["-1", "1757200000000"], "isWatermarkEnabled":true, "from":"in", "to":"cat"}]}"#;
    const STATUS: &str = r#"{"status":{"status":"healthy", "message":"Pipeline data flow is healthy", "code":"D1"}}"#;

    fn v(s: &str) -> VertexName {
        VertexName::new(s).unwrap()
    }

    #[test]
    fn buffers_parse_int64_strings() {
        let d: ListBuffersDto = serde_json::from_str(BUFFERS).unwrap();
        let b = buffer_from(&d.buffers[0], v("in"), v("cat")).unwrap();
        assert_eq!(b.pending, Some(7));
        assert_eq!(b.length, Some(30000));
        assert_eq!(b.usage_limit.map(Fraction::get), Some(0.8));
        assert_eq!(b.is_full, Some(false));
    }

    #[test]
    fn metrics_map_sentinels_to_none() {
        let d: VertexMetricsListDto = serde_json::from_str(METRICS).unwrap();
        let m = metrics_from(&d.vertex_metrics[0]).unwrap();
        assert_eq!(m.rate.m15, None, "i64::MIN-as-float sentinel");
        assert_eq!(m.rate.m5, None, "NaN");
        assert_eq!(m.rate.m1, Some(4.5));
        assert_eq!(m.pending.m5, None, "i64::MIN string sentinel");
        assert_eq!(m.pending.m1, Some(3));
    }

    #[test]
    fn watermarks_minus_one_is_none() {
        let d: WatermarksDto = serde_json::from_str(WATERMARKS).unwrap();
        let w = watermark_from(d.pipeline_watermarks.into_iter().next().unwrap()).unwrap();
        assert_eq!(w.per_partition.len(), 2);
        assert!(w.per_partition[0].is_none());
        assert!(w.per_partition[1].is_some());
        assert!(w.enabled);
    }

    #[test]
    fn status_and_empty_errors() {
        let d: GetStatusDto = serde_json::from_str(STATUS).unwrap();
        let h = health_from(d.status.unwrap());
        assert_eq!(h.status, Health::Healthy);
        assert_eq!(h.code, "D1");
        let e: GetErrorsDto = serde_json::from_str("{}").unwrap();
        assert!(e.errors.is_empty());
        let bad: Result<VertexMetrics, _> = metrics_from(&VertexMetricsDto {
            vertex: "Bad".into(),
            ..Default::default()
        });
        assert!(bad.is_err());
    }
}
