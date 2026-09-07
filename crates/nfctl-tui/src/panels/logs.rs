use std::collections::VecDeque;

use crossterm::event::KeyCode;
use nfctl_core::model::{PipelineKey, TaggedLine, VertexName};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::event::{Action, AppEvent};
use crate::panels::pressed;
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, style};

const CAPACITY: usize = 5000;

#[derive(Debug)]
pub struct LogsPanel {
    key: PipelineKey,
    vertex: Option<VertexName>,
    lines: VecDeque<TaggedLine>,
    /// `None` = follow the tail; `Some(n)` = pinned so the top line is index n.
    scroll: Option<usize>,
    error: Option<String>,
}

impl LogsPanel {
    pub fn new(key: PipelineKey, vertex: Option<VertexName>) -> Self {
        Self {
            key,
            vertex,
            lines: VecDeque::new(),
            scroll: None,
            error: None,
        }
    }

    fn scroll_by(&mut self, delta: isize, page: usize) {
        let max = self.lines.len().saturating_sub(page);
        let cur = self.scroll.unwrap_or(max);
        let next = cur.saturating_add_signed(delta).min(max);
        self.scroll = if next >= max { None } else { Some(next) };
    }
}

impl Model for LogsPanel {
    fn on_enter(&mut self) -> Vec<WorkerMessage> {
        vec![WorkerMessage::StartLogs(
            self.key.clone(),
            self.vertex.clone(),
        )]
    }

    fn update(&mut self, ev: &AppEvent) -> (Option<Action>, Vec<WorkerMessage>) {
        match ev {
            AppEvent::Worker(WorkerReply::Log(l)) => {
                if self.lines.len() == CAPACITY {
                    self.lines.pop_front();
                    if let Some(s) = &mut self.scroll {
                        *s = s.saturating_sub(1);
                    }
                }
                self.lines.push_back(l.clone());
                (None, vec![])
            }
            AppEvent::Worker(WorkerReply::LogsFailed(e)) => {
                self.error = Some(e.clone());
                (None, vec![])
            }
            AppEvent::Key(k) => match pressed(k) {
                Some(KeyCode::Char('q')) => (Some(Action::Quit), vec![WorkerMessage::StopLogs]),
                Some(KeyCode::Esc | KeyCode::Char('h') | KeyCode::Backspace) => {
                    (Some(Action::Back), vec![WorkerMessage::StopLogs])
                }
                Some(KeyCode::Char('j') | KeyCode::Down) => {
                    self.scroll_by(1, 1);
                    (None, vec![])
                }
                Some(KeyCode::Char('k') | KeyCode::Up) => {
                    self.scroll_by(-1, 1);
                    (None, vec![])
                }
                Some(KeyCode::PageDown) => {
                    self.scroll_by(20, 1);
                    (None, vec![])
                }
                Some(KeyCode::PageUp) => {
                    self.scroll_by(-20, 1);
                    (None, vec![])
                }
                Some(KeyCode::Char('G') | KeyCode::End) => {
                    self.scroll = None;
                    (None, vec![])
                }
                _ => (None, vec![]),
            },
            AppEvent::Tick | AppEvent::Worker(_) => (None, vec![]),
        }
    }

    fn view(&self, frame: &mut Frame, area: Rect) {
        let what = match &self.vertex {
            Some(v) => format!(" logs {}/{} ", self.key, v),
            None => format!(" logs {} ", self.key),
        };
        let mode = if self.scroll.is_none() {
            " following "
        } else {
            " paused (G to follow) "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(what)
            .title_top(Line::styled(mode, style::dim()).right_aligned());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if let Some(e) = &self.error {
            frame.render_widget(Paragraph::new(format!("error: {e}")), inner);
            return;
        }
        let page = usize::from(inner.height.max(1));
        let start = self
            .scroll
            .unwrap_or_else(|| self.lines.len().saturating_sub(page));
        let text: Vec<Line> = self
            .lines
            .iter()
            .skip(start)
            .take(page)
            .map(|l| {
                Line::from(vec![
                    Span::styled(format!("{}/{} ", l.pod, l.container), style::key()),
                    Span::raw(l.line.text.clone()),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(text), inner);
    }
}
