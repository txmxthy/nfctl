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
.frame{display:none}.frame.on{display:block}
.stage{position:relative;display:inline-block;max-width:100%;overflow:auto;background:var(--bg)}
.ink{position:absolute;left:0;top:0;cursor:crosshair;touch-action:none}
.tools{display:flex;flex-wrap:wrap;gap:6px;align-items:center;margin:0 0 8px;color:var(--muted);font-size:12px}
.tools button{font:inherit;background:var(--line);color:var(--fg);border:0;padding:3px 8px;border-radius:3px;cursor:pointer}
.tools button.sw{width:22px;height:22px;padding:0;border:2px solid transparent}.tools button.sw.on{border-color:var(--fg)}
.tools textarea{font:inherit;background:var(--panel);color:var(--fg);border:1px solid var(--line);border-radius:3px;padding:4px 6px;min-height:38px;resize:vertical;margin-top:6px}
.tools .hint{margin-left:auto}
.frame textarea{display:block;width:min(900px,100%);box-sizing:border-box}
pre{margin:0;white-space:pre;line-height:1.2;font-variant-numeric:tabular-nums}
.c-cyan{color:var(--cyan)}.c-magenta{color:var(--magenta)}.c-yellow{color:var(--yellow)}.c-green{color:var(--green)}.c-blue{color:var(--blue)}.c-red{color:var(--red)}.c-dim{color:var(--dim)}.c-fg{color:var(--fg)}
.bold{font-weight:700}.rev{background:var(--fg);color:var(--bg)}
@media (prefers-reduced-motion:no-preference){nav a,.tabs button{transition:background .12s}}
";

const SCRIPT: &str = r"
const items=[...document.querySelectorAll('nav a')];
let tab=localStorage.getItem('tab')||'cards 160 collapsed';
let colour='#f38ba8';
// Annotations: one store, keyed by pipeline name and tab. Saved to the notes
// server started by `just gallery` (target/gallery/notes.json) so they reach
// the repo, with localStorage as the offline fallback.
const API='http://127.0.0.1:8767';
const key=f=>f.closest('section').querySelector('h2').firstChild.textContent+' | '+f.dataset.tab;
let store={};let online=false;
try{store=JSON.parse(localStorage.getItem('ink')||'{}')}catch(e){}
// Notes made before the store existed were keyed by section id; carry them over.
for(const k of Object.keys(localStorage)){const m=k.match(/^ink:(i\d+):(.*)$/);if(!m)continue;
 const sec=document.getElementById(m[1]);if(sec){const name=sec.querySelector('h2').firstChild.textContent+' | '+m[2];if(!store[name])try{store[name]=JSON.parse(localStorage.getItem(k))}catch(e){}}
 localStorage.removeItem(k);}
const load=f=>store[key(f)]||{strokes:[],note:''};
let pending=null;
function flush(){pending=null;try{localStorage.setItem('ink',JSON.stringify(store))}catch(e){}
 if(online)fetch(API+'/notes.json',{method:'PUT',body:JSON.stringify(store)}).then(r=>setStatus(r.ok?'saved to target/gallery/notes.json':'save failed')).catch(()=>{online=false;setStatus('notes server gone; saving in browser only')});}
const save=(f,d)=>{store[key(f)]=d;clearTimeout(pending);pending=setTimeout(flush,400);};
const setStatus=t=>document.querySelectorAll('.hint').forEach(h=>h.textContent=t);
fetch(API+'/notes.json').then(r=>r.json()).then(remote=>{online=true;
 // Union: anything drawn here before the server existed is kept and pushed.
 for(const k in remote)if(!store[k])store[k]=remote[k];
 flush();document.querySelectorAll('.frame.on').forEach(f=>{paint(f);f.querySelector('textarea').value=load(f).note;});
}).catch(()=>setStatus('notes server not running (just gallery starts it); saving in browser only'));
function fit(f){const pre=f.querySelector('pre'),c=f.querySelector('canvas');if(!pre||!c)return;
 const w=pre.scrollWidth,h=pre.scrollHeight,r=devicePixelRatio||1;
 c.style.width=w+'px';c.style.height=h+'px';c.width=w*r;c.height=h*r;paint(f);}
