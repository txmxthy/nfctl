//! Renderers. Tables follow kubectl's conventions: upper-case headers, two-space
//! gutters, no borders, so the output pipes into `awk` and `column` cleanly.

use std::fmt::Write as _;
use std::time::Duration;

use nfctl_core::model::{Pipeline, Timestamp, Workload};
use nfctl_core::service::MonoVertexView;
use nfctl_core::{Error, Result};
use serde::Serialize;
use unicode_width::UnicodeWidthStr;

use crate::cli::OutputFormat;

/// A left-aligned, padded table.
#[derive(Debug, Default)]
pub struct Table {
    header: Vec<&'static str>,
    rows: Vec<Vec<String>>,
}

impl Table {
    #[must_use]
    pub fn new(header: Vec<&'static str>) -> Self {
        Self {
            header,
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        debug_assert_eq!(cells.len(), self.header.len());
        self.rows.push(cells);
    }

    #[must_use]
    pub fn render(&self) -> String {
        let n = self.header.len();
        let mut widths: Vec<usize> = self.header.iter().map(|h| h.width()).collect();
        for r in &self.rows {
            for (i, c) in r.iter().enumerate().take(n) {
                widths[i] = widths[i].max(c.width());
            }
        }
        let mut out = String::new();
        let line = |out: &mut String, cells: &[&str]| {
            for (i, c) in cells.iter().enumerate() {
                let last = i + 1 == cells.len();
                out.push_str(c);
                if !last {
                    let pad = widths[i].saturating_sub(c.width()) + 2;
                    out.extend(std::iter::repeat_n(' ', pad));
                }
            }
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
        };
        line(&mut out, &self.header);
        for r in &self.rows {
            let cells: Vec<&str> = r.iter().map(String::as_str).collect();
            line(&mut out, &cells);
        }
        out
    }
}

/// kubectl-style age: `42s`, `7m`, `3h`, `12d`.
#[must_use]
pub fn age(d: Option<Duration>) -> String {
    let Some(d) = d else {
        return "<unknown>".to_owned();
    };
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// Render workloads of either kind as a table, or as JSON/YAML. A column that
/// only one kind has shows `-` for the other rather than being left out: the
/// list is one table, so every row has to line up.
pub fn workloads(
    items: &[Workload],
    fmt: OutputFormat,
    with_namespace: bool,
    now: Timestamp,
) -> Result<String> {
    match fmt {
        OutputFormat::Json | OutputFormat::Yaml => serialised(&items, fmt),
        OutputFormat::Table | OutputFormat::Wide => {
            if items.is_empty() {
                return Ok("No pipelines or MonoVertices found.\n".to_owned());
            }
            let wide = fmt == OutputFormat::Wide;
            let mut header = Vec::new();
            if with_namespace {
                header.push("NAMESPACE");
            }
            header.extend(["KIND", "NAME", "PHASE", "VERTICES", "AGE"]);
            if wide {
                header.extend([
                    "DESIRED", "SOURCES", "SINKS", "UDFS", "REPLICAS", "READY", "ISB", "MESSAGE",
                ]);
            }
            let mut t = Table::new(header);
            for w in items {
                let mut row = Vec::new();
                if with_namespace {
                    row.push(w.namespace().to_string());
                }
                row.extend([
                    w.kind().as_str().to_owned(),
                    w.name().to_string(),
                    w.phase().as_str().to_owned(),
                    w.vertices().to_string(),
                    age(w.age(now)),
                ]);
                if wide {
                    let dash = || "-".to_owned();
                    let counts = w.counts();
                    let replicas = w.replicas();
                    row.extend([
                        w.desired().as_str().to_owned(),
                        counts.map_or_else(dash, |c| c.sources.to_string()),
                        counts.map_or_else(dash, |c| c.sinks.to_string()),
                        counts.map_or_else(dash, |c| c.udfs.to_string()),
                        replicas.map_or_else(dash, |(n, _)| n.to_string()),
                        replicas.map_or_else(dash, |(_, ready)| {
                            ready.map_or_else(dash, |n| n.to_string())
                        }),
                        w.isb().map_or_else(dash, ToString::to_string),
                        w.message().unwrap_or_default().to_owned(),
                    ]);
                }
                t.row(row);
            }
            Ok(t.render())
        }
    }
}

/// JSON or YAML for any serialisable value.
pub fn serialised<T: Serialize>(value: &T, fmt: OutputFormat) -> Result<String> {
    let mut s = match fmt {
        OutputFormat::Json => {
            serde_json::to_string_pretty(value).map_err(|e| Error::Cluster(Box::new(e)))?
        }
        OutputFormat::Yaml => {
            serde_yaml_ng::to_string(value).map_err(|e| Error::Cluster(Box::new(e)))?
        }
        OutputFormat::Table | OutputFormat::Wide => {
            unreachable!("serialised() is only called for json/yaml")
        }
    };
    if !s.ends_with('\n') {
        let _ = writeln!(s);
    }
    Ok(s)
}

/// One value in a machine-readable stream: compact JSON for NDJSON, or one
/// explicit YAML document. Serialization failures remain valid stream values.
pub fn stream_item<T: Serialize>(value: &T, fmt: OutputFormat) -> String {
    match fmt {
        OutputFormat::Json => serde_json::to_string(value)
            .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }).to_string()),
        OutputFormat::Yaml => match serde_yaml_ng::to_string(value) {
            Ok(value) => format!("---\n{}", value.trim_end()),
            Err(e) => format!("---\nerror: {:?}", e.to_string()),
        },
        OutputFormat::Table | OutputFormat::Wide => {
            unreachable!("stream_item() is only called for json/yaml")
        }
    }
}

