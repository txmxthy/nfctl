#![allow(clippy::print_stderr, clippy::unwrap_used, clippy::expect_used)]
//! Render every pipeline the test suite knows (the demo fixture, plus the
//! private corpus when present) through both renderers into one HTML page,
//! so layout changes can be eyeballed side by side.
//!
//!   cargo run -p nfctl-tui --example gallery -- target/gallery/index.html [--public]
//!
//! `--public` leaves the private corpus out, for a page that can be shared.
//! The page is static: a sidebar of pipelines, tabs for the card view at
//! several widths (collapsed and expanded shards) and the `dag` box-drawing
//! output, all with the same colours the terminal would show.

#[path = "../../nfctl-graph/tests/common/mod.rs"]
mod common;

use std::fmt::Write as _;

use nfctl_core::fake::{FakeCluster, FakeDaemons, Fixture};
use nfctl_core::model::{Namespace, PipelineKey, PipelineName, PipelinePhase, Timestamp, Topology};
use nfctl_core::service::pipeline_view;
use nfctl_graph::layout::ViewGraph;
use nfctl_graph::{Format, RenderOptions, from_mermaid, render_with, to_mermaid};
use nfctl_tui::panels::detail::DetailPanel;
use nfctl_tui::{AppEvent, Model, WorkerReply};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

const WIDTHS: [u16; 3] = [110, 160, 220];

struct Item {
    name: String,
    topology: Topology,
}

fn items(public: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let fixture = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/fixtures/demo.yaml"),
    )
    .expect("demo fixture");
    let fixture = Fixture::parse(&fixture).expect("fixture parses");
    for p in &fixture.pipelines {
        out.push(Item {
            name: format!("fixture/{}", p.key.name),
            topology: p.spec.topology.clone(),
        });
    }
    if public {
        return out;
    }
    for (file, src) in common::corpus() {
        for (pl, topology) in from_mermaid(&src).expect("corpus parses") {
            out.push(Item {
                name: format!("corpus/{file}/{pl}"),
                topology,
            });
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn class(fg: Color, m: Modifier) -> String {
    let mut c = match fg {
        Color::Cyan => "c-cyan",
        Color::Magenta => "c-magenta",
        Color::Yellow => "c-yellow",
        Color::Green => "c-green",
        Color::Blue => "c-blue",
        Color::Red => "c-red",
        Color::DarkGray => "c-dim",
        _ => "c-fg",
    }
    .to_owned();
    if m.contains(Modifier::BOLD) {
        c.push_str(" bold");
    }
    if m.contains(Modifier::REVERSED) {
        c.push_str(" rev");
    }
    c
}

/// A ratatui buffer as `<pre>` with one span per run of equal style.
fn buffer_html(buf: &Buffer) -> String {
    let mut out = String::from("<pre>");
    for y in 0..buf.area.height {
        let mut run = String::new();
        let mut run_class = String::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let c = class(cell.fg, cell.modifier);
            if c != run_class && !run.is_empty() {
                let _ = write!(out, "<span class=\"{run_class}\">{}</span>", escape(&run));
                run.clear();
            }
            run_class = c;
            run.push_str(cell.symbol());
        }
        let _ = writeln!(out, "<span class=\"{run_class}\">{}</span>", escape(&run));
    }
    out.push_str("</pre>");
    out
}

/// ANSI SGR (the CLI's own codes) to spans.
fn ansi_html(s: &str) -> String {
    let mut out = String::from("<pre>");
    let mut chars = s.chars().peekable();
    let mut open = false;
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            let mut code = String::new();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
                if c != '[' {
                    code.push(c);
                }
            }
            if open {
                out.push_str("</span>");
                open = false;
            }
            let cls = match code.as_str() {
                "36" => Some("c-cyan"),
                "35" => Some("c-magenta"),
                "33" => Some("c-yellow"),
                "32" => Some("c-green"),
                "34" => Some("c-blue"),
                "31" => Some("c-red"),
                "2" => Some("c-dim"),
                _ => None,
            };
            if let Some(cls) = cls {
                let _ = write!(out, "<span class=\"{cls}\">");
                open = true;
            }
        } else {
            out.push_str(&escape(&c.to_string()));
        }
    }
    if open {
        out.push_str("</span>");
    }
    out.push_str("</pre>");
    out
}