function paint(f){const c=f.querySelector('canvas'),g=c.getContext('2d'),r=devicePixelRatio||1,d=load(f);
 g.setTransform(r,0,0,r,0,0);g.clearRect(0,0,c.width,c.height);g.lineCap='round';g.lineJoin='round';g.lineWidth=2.5;
 for(const s of d.strokes){g.strokeStyle=s.c;g.beginPath();s.p.forEach((q,i)=>i?g.lineTo(q[0],q[1]):g.moveTo(q[0],q[1]));g.stroke();}}
function show(id){items.forEach(a=>a.classList.toggle('on',a.dataset.id===id));
 document.querySelectorAll('section').forEach(s=>s.style.display=s.id===id?'block':'none');
 document.querySelectorAll('.tabs button').forEach(b=>b.classList.toggle('on',b.dataset.tab===tab));
 document.querySelectorAll('.frame').forEach(f=>{const on=f.dataset.tab===tab&&f.closest('section').id===id;f.classList.toggle('on',on);if(on){fit(f);f.querySelector('textarea').value=load(f).note;}});
 location.hash=id;}
items.forEach(a=>a.onclick=e=>{e.preventDefault();show(a.dataset.id)});
document.querySelectorAll('.tabs button').forEach(b=>b.onclick=()=>{tab=b.dataset.tab;localStorage.setItem('tab',tab);show(location.hash.slice(1))});
document.querySelectorAll('.tools').forEach(t=>t.addEventListener('click',e=>{const b=e.target.closest('button');if(!b)return;
 const f=t.closest('section').querySelector('.frame.on');
 if(b.dataset.c){colour=b.dataset.c;document.querySelectorAll('.sw').forEach(x=>x.classList.toggle('on',x.dataset.c===colour));}
 else if(b.dataset.act==='undo'){const d=load(f);d.strokes.pop();save(f,d);paint(f);}
 else if(b.dataset.act==='clear'){const d=load(f);d.strokes=[];save(f,d);paint(f);}
 else if(b.dataset.act==='png'){exportPng(f);}
 else if(b.dataset.act==='all'){exportAll();}}));
document.querySelectorAll('.frame textarea').forEach(ta=>ta.oninput=()=>{const f=ta.closest('.frame'),d=load(f);d.note=ta.value;save(f,d);});
document.querySelectorAll('canvas.ink').forEach(c=>{let cur=null;const at=e=>{const r=c.getBoundingClientRect();return [e.clientX-r.left,e.clientY-r.top]};
 c.onpointerdown=e=>{c.setPointerCapture(e.pointerId);cur={c:colour,p:[at(e)]};};
 c.onpointermove=e=>{if(!cur)return;cur.p.push(at(e));const f=c.closest('.frame'),g=c.getContext('2d'),r=devicePixelRatio||1;g.setTransform(r,0,0,r,0,0);g.strokeStyle=cur.c;g.lineWidth=2.5;g.lineCap='round';g.beginPath();const n=cur.p.length;g.moveTo(...cur.p[n-2]);g.lineTo(...cur.p[n-1]);g.stroke();};
 c.onpointerup=c.onpointercancel=e=>{if(!cur)return;const f=c.closest('.frame'),d=load(f);if(cur.p.length>1)d.strokes.push(cur);save(f,d);cur=null;paint(f);};});