fn opt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

fn opt_rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |r| format!("{r:.1}"))
}

fn opt_pct(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |f| format!("{:.0}%", f * 100.0))
}

fn vertex_table(vertices: &[nfctl_core::service::VertexView]) -> String {
    let mut t = Table::new(vec![
        "VERTEX", "KIND", "PARTS", "RATE/1m", "RATE/5m", "PENDING",
    ]);
    for x in vertices {
        t.row(vec![
            x.name.to_string(),
            x.kind.as_str().to_owned(),
            x.partitions.to_string(),
            opt_rate(x.rate.m1),
            opt_rate(x.rate.m5),
            opt_i64(x.pending.default.or(x.pending.m1)),
        ]);
    }
    t.render()
}

fn edge_table(edges: &[nfctl_core::service::EdgeView], at: Timestamp) -> String {
    let mut t = Table::new(vec![
        "EDGE",
        "PENDING",
        "ACK-PENDING",
        "USAGE",
        "FULL",
        "WATERMARK",
    ]);
    for e in edges {
        let ack = e
            .buffers
            .iter()
            .filter_map(|b| b.ack_pending)
            .reduce(|a, b| a + b);
        let wm = e.watermark.as_ref().map_or_else(
            || "-".to_owned(),
            |w| {
                if !w.enabled {
                    return "disabled".to_owned();
                }
                match w.per_partition.iter().flatten().max() {
                    Some(ts) => ts
                        .elapsed_until(at)
                        .map_or_else(|| "ahead".to_owned(), |d| format!("{} ago", age(Some(d)))),
                    None => "-".to_owned(),
                }
            },
        );
        t.row(vec![
            edge_label(e),
            opt_i64(e.pending()),
            opt_i64(ack),
            opt_pct(e.usage()),
            if e.is_full() {
                "yes".to_owned()
            } else {
                "no".to_owned()
            },
            wm,
        ]);
    }
    t.render()
}

fn edge_label(edge: &nfctl_core::service::EdgeView) -> String {
    let Some(buffer) = edge.buffers.first() else {
        return format!("{} -> {}", edge.from, edge.to);
    };
    let sources = buffer
        .sources
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if buffer.sources.len() > 1 {
        format!("{{{sources}}} -> {}", buffer.to)
    } else {
        format!("{sources} -> {}", buffer.to)
    }
}

/// The `top`/`status` screen.
pub fn view(v: &nfctl_core::service::PipelineView, fmt: OutputFormat) -> Result<String> {
    if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
        return serialised(v, fmt);
    }
    let p = &v.pipeline;
    let mut out = String::new();
    let health = v
        .health
        .as_ref()
        .map_or_else(|| "unknown".to_owned(), |h| h.status.as_str().to_owned());
    let _ = writeln!(
        out,
        "{}  phase={}  health={}  desired={}",
        p.key,
        p.status.phase.as_str(),
        health,
        p.spec.lifecycle.desired.as_str()
    );
    if let Some(h) = &v.health
        && !h.message.is_empty()
    {
        let _ = writeln!(out, "{} ({})", h.message, h.code);
    }
    if let Some(m) = &p.status.message {
        let _ = writeln!(out, "{m}");
    }
    if p.status.phase == nfctl_core::model::PipelinePhase::Pausing
        || p.status.phase == nfctl_core::model::PipelinePhase::Paused
    {
        let drained = match v.drained() {
            Some(true) => "drained",
            Some(false) => "draining",
            None => "drain state unknown",
        };
        let _ = writeln!(
            out,
            "pause: {drained}; controller reports drainedOnPause={}",
            p.status.drained_on_pause
        );
    }
    out.push('\n');

    out.push_str(&vertex_table(&v.vertices));
    out.push('\n');
    out.push_str(&edge_table(&v.edges, v.at));
    for w in &v.warnings {
        let _ = writeln!(out, "\nwarning: {w}");
    }
    Ok(out)
}

