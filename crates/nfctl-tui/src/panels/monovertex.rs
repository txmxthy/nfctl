//! A `MonoVertex`'s detail. There is no topology to draw — one vertex, no
//! edges, no buffers and no watermarks — but there is structure: the container
//! chain inside the pod. The stages are boxed, but nested inside the one card
//! that carries the name, the replica badge and the rate — the wrapper is what
//! says they are one pod, so a box here reads as a container rather than as a
//! vertex with a buffer and a replica count of its own.

use crossterm::event::KeyCode;
use nfctl_core::model::{MonoVertex, MonoVertexKey, WorkloadKey};
use nfctl_core::service::MonoVertexView;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use crate::event::{Action, AppEvent};
use crate::panels::{centre, pressed, waiting};
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, style};

/// Blank columns between the card's frame and the chain inside it.
const PAD: usize = 3;
/// What joins one stage to the next.
const HOP: &str = "──▶";
/// What joins the sink to its fallback. Dashed, because nothing goes that way
/// unless the primary sink fails.
const HOP_FALLBACK: &str = "╌╌▶";
/// Blank columns inside a stage's own box, either side of its label.
const BOX_PAD: usize = 1;

/// How the chain is drawn. The ladder is tried widest first, so a narrow
/// terminal steps down a rung instead of the card overflowing its room.
#[derive(Debug, Clone, Copy)]
struct Step {
    /// Abbreviated wording (`src`) rather than spelled out (`source`).
    short: bool,
    /// Each stage in a box of its own.
    boxed: bool,
}

const LADDER: [Step; 3] = [
    Step {
        short: false,
        boxed: true,
    },
    Step {
        short: true,
        boxed: true,
    },
    Step {
        short: true,
        boxed: false,
    },
];

#[derive(Debug)]
pub struct MonoVertexPanel {
    key: MonoVertexKey,
    view: Option<MonoVertexView>,
    error: Option<String>,
    /// Frame of the spinner while the first load is outstanding.
    spin: usize,
}

impl MonoVertexPanel {
    #[must_use]
    pub fn new(key: MonoVertexKey) -> Self {
        Self {
            key,
            view: None,
            error: None,
            spin: 0,
        }
    }

    fn reload(&self) -> Vec<WorkerMessage> {
        vec![WorkerMessage::LoadMonoVertex(self.key.clone())]
    }

    fn header(v: &MonoVertexView) -> Vec<Line<'static>> {
        let m = &v.monovertex;
        let health = v.health.as_ref().map(|h| h.status);
        let line = Line::from(vec![
            Span::styled("phase ", style::dim()),
            Span::styled(m.phase.as_str(), style::phase(m.phase.into())),
            Span::styled("   health ", style::dim()),
            Span::styled(
                health.map_or("unknown", |h| h.as_str()),
                style::health(health),
            ),
            Span::styled("   desired ", style::dim()),
            Span::raw(m.desired.as_str()),
        ]);
        let said = v
            .health
            .as_ref()
            .map(|h| h.message.clone())
            .or_else(|| m.message.clone())
            .unwrap_or_default();
        let mut lines = vec![line.centered()];
        if !said.is_empty() {
            lines.push(Line::styled(said, style::dim()).centered());
        }
        lines
    }
}

/// The container chain, in the order a message passes through it. The source
/// and the sink are always there; the transformer and the map are optional, and
/// the fallback hangs off the sink. `short` is the abbreviated wording a narrow
/// terminal gets: the labels shrink before the card is allowed to overflow.
fn stages(m: &MonoVertex, short: bool) -> Vec<&'static str> {
    let mut s = vec![if short { "src" } else { "source" }];
    if m.has_transformer {
        s.push(if short { "trf" } else { "transform" });
    }
    if m.has_map {
        s.push("map");
    }
    s.push("sink");
    if m.has_fallback {
        s.push(if short { "fb" } else { "fallback" });
    }
    s
}

/// How wide a hop is drawn. It abuts the boxes when there are boxes, and takes
/// a blank either side when the labels are bare and would otherwise run into
/// it. Measuring and drawing both read this, so they cannot disagree.
fn hop_width(step: Step) -> usize {
    if step.boxed {
        HOP.width()
    } else {
        HOP.width() + 2
    }
}

/// How wide the chain is drawn at this rung. A boxed stage costs its label
/// plus a border and a blank either side.
fn chain_width(m: &MonoVertex, step: Step) -> usize {
    let labels = stages(m, step.short);
    let hops = (labels.len() - 1) * hop_width(step);
    let per_box = if step.boxed { (1 + BOX_PAD) * 2 } else { 0 };
    labels.iter().map(|l| l.width() + per_box).sum::<usize>() + hops
}

