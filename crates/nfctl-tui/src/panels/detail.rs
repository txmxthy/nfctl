use crossterm::event::KeyCode;
use nfctl_core::model::PipelineKey;
use nfctl_core::service::PipelineView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};

use crate::event::{Action, AppEvent};
use crate::panels::pressed;
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, cards, style};

#[derive(Debug)]
pub struct DetailPanel {
    key: PipelineKey,
    view: Option<PipelineView>,
    error: Option<String>,
    selected: usize,
}

impl DetailPanel {
    pub fn new(key: PipelineKey) -> Self {
        Self {
            key,
            view: None,
            error: None,
            selected: 0,
        }
    }

    fn selected_vertex(&self) -> Option<&nfctl_core::service::VertexView> {
        self.view
            .as_ref()
            .and_then(|v| v.vertices.get(self.selected))
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.view.as_ref().map_or(0, |v| v.vertices.len());
        if n == 0 {
            return;
        }
        let next = self.selected.saturating_add_signed(delta);
        self.selected = next.min(n - 1);
    }
}

impl Model for DetailPanel {
    fn on_enter(&mut self) -> Vec<WorkerMessage> {
        vec![WorkerMessage::LoadView(self.key.clone())]
    }

    fn update(&mut self, ev: &AppEvent) -> (Option<Action>, Vec<WorkerMessage>) {
        match ev {
            AppEvent::Tick => (None, self.on_enter()),
            AppEvent::Worker(WorkerReply::View(r)) => {
                match r.as_ref() {
                    Ok(v) if v.pipeline.key == self.key => {
                        self.view = Some(v.clone());
                        self.error = None;
                    }
                    Ok(_) => {}
                    Err(e) => self.error = Some(e.clone()),
                }
                (None, vec![])
            }
            AppEvent::Key(k) => match pressed(k) {
                Some(KeyCode::Char('q')) => (Some(Action::Quit), vec![]),
                Some(KeyCode::Esc | KeyCode::Char('h') | KeyCode::Backspace) => {
                    (Some(Action::Back), vec![])
                }
                Some(KeyCode::Char('j') | KeyCode::Down | KeyCode::Right) => {
                    self.move_by(1);
                    (None, vec![])
                }
                Some(KeyCode::Char('k') | KeyCode::Up | KeyCode::Left) => {
                    self.move_by(-1);
                    (None, vec![])
                }
                Some(KeyCode::Char('r')) => (None, self.on_enter()),
                Some(KeyCode::Char('l')) => {
                    (Some(Action::OpenLogs(self.key.clone(), None)), vec![])
                }
                Some(KeyCode::Enter) => {
                    let v = self.selected_vertex().map(|v| v.name.clone());
                    (Some(Action::OpenLogs(self.key.clone(), v)), vec![])
                }
                _ => (None, vec![]),
            },
            AppEvent::Worker(_) => (None, vec![]),
        }
    }

    fn view(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", self.key));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let Some(v) = &self.view else {
            let msg = self
                .error
                .clone()
                .map_or_else(|| "loading...".to_owned(), |e| format!("error: {e}"));
            frame.render_widget(Paragraph::new(msg), inner);
            return;
        };
        let p = &v.pipeline;
        let ranks = p.spec.topology.ranks();
        let tallest = ranks.iter().map(Vec::len).max().unwrap_or(1);
        let cards_h = u16::try_from(tallest).unwrap_or(1) * cards::CARD_HEIGHT;
        let [head, dag, edges, warn] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(cards_h),
            Constraint::Min(4),
            Constraint::Length(u16::try_from(v.warnings.len()).unwrap_or(0)),
        ])
        .areas(inner);

        let health = v.health.as_ref().map(|h| h.status);
        let line1 = Line::from(vec![
            Span::styled("phase ", style::dim()),
            Span::styled(p.status.phase.as_str(), style::phase(p.status.phase)),
            Span::styled("   health ", style::dim()),
            Span::styled(
                health.map_or("unknown", |h| h.as_str()),
                style::health(health),
            ),
            Span::styled("   desired ", style::dim()),
            Span::raw(p.spec.lifecycle.desired.as_str()),
            Span::styled("   isb ", style::dim()),
            Span::raw(p.spec.isb.to_string()),
        ]);
        let line2 = Line::styled(
            v.health
                .as_ref()
                .map(|h| h.message.clone())
                .or_else(|| p.status.message.clone())
                .unwrap_or_default(),
            style::dim(),
        );
        frame.render_widget(Paragraph::new(vec![line1, line2]), head);

        cards::render(
            frame,
            dag,
            v,
            self.selected_vertex().map(|x| x.name.as_str()),
        );

        frame.render_widget(edge_table(v), edges);

        let warnings: Vec<Line> = v
            .warnings
            .iter()
            .map(|w| {
                Line::styled(
                    format!("warning: {w}"),
                    style::phase(nfctl_core::model::PipelinePhase::Pausing),
                )
            })
            .collect();
        frame.render_widget(Paragraph::new(warnings), warn);
    }
}

fn edge_table(v: &PipelineView) -> Table<'_> {
    let rows = v.edges.iter().map(|e| {
        let usage = e
            .usage()
            .map_or_else(|| "-".to_owned(), |u| format!("{:.0}%", u * 100.0));
        let wm = e.watermark.as_ref().map_or_else(
            || "-".to_owned(),
            |w| {
                w.per_partition
                    .iter()
                    .flatten()
                    .max()
                    .and_then(|ts| ts.elapsed_until(v.at))
                    .map_or_else(|| "-".to_owned(), |d| format!("{}s ago", d.as_secs()))
            },
        );
        Row::new(vec![
            format!("{} -> {}", e.from, e.to),
            e.pending()
                .map_or_else(|| "-".to_owned(), |n| n.to_string()),
            usage,
            if e.is_full() {
                "FULL".to_owned()
            } else {
                String::new()
            },
            wm,
        ])
    });
    Table::new(
        rows,
        [
            Constraint::Length(30),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Min(8),
        ],
    )
    .header(Row::new(["EDGE", "PENDING", "USAGE", "", "WATERMARK"]).style(style::title()))
}