const colourOf=el=>getComputedStyle(el).color;
function render(f){const pre=f.querySelector('pre'),ink=f.querySelector('canvas'),r=2;
 const cs=getComputedStyle(pre),lh=parseFloat(cs.lineHeight),font=cs.fontSize+' '+cs.fontFamily;
 const o=document.createElement('canvas');o.width=pre.scrollWidth*r;o.height=(pre.scrollHeight+lh)*r;const g=o.getContext('2d');g.scale(r,r);
 g.fillStyle=getComputedStyle(document.body).backgroundColor;g.fillRect(0,0,o.width,o.height);g.font=font;g.textBaseline='top';
 const pad=parseFloat(cs.paddingLeft)||0;let y=parseFloat(cs.paddingTop)||0;
 for(const line of pre.innerHTML.split('\n')){const tmp=document.createElement('pre');tmp.innerHTML=line;let x=pad;
  for(const n of tmp.childNodes){const t=n.textContent;const span=n.nodeType===1?n:null;g.fillStyle=span?colourFor(span.className):colourOf(pre);g.font=(span&&span.className.includes('bold')?'bold ':'')+font;g.fillText(t,x,y);x+=g.measureText(t).width;}
  y+=lh;}
 g.drawImage(ink,0,0,ink.width,ink.height,0,0,ink.width/(devicePixelRatio||1),ink.height/(devicePixelRatio||1));
 const note=load(f).note;if(note){g.font='bold 14px sans-serif';g.fillStyle='#f38ba8';g.fillText(note,pad,y+4);}
 return o;}
const palette={};function colourFor(cls){const k=cls.split(' ')[0];if(!palette[k]){const s=document.createElement('span');s.className=k;document.body.appendChild(s);palette[k]=colourOf(s);s.remove();}return palette[k];}
const dl=(name,url)=>{const a=document.createElement('a');a.href=url;a.download=name;a.click();};
window.onbeforeunload=()=>{if(pending)flush();};
const nameOf=f=>(f.closest('section').querySelector('h2').firstChild.textContent+' '+f.dataset.tab).replace(/[^a-z0-9]+/gi,'-');
function exportPng(f){const o=render(f),url=o.toDataURL('image/png');
 if(online)o.toBlob(b=>fetch(API+'/png/'+nameOf(f)+'.png',{method:'POST',body:b}).then(()=>setStatus('PNG saved to target/gallery/png/')).catch(()=>dl(nameOf(f)+'.png',url)));else dl(nameOf(f)+'.png',url);}
function exportAll(){let n=0;document.querySelectorAll('.frame').forEach(f=>{const d=load(f);if(!d.strokes.length&&!d.note)return;const shown=f.classList.contains('on');if(!shown){f.classList.add('on');fit(f);}
 setTimeout(()=>{exportPng(f);if(!shown)f.classList.remove('on');},150*n++);});if(!n)alert('nothing annotated yet');}
show(location.hash.slice(1)||items[0].dataset.id);
window.onhashchange=()=>show(location.hash.slice(1));
window.onresize=()=>document.querySelectorAll('.frame.on').forEach(fit);
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
        html.push_str("</div><div class=\"tools\"><span>draw:</span><button class=\"sw on\" data-c=\"#f38ba8\" style=\"background:#f38ba8\"></button><button class=\"sw\" data-c=\"#f9e2af\" style=\"background:#f9e2af\"></button><button class=\"sw\" data-c=\"#a6e3a1\" style=\"background:#a6e3a1\"></button><button class=\"sw\" data-c=\"#cdd6f4\" style=\"background:#cdd6f4\"></button><button data-act=\"undo\">undo</button><button data-act=\"clear\">clear</button><button data-act=\"png\">export PNG</button><button data-act=\"all\">export all annotated</button><span class=\"hint\">drawings and notes persist in this browser</span></div>");
        for (label, body) in &frames {
            let _ = write!(
                html,
                "<div class=\"frame\" data-tab=\"{label}\"><div class=\"stage\">{body}<canvas class=\"ink\"></canvas></div><textarea placeholder=\"Notes for this frame (saved with the drawing)\"></textarea></div>"
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