/// The chain's rows: three when the stages are boxed (their tops, their labels
/// and their bottoms), one when they are not. The fallback's hop is dashed and
/// its label takes the warning hue, since it is the failure path.
fn chain_rows(m: &MonoVertex, step: Step) -> Vec<Vec<Span<'static>>> {
    let dim = style::dim();
    let labels = stages(m, step.short);
    let last = labels.len() - 1;
    let hue = |i: usize| {
        if m.has_fallback && i == last {
            style::warn()
        } else {
            style::title()
        }
    };
    let hop = |i: usize| {
        if m.has_fallback && i == last {
            HOP_FALLBACK
        } else {
            HOP
        }
    };

    if !step.boxed {
        let mut mid = Vec::new();
        for (i, l) in labels.iter().enumerate() {
            if i > 0 {
                mid.push(Span::styled(format!(" {} ", hop(i)), dim));
            }
            mid.push(Span::styled((*l).to_owned(), hue(i)));
        }
        return vec![mid];
    }

    let (mut top, mut mid, mut bot) = (Vec::new(), Vec::new(), Vec::new());
    for (i, l) in labels.iter().enumerate() {
        if i > 0 {
            // The hop is drawn on the label row; the rows above and below it
            // hold the gap between the two boxes.
            top.push(Span::raw(" ".repeat(hop_width(step))));
            mid.push(Span::styled(hop(i), dim));
            bot.push(Span::raw(" ".repeat(hop_width(step))));
        }
        let rule = "─".repeat(l.width() + BOX_PAD * 2);
        top.push(Span::styled(format!("┌{rule}┐"), dim));
        mid.push(Span::styled("│", dim));
        mid.push(Span::raw(" ".repeat(BOX_PAD)));
        mid.push(Span::styled((*l).to_owned(), hue(i)));
        mid.push(Span::raw(" ".repeat(BOX_PAD)));
        mid.push(Span::styled("│", dim));
        bot.push(Span::styled(format!("└{rule}┘"), dim));
    }
    vec![top, mid, bot]
}

fn opt_rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |r| format!("{r:.1}"))
}

fn opt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

/// `×N` when every replica is ready, `×R/N` when they are not.
fn replicas(m: &MonoVertex) -> String {
    match m.ready_replicas {
        Some(r) if r == m.replicas => format!("×{}", m.replicas),
        Some(r) => format!("×{r}/{}", m.replicas),
        None => format!("×{}", m.replicas),
    }
}

/// The card: a rounded frame with the replica badge worked into the top border
/// and the throughput into the bottom one, around the container chain.
fn card(v: &MonoVertexView, name: &str, room: usize) -> Vec<Line<'static>> {
    let m = &v.monovertex;
    let metrics = v.metrics();
    // The widest rung of the ladder that fits, or the narrowest if none does.
    let fits = |s: &&Step| chain_width(m, **s) + PAD * 2 + 2 <= room;
    let step = LADDER
        .iter()
        .find(fits)
        .copied()
        .unwrap_or(LADDER[LADDER.len() - 1]);
    let chain = chain_width(m, step);

    let badge = replicas(m);
    let rate = opt_rate(metrics.and_then(|x| x.rate.m1));
    let pending = opt_i64(metrics.and_then(|x| x.pending.default.or(x.pending.m1)));
    let flow = format!("{rate}/s · {pending} pending");

    // Wide enough for the name, the chain and both badges, and never wider
    // than the room it has.
    let inner = name.width().max(chain) + PAD * 2;
    let inner = inner
        .max(badge.width() + flow.width() + 8)
        .min(room.saturating_sub(2));

    let dim = style::dim();
    let ready = m.ready_replicas.is_some_and(|r| r == m.replicas);
    let pending_hot = metrics
        .and_then(|x| x.pending.default.or(x.pending.m1))
        .is_some_and(|p| p > 0);

    // ╭─ ×3 ──…──╮ : the badge sits in the border rather than costing a row.
    let top = Line::from(vec![
        Span::styled("╭─ ", dim),
        Span::styled(
            badge.clone(),
            if ready { style::title() } else { style::warn() },
        ),
        Span::styled(
            format!(" {}╮", "─".repeat(inner.saturating_sub(badge.width() + 3))),
            dim,
        ),
    ]);
    // ╰──…── 6753.2/s · 9553 pending ─╯
    let bottom = Line::from(vec![
        Span::styled(
            format!("╰{}", "─".repeat(inner.saturating_sub(flow.width() + 3))),
            dim,
        ),
        Span::raw(" "),
        Span::styled(format!("{rate}/s"), style::key()),
        Span::styled(" · ", dim),
        Span::styled(pending, if pending_hot { style::warn() } else { dim }),
        Span::styled(" pending ─╯", dim),
    ]);

    // One interior row per line of content, each padded out to the frame.
    let row = |body: Vec<Span<'static>>| {
        let used: usize = body.iter().map(|s| s.content.width()).sum();
        let mut spans = vec![Span::styled("│", dim), Span::raw(" ".repeat(PAD))];
        spans.extend(body);
        spans.push(Span::raw(" ".repeat(inner.saturating_sub(used + PAD))));
        spans.push(Span::styled("│", dim));
        Line::from(spans)
    };
    let blank = || row(vec![]);

    let lead = inner.saturating_sub(name.width() + PAD * 2) / 2;
    let mut lines = vec![
        top,
        row(vec![
            Span::raw(" ".repeat(lead)),
            Span::styled(name.to_owned(), style::title()),
        ]),
        blank(),
    ];
    lines.extend(chain_rows(m, step).into_iter().map(row));
    lines.push(blank());
    lines.push(bottom);
    lines
}

