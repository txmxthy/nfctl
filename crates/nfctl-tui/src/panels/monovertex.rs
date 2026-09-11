//! A `MonoVertex`'s detail. There is no topology to draw: one vertex, no
//! edges, no buffers and no watermarks, so the panel is the header a pipeline
//! shows plus the one row of numbers its daemon answers with.

use crossterm::event::KeyCode;
use nfctl_core::model::{MonoVertexKey, WorkloadKey};
use nfctl_core::service::MonoVertexView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};

use crate::event::{Action, AppEvent};
use crate::panels::{pressed, waiting};
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, style};

const HEADER: [&str; 5] = ["MONOVERTEX", "RATE/1m", "RATE/5m", "PENDING", "READY"];

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

    fn header(v: &MonoVertexView) -> Line<'static> {
        let m = &v.monovertex;
        let health = v.health.as_ref().map(|h| h.status);
        let mut line = Line::from(vec![
            Span::styled("phase ", style::dim()),
            Span::styled(m.phase.as_str(), style::phase(m.phase.into())),
            Span::styled("   health ", style::dim()),
            Span::styled(
                health.map_or("unknown", |h| h.as_str()),
                style::health(health),
            ),
            Span::styled("   desired ", style::dim()),
            Span::raw(m.desired.as_str()),
            Span::styled("   replicas ", style::dim()),
            Span::raw(format!("{}/{}", m.ready_replicas.unwrap_or(0), m.replicas)),
        ]);
        let said = v
            .health
            .as_ref()
            .map(|h| h.message.clone())
            .or_else(|| m.message.clone())
            .unwrap_or_default();
        if !said.is_empty() {
            line.push_span(Span::styled(format!("   {said}"), style::dim()));
        }
        line
    }
}

fn opt_rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |r| format!("{r:.1}"))
}

fn opt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
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
        let warnings = u16::try_from(v.warnings.len()).unwrap_or(0);
        let [head, numbers, notes] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Length(warnings),
        ])
        .areas(room);
        frame.render_widget(Paragraph::new(MonoVertexPanel::header(v)), head);

        let m = &v.monovertex;
        let metrics = v.metrics();
        let cells = vec![
            m.key.name.to_string(),
            opt_rate(metrics.and_then(|x| x.rate.m1)),
            opt_rate(metrics.and_then(|x| x.rate.m5)),
            opt_i64(metrics.and_then(|x| x.pending.default.or(x.pending.m1))),
            format!("{}/{}", m.ready_replicas.unwrap_or(0), m.replicas),
        ];
        let widths = crate::table::fit(&HEADER, std::slice::from_ref(&cells));
        let table = Table::new(std::iter::once(Row::new(cells)), widths)
            .column_spacing(2)
            .header(Row::new(HEADER).style(style::title()))
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(table, numbers);

        let lines: Vec<Line> = v
            .warnings
            .iter()
            .map(|w| Line::styled(format!("warning: {w}"), style::dim()))
            .collect();
        frame.render_widget(Paragraph::new(lines), notes);
    }
}