async fn card_frames(item: &Item) -> Vec<(String, String)> {
    let mut p = nfctl_core::fake::sample_pipeline("gallery", "p", PipelinePhase::Running);
    p.key = PipelineKey::new(
        Namespace::new("gallery").unwrap(),
        PipelineName::new("p").unwrap(),
    );
    p.spec.topology = item.topology.clone();
    let cluster = FakeCluster::with_pipelines(vec![p.clone()]);
    let view = pipeline_view(&cluster, &FakeDaemons::default(), &p.key, Timestamp::now())
        .await
        .unwrap();
    let mut frames = Vec::new();
    for expand in [false, true] {
        let mut panel = DetailPanel::new(p.key.clone());
        panel.update(&AppEvent::Worker(WorkerReply::View(Box::new(Ok(
            view.clone()
        )))));
        if expand {
            panel.update(&AppEvent::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('x'),
                crossterm::event::KeyModifiers::NONE,
            )));
        }
        for w in WIDTHS {
            let nodes = if expand {
                ViewGraph::expanded(&item.topology).nodes.len()
            } else {
                ViewGraph::collapsed(&item.topology).nodes.len()
            };
            let h = u16::try_from(nodes * 5 + 12).unwrap_or(u16::MAX).min(200);
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| panel.view(f, f.area())).unwrap();
            let label = format!(
                "cards {w} {}",
                if expand { "expanded" } else { "collapsed" }
            );
            frames.push((label, buffer_html(term.backend().buffer())));
        }
    }
    frames
}

fn dag_frames(item: &Item) -> Vec<(String, String)> {
    let mut frames = Vec::new();
    for (label, collapse) in [("dag collapsed", true), ("dag expanded", false)] {
        let opts = RenderOptions {
            width: Some(200),
            collapse_shards: collapse,
            colour: true,
        };
        frames.push((
            label.to_owned(),
            ansi_html(&render_with(&item.topology, Format::Ascii, opts)),
        ));
    }
    frames.push((
        "mermaid".to_owned(),
        format!(
            "<pre>{}</pre>",
            escape(&to_mermaid(
                &item.topology,
                nfctl_graph::Direction::LeftRight
            ))
        ),
    ));
    frames
}