/// The numbers under the card: what the badge rounds off.
fn footer(v: &MonoVertexView) -> Line<'static> {
    let m = &v.monovertex;
    let metrics = v.metrics();
    Line::from(vec![
        Span::styled("rate/1m ", style::dim()),
        Span::raw(opt_rate(metrics.and_then(|x| x.rate.m1))),
        Span::styled("   rate/5m ", style::dim()),
        Span::raw(opt_rate(metrics.and_then(|x| x.rate.m5))),
        Span::styled("   ready ", style::dim()),
        Span::raw(format!("{}/{}", m.ready_replicas.unwrap_or(0), m.replicas)),
    ])
    .centered()
}

impl Model for MonoVertexPanel {
    fn on_enter(&mut self) -> Vec<WorkerMessage> {
        self.reload()
    }

    fn update(&mut self, ev: &AppEvent) -> (Option<Action>, Vec<WorkerMessage>) {
        match ev {
            AppEvent::Tick => (None, self.reload()),
            AppEvent::Frame => {
                self.spin = self.spin.wrapping_add(1);
                (None, vec![])
            }
            AppEvent::Worker(WorkerReply::MonoVertex(r)) => {
                match r.as_ref() {
                    Ok(v) => {
                        self.view = Some(v.clone());
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.clone()),
                }
                (None, vec![])
            }
            AppEvent::Key(k) => match pressed(k) {
                Some(KeyCode::Char('q')) => (Some(Action::Quit), vec![]),
                Some(KeyCode::Esc) => (Some(Action::Back), vec![]),
                Some(KeyCode::Char('r')) => (None, self.reload()),
                Some(KeyCode::Char('l') | KeyCode::Enter) => (
                    Some(Action::OpenLogs(WorkloadKey::from(&self.key), None)),
                    vec![],
                ),
                _ => (None, vec![]),
            },
            AppEvent::Mouse(_) | AppEvent::Worker(_) => (None, vec![]),
        }
    }

    fn view(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" monovertex {} ", self.key),
                style::title(),
            ))
            .border_style(style::dim());
        let room = block.inner(area);
        frame.render_widget(block, area);
        if let Some(e) = &self.error {
            frame.render_widget(Paragraph::new(format!("error: {e}")), room);
            return;
        }
        let Some(v) = &self.view else {
            waiting(frame, room, self.spin, "loading monovertex");
            return;
        };

        // Header, card, numbers and any warnings as one stack, floated in the
        // middle of the panel rather than pinned to the top-left corner.
        let name = v.monovertex.key.name.to_string();
        let mut lines = Self::header(v);
        lines.push(Line::default());
        let body = card(v, &name, usize::from(room.width));
        let width = body.iter().map(Line::width).max().unwrap_or(0);
        // The card's rows are built to a fixed width, so centre them by hand:
        // `Line::centered` would re-centre each row against the whole panel.
        let pad = " ".repeat(
            usize::from(room.width)
                .saturating_sub(width)
                .saturating_div(2),
        );
        for mut line in body {
            line.spans.insert(0, Span::raw(pad.clone()));
            lines.push(line);
        }
        lines.push(Line::default());
        lines.push(footer(v));
        for w in &v.warnings {
            lines.push(Line::styled(format!("warning: {w}"), style::warn()).centered());
        }

        let h = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let at = centre(room, room.width, h);
        frame.render_widget(Paragraph::new(lines), at);
    }
}
