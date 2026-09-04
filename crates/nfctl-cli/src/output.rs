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
