//! Renderers. Tables follow kubectl's conventions: upper-case headers, two-space
//! gutters, no borders, so the output pipes into `awk` and `column` cleanly.

use std::fmt::Write as _;
use std::time::Duration;

use nfctl_core::model::{Pipeline, Timestamp};
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

/// Render pipelines as a table, or as JSON/YAML.
pub fn pipelines(
    items: &[Pipeline],
    fmt: OutputFormat,
    with_namespace: bool,
    now: Timestamp,
) -> Result<String> {
    match fmt {
        OutputFormat::Json | OutputFormat::Yaml => serialised(&items, fmt),
        OutputFormat::Table | OutputFormat::Wide => {
            if items.is_empty() {
                return Ok("No pipelines found.\n".to_owned());
            }
            let wide = fmt == OutputFormat::Wide;
            let mut header = Vec::new();
            if with_namespace {
                header.push("NAMESPACE");
            }
            header.extend([
                "NAME", "PHASE", "VERTICES", "SOURCES", "SINKS", "UDFS", "AGE",
            ]);
            if wide {
                header.extend(["DESIRED", "ISB", "MESSAGE"]);
            }
            let mut t = Table::new(header);
            for p in items {
                let mut row = Vec::new();
                if with_namespace {
                    row.push(p.key.namespace.to_string());
                }
                let c = p.status.counts;
                row.extend([
                    p.key.name.to_string(),
                    p.status.phase.as_str().to_owned(),
                    c.total.to_string(),
                    c.sources.to_string(),
                    c.sinks.to_string(),
                    c.udfs.to_string(),
                    age(p.age(now)),
                ]);
                if wide {
                    row.extend([
                        p.spec.lifecycle.desired.as_str().to_owned(),
                        p.spec.isb.to_string(),
                        p.status.message.clone().unwrap_or_default(),
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
            format!("{} -> {}", e.from, e.to),
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
        "NAME",
        "PHASE",
        "HEALTHY",
        "REPLICAS",
        "VERSION",
        "PERSISTENT",
    ]);
    for i in items {
        t.row(vec![
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

/// `mvtx ls` / `mvtx get`.
pub fn monovertices(
    items: &[nfctl_core::model::MonoVertex],
    fmt: OutputFormat,
    with_namespace: bool,
    now: Timestamp,
) -> Result<String> {
    if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
        return serialised(&items, fmt);
    }
    if items.is_empty() {
        return Ok("No monovertices found.\n".to_owned());
    }
    let wide = fmt == OutputFormat::Wide;
    let mut header = Vec::new();
    if with_namespace {
        header.push("NAMESPACE");
    }
    header.extend(["NAME", "PHASE", "REPLICAS", "READY", "AGE"]);
    if wide {
        header.extend(["DESIRED", "TRANSFORMER", "MAP", "MESSAGE"]);
    }
    let mut t = Table::new(header);
    for m in items {
        let mut row = Vec::new();
        if with_namespace {
            row.push(m.key.namespace.to_string());
        }
        let yes_no = |b: bool| if b { "yes".to_owned() } else { "no".to_owned() };
        row.extend([
            m.key.name.to_string(),
            m.phase.as_str().to_owned(),
            m.replicas.to_string(),
            m.ready_replicas
                .map_or_else(|| "-".to_owned(), |n| n.to_string()),
            age(m.created.and_then(|c| c.elapsed_until(now))),
        ]);
        if wide {
            row.extend([
                m.desired.as_str().to_owned(),
                yes_no(m.has_transformer),
                yes_no(m.has_map),
                m.message.clone().unwrap_or_default(),
            ]);
        }
        t.row(row);
    }
    Ok(t.render())
}

/// `mvtx status`.
pub fn monovertex_status(
    m: &nfctl_core::model::MonoVertex,
    health: Option<&nfctl_core::model::PipelineHealth>,
    metrics: Option<&[nfctl_core::model::VertexMetrics]>,
    warnings: &[String],
    fmt: OutputFormat,
) -> Result<String> {
    if matches!(fmt, OutputFormat::Json | OutputFormat::Yaml) {
        #[derive(Serialize)]
        struct View<'a> {
            monovertex: &'a nfctl_core::model::MonoVertex,
            health: Option<&'a nfctl_core::model::PipelineHealth>,
            metrics: Option<&'a [nfctl_core::model::VertexMetrics]>,
            warnings: &'a [String],
        }
        return serialised(
            &View {
                monovertex: m,
                health,
                metrics,
                warnings,
            },
            fmt,
        );
    }
    let mut out = String::new();
    let h = health.map_or("unknown", |h| h.status.as_str());
    let _ = writeln!(
        out,
        "{}  phase={}  health={h}  desired={}  replicas={}/{}",
        m.key,
        m.phase.as_str(),
        m.desired.as_str(),
        m.ready_replicas.unwrap_or(0),
        m.replicas
    );
    if let Some(h) = health
        && !h.message.is_empty()
    {
        let _ = writeln!(out, "{} ({})", h.message, h.code);
    }
    if let Some(msg) = &m.message {
        let _ = writeln!(out, "{msg}");
    }
    if let Some(mm) = metrics.and_then(|v| v.first()) {
        let _ = writeln!(
            out,
            "rate/1m={}  rate/5m={}  pending={}",
            opt_rate(mm.rate.m1),
            opt_rate(mm.rate.m5),
            opt_i64(mm.pending.default.or(mm.pending.m1))
        );
    }
    for w in warnings {
        let _ = writeln!(out, "warning: {w}");
    }
    Ok(out)
}