const STYLE: &str = r"
:root{--bg:#1b1d27;--panel:#22242f;--line:#343747;--fg:#d6d9e6;--muted:#8b90a6;--accent:#89b4fa;
 --cyan:#89dceb;--magenta:#f5c2e7;--yellow:#f9e2af;--green:#a6e3a1;--blue:#89b4fa;--red:#f38ba8;--dim:#6c7086}
body{margin:0;font:13px/1.35 'JetBrains Mono',ui-monospace,Menlo,Consolas,monospace;background:var(--bg);color:var(--fg);display:flex;height:100vh}
nav{width:270px;overflow:auto;border-right:1px solid var(--line);padding:10px 8px;background:var(--panel)}
nav h1{font-size:13px;margin:0 4px 2px;letter-spacing:.06em;text-transform:uppercase}
nav p{margin:0 4px 10px;color:var(--muted);font-size:12px;line-height:1.4}
nav a{display:block;color:var(--muted);text-decoration:none;padding:2px 6px;border-radius:3px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
nav a.on,nav a:hover{color:var(--fg);background:var(--line)}
nav a:focus-visible,.tabs button:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
main{flex:1;overflow:auto;padding:14px 16px}
h2{font-size:14px;margin:0 0 10px;font-weight:600}
h2 small{color:var(--muted);font-weight:400;margin-left:8px}
.tabs{display:flex;flex-wrap:wrap;gap:4px;margin-bottom:10px}
.tabs button{font:inherit;background:var(--line);color:var(--fg);border:0;padding:4px 10px;border-radius:3px;cursor:pointer}
.tabs button.on{background:var(--accent);color:var(--bg)}
.frame{display:none;overflow-x:auto}.frame.on{display:block}
pre{margin:0;white-space:pre;line-height:1.2;font-variant-numeric:tabular-nums}
.c-cyan{color:var(--cyan)}.c-magenta{color:var(--magenta)}.c-yellow{color:var(--yellow)}.c-green{color:var(--green)}.c-blue{color:var(--blue)}.c-red{color:var(--red)}.c-dim{color:var(--dim)}.c-fg{color:var(--fg)}
.bold{font-weight:700}.rev{background:var(--fg);color:var(--bg)}
@media (prefers-reduced-motion:no-preference){nav a,.tabs button{transition:background .12s}}
";

const SCRIPT: &str = r"
const items=[...document.querySelectorAll('nav a')];const frames=[...document.querySelectorAll('.frame')];
let tab=localStorage.getItem('tab')||'cards 160 collapsed';
function show(id){items.forEach(a=>a.classList.toggle('on',a.dataset.id===id));
 document.querySelectorAll('section').forEach(s=>s.style.display=s.id===id?'block':'none');
 document.querySelectorAll('.tabs button').forEach(b=>b.classList.toggle('on',b.dataset.tab===tab));
 frames.forEach(f=>f.classList.toggle('on',f.dataset.tab===tab));location.hash=id;}
items.forEach(a=>a.onclick=e=>{e.preventDefault();show(a.dataset.id)});
document.querySelectorAll('.tabs button').forEach(b=>b.onclick=()=>{tab=b.dataset.tab;localStorage.setItem('tab',tab);show(location.hash.slice(1))});
show(location.hash.slice(1)||items[0].dataset.id);
window.onhashchange=()=>show(location.hash.slice(1));
";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let public = args.iter().any(|a| a == "--public");
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "target/gallery/index.html".to_owned());
    let items = items(public);
    let mut html = String::new();
    let _ = write!(
        html,
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>nfctl Layout Gallery</title><link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;700&display=swap\"><style>{STYLE}</style><nav><h1>nfctl layout gallery</h1><p>{} pipelines from the test suite, drawn as the terminal would: the card view at 110, 160 and 220 columns with shard groups collapsed or expanded, and the box-drawing <code>dag</code> output. Tabs stick across pipelines.</p>",
        items.len()
    );
    for (i, it) in items.iter().enumerate() {
        let _ = write!(
            html,
            "<a href=\"#i{i}\" data-id=\"i{i}\" title=\"{0}\">{0}</a>",
            escape(&it.name)
        );
    }
    html.push_str("</nav><main>");
    for (i, it) in items.iter().enumerate() {
        let mut frames = card_frames(it).await;
        frames.extend(dag_frames(it));
        let _ = write!(
            html,
            "<section id=\"i{i}\" style=\"display:none\"><h2>{}<small>{} vertices · {} edges</small></h2><div class=\"tabs\">",
            escape(&it.name),
            it.topology.vertices().len(),
            it.topology.edges().len()
        );
        for (label, _) in &frames {
            let _ = write!(html, "<button data-tab=\"{label}\">{label}</button>");
        }
        html.push_str("</div>");
        for (label, body) in &frames {
            let _ = write!(
                html,
                "<div class=\"frame\" data-tab=\"{label}\">{body}</div>"
            );
        }
        html.push_str("</section>");
    }
    let _ = write!(html, "</main><script>{SCRIPT}</script>");
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&out, html).unwrap();
    eprintln!("gallery: {} pipelines → {out}", items.len());
}