/// `isb ls`.
pub fn isbs(items: &[nfctl_core::model::IsbService], fmt: OutputFormat) -> Result<String> {
    if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
        return serialised(&items, fmt);
    }
    if items.is_empty() {
        return Ok("No inter-step buffer services found.\n".to_owned());
    }
    let mut t = Table::new(vec![
        "NAMESPACE",
        "NAME",
        "PHASE",
        "HEALTHY",
        "REPLICAS",
        "VERSION",
        "PERSISTENT",
    ]);
    for i in items {
        t.row(vec![
            i.namespace.to_string(),
            i.name.to_string(),
            format!("{:?}", i.phase),
            if i.healthy {
                "yes".to_owned()
            } else {
                "no".to_owned()
            },
            i.replicas.to_string(),
            i.version.clone(),
            if i.persistent {
                "yes".to_owned()
            } else {
                "no".to_owned()
            },
        ]);
    }
    Ok(t.render())
}

/// `isb inspect`: the service plus the pipelines that use it.
pub fn isb_detail(
    isb: &nfctl_core::model::IsbService,
    users: &[Pipeline],
    fmt: OutputFormat,
) -> Result<String> {
    if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
        #[derive(Serialize)]
        struct Detail<'a> {
            service: &'a nfctl_core::model::IsbService,
            pipelines: Vec<&'a nfctl_core::model::PipelineKey>,
        }
        return serialised(
            &Detail {
                service: isb,
                pipelines: users.iter().map(|p| &p.key).collect(),
            },
            fmt,
        );
    }
    let mut out = isbs(std::slice::from_ref(isb), fmt)?;
    out.push('\n');
    if users.is_empty() {
        out.push_str("No pipelines use this service.\n");
    } else {
        let mut t = Table::new(vec!["PIPELINE", "PHASE", "VERTICES"]);
        for p in users {
            t.row(vec![
                p.key.name.to_string(),
                p.status.phase.as_str().to_owned(),
                p.status.counts.total.to_string(),
            ]);
        }
        out.push_str(&t.render());
    }
    Ok(out)
}

/// Human summary of an apply check.
#[must_use]
pub fn apply_report(
    key: &nfctl_core::model::PipelineKey,
    r: &nfctl_core::service::ApplyReport,
    exists: bool,
) -> String {
    let mut out = String::new();
    if !exists {
        let _ = writeln!(out, "{key}: does not exist yet; will be created");
        return out;
    }
    for b in &r.blocks {
        let _ = writeln!(out, "BLOCK  {b}");
    }
    for w in &r.warnings {
        let _ = writeln!(out, "WARN   {w}");
    }
    if let Some(d) = &r.diff
        && !d.shape_changed()
        && d.kind_changed.is_empty()
        && d.partitions_changed.is_empty()
        && d.image_changed.is_empty()
        && r.blocks.is_empty()
    {
        let _ = writeln!(out, "{key}: no topology changes");
    }
    if !r.blocks.is_empty() {
        let _ = writeln!(
            out,
            "{key}: refusing to apply; delete and recreate the pipeline instead"
        );
    }
    out
}

/// `mvtx status`: the `MonoVertex` half of what `status` shows for a pipeline.
pub fn monovertex_view(v: &MonoVertexView, fmt: OutputFormat) -> Result<String> {
    if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
        return serialised(v, fmt);
    }
    let m = &v.monovertex;
    let mut out = String::new();
    let h = v.health.as_ref().map_or("unknown", |h| h.status.as_str());
    let _ = writeln!(
        out,
        "{}  phase={}  health={h}  desired={}  replicas={}/{}",
        m.key,
        m.phase.as_str(),
        m.desired.as_str(),
        m.ready_replicas.unwrap_or(0),
        m.replicas
    );
    if let Some(h) = &v.health
        && !h.message.is_empty()
    {
        let _ = writeln!(out, "{} ({})", h.message, h.code);
    }
    if let Some(msg) = &m.message {
        let _ = writeln!(out, "{msg}");
    }
    if let Some(mm) = v.metrics() {
        let _ = writeln!(
            out,
            "rate/1m={}  rate/5m={}  pending={}",
            opt_rate(mm.rate.m1),
            opt_rate(mm.rate.m5),
            opt_i64(mm.pending.default.or(mm.pending.m1))
        );
    }
    for w in &v.warnings {
        let _ = writeln!(out, "warning: {w}");
    }
    Ok(out)
}
