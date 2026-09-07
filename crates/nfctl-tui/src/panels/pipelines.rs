use crossterm::event::KeyCode;
use nfctl_core::model::{Namespace, Pipeline};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Row, Table, TableState};

use crate::event::{Action, AppEvent};
use crate::panels::pressed;
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, style};

#[derive(Debug)]
pub struct PipelinesPanel {
    ns: Option<Namespace>,
    items: Vec<Pipeline>,
    state: TableState,
    error: Option<String>,
    loading: bool,
}

const HEADER: [&str; 6] = [
    "NAMESPACE",
    "NAME",
    "PHASE",
    "VERTICES",
    "DESIRED",
    "MESSAGE",
];

impl PipelinesPanel {
    pub fn new(ns: Option<Namespace>) -> Self {
        Self {
            ns,
            items: Vec::new(),
            state: TableState::default(),
            error: None,
            loading: true,
        }
    }

    fn selected(&self) -> Option<&Pipeline> {
        self.state.selected().and_then(|i| self.items.get(i))
    }

    fn move_by(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.state.select(None);
            return;
        }
        let cur = self.state.selected().unwrap_or(0);
        let next = cur.saturating_add_signed(delta).min(self.items.len() - 1);
        self.state.select(Some(next));
    }

    fn reload(&self) -> Vec<WorkerMessage> {
        vec![WorkerMessage::LoadPipelines(self.ns.clone())]
    }
}

impl Model for PipelinesPanel {
    fn on_enter(&mut self) -> Vec<WorkerMessage> {
        self.reload()
    }

    fn update(&mut self, ev: &AppEvent) -> (Option<Action>, Vec<WorkerMessage>) {
        match ev {
            AppEvent::Tick => (None, self.reload()),
            AppEvent::Worker(WorkerReply::Pipelines(r)) => {
                self.loading = false;
                match r {
                    Ok(items) => {
                        // Keep the selection on the same pipeline across refreshes.
                        let keep = self.selected().map(|p| p.key.clone());
                        self.items.clone_from(items);
                        let idx = keep.and_then(|k| self.items.iter().position(|p| p.key == k));
                        self.state
                            .select(idx.or_else(|| (!self.items.is_empty()).then_some(0)));
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.clone()),
                }
                (None, vec![])
            }
            AppEvent::Key(k) => match pressed(k) {
                Some(KeyCode::Char('q')) => (Some(Action::Quit), vec![]),
                Some(KeyCode::Char('j') | KeyCode::Down) => {
                    self.move_by(1);
                    (None, vec![])
                }
                Some(KeyCode::Char('k') | KeyCode::Up) => {
                    self.move_by(-1);
                    (None, vec![])
                }
                Some(KeyCode::Char('r')) => (None, self.reload()),
                Some(KeyCode::Enter) => (
                    self.selected().map(|p| Action::OpenDetail(p.key.clone())),
                    vec![],
                ),
                Some(KeyCode::Char('l')) => (
                    self.selected()
                        .map(|p| Action::OpenLogs(p.key.clone(), None)),
                    vec![],
                ),
                _ => (None, vec![]),
            },
            AppEvent::Worker(_) => (None, vec![]),
        }
    }

    fn view(&self, frame: &mut Frame, area: Rect) {
        let title = match &self.ns {
            Some(ns) => format!(" pipelines in {ns} "),
            None => " pipelines (all namespaces) ".to_owned(),
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        if let Some(e) = &self.error {
            frame.render_widget(
                ratatui::widgets::Paragraph::new(format!("error: {e}")).block(block),
                area,
            );
            return;
        }
        if self.loading {
            frame.render_widget(
                ratatui::widgets::Paragraph::new("loading...").block(block),
                area,
            );
            return;
        }
        let header = Row::new(HEADER).style(style::title());
        let cells: Vec<Vec<String>> = self
            .items
            .iter()
            .map(|p| {
                vec![
                    p.key.namespace.to_string(),
                    p.key.name.to_string(),
                    p.status.phase.as_str().to_owned(),
                    p.status.counts.total.to_string(),
                    p.spec.lifecycle.desired.as_str().to_owned(),
                    p.status.message.clone().unwrap_or_default(),
                ]
            })
            .collect();
        let widths = crate::table::fit(&HEADER, &cells);
        let rows = self.items.iter().zip(cells).map(|(p, c)| {
            let mut c = c.into_iter();
            Row::new(vec![
                Line::raw(c.next().unwrap_or_default()),
                Line::raw(c.next().unwrap_or_default()),
                Line::styled(c.next().unwrap_or_default(), style::phase(p.status.phase)),
                Line::raw(c.next().unwrap_or_default()),
                Line::raw(c.next().unwrap_or_default()),
                Line::styled(c.next().unwrap_or_default(), style::dim()),
            ])
        });
        let table = Table::new(rows, widths)
            .column_spacing(2)
            .header(header)
            .block(block)
            .row_highlight_style(style::selected());
        let mut state = self.state;
        frame.render_stateful_widget(table, area, &mut state);
    }
}
