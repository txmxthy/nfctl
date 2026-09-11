use crossterm::event::KeyCode;
use nfctl_core::model::{Namespace, Workload};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Row, Table, TableState};

use crate::event::{Action, AppEvent};
use crate::panels::{centre, pressed, waiting};
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, style};
use unicode_width::UnicodeWidthStr as _;

/// Blank columns between two columns of the table.
const SPACING: u16 = 2;
/// The panel is never narrower than its own title.
const TITLE_ROOM: u16 = 30;

#[derive(Debug)]
pub struct WorkloadsPanel {
    ns: Option<Namespace>,
    items: Vec<Workload>,
    state: TableState,
    error: Option<String>,
    loading: bool,
    /// Show how long the last list took, from `--timings`.
    timings: bool,
    /// Frame of the spinner.
    spin: usize,
    /// Where these are being read from: a context, or a fixture.
    source: String,
    took: Option<std::time::Duration>,
}

const HEADER: [&str; 7] = [
    "NAMESPACE",
    "KIND",
    "NAME",
    "PHASE",
    "VERTICES",
    "DESIRED",
    "MESSAGE",
];

impl WorkloadsPanel {
    pub fn new(ns: Option<Namespace>) -> Self {
        Self {
            ns,
            items: Vec::new(),
            state: TableState::default(),
            error: None,
            loading: true,
            timings: false,
            spin: 0,
            source: String::new(),
            took: None,
        }
    }

    /// Show how long the last list took in the title.
    #[must_use]
    pub fn with_timings(mut self, on: bool) -> Self {
        self.timings = on;
        self
    }

    /// Name what the list is being read from, so a screenshot says which
    /// cluster it came from.
    #[must_use]
    pub fn from(mut self, source: String) -> Self {
        self.source = source;
        self
    }

    fn selected(&self) -> Option<&Workload> {
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
        vec![WorkerMessage::LoadWorkloads(self.ns.clone())]
    }
}

impl Model for WorkloadsPanel {
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
            AppEvent::Worker(WorkerReply::Workloads(r, took)) => {
                self.loading = false;
                self.took = Some(*took);
                match r {
                    Ok(items) => {
                        // Keep the selection on the same workload across refreshes.
                        let keep = self.selected().map(Workload::key);
                        self.items.clone_from(items);
                        let idx = keep.and_then(|k| self.items.iter().position(|w| w.key() == k));
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
                Some(KeyCode::Enter) => {
                    (self.selected().map(|w| Action::OpenDetail(w.key())), vec![])
                }
                Some(KeyCode::Char('l')) => (
                    self.selected().map(|w| Action::OpenLogs(w.key(), None)),
                    vec![],
                ),
                _ => (None, vec![]),
            },
            AppEvent::Mouse(_) | AppEvent::Worker(_) => (None, vec![]),
        }
    }

    fn view(&self, frame: &mut Frame, area: Rect) {
        // The terminal keeps a border of its own, dim, with the title on it;
        // what the panel holds sits in a brighter box floating inside.
        let mut title = match (&self.ns, self.source.as_str()) {
            (Some(ns), "") => format!(" workloads in {ns} "),
            (Some(ns), from) => format!(" {from} · workloads in {ns} "),
            (None, "") => " workloads (all namespaces) ".to_owned(),
            (None, from) => format!(" {from} · workloads (all namespaces) "),
        };
        if let (true, Some(took)) = (self.timings, self.took) {
            title = format!("{title}· listed in {:.2}s ", took.as_secs_f32());
        }
        let chrome = Block::default()
            .borders(Borders::ALL)
            .border_style(style::dim())
            .title(Span::styled(title, style::title()));
        let room = chrome.inner(area);
        frame.render_widget(chrome, area);
        let block = Block::default().borders(Borders::ALL);
        if let Some(e) = &self.error {
            let msg = format!("error: {e}");
            let w = u16::try_from(msg.width()).unwrap_or(0) + 2;
            let at = centre(room, w.max(TITLE_ROOM), 3);
            frame.render_widget(ratatui::widgets::Paragraph::new(msg).block(block), at);
            return;
        }
        if self.loading {
            waiting(frame, room, self.spin, "listing workloads");
            return;
        }
        let header = Row::new(HEADER).style(style::title());
        let cells: Vec<Vec<String>> = self
            .items
            .iter()
            .map(|w| {
                vec![
                    w.namespace().to_string(),
                    w.kind().as_str().to_owned(),
                    w.name().to_string(),
                    w.phase().as_str().to_owned(),
                    w.vertices().to_string(),
                    w.desired().as_str().to_owned(),
                    w.message().unwrap_or_default().to_owned(),
                ]
            })
            .collect();
        let widths = crate::table::fit(&HEADER, &cells);
        // The panel is as big as the table it holds and floats in the middle
        // of the terminal; the last column is `Min`, so it is measured at the
        // width its content wants rather than stretched to the screen.
        let at = centre(
            room,
            crate::table::width(&widths, SPACING) + 2,
            u16::try_from(self.items.len())
                .unwrap_or(u16::MAX)
                .saturating_add(3),
        );
        let rows = self.items.iter().zip(cells).map(|(w, c)| {
            let mut c = c.into_iter();
            Row::new(vec![
                Line::raw(c.next().unwrap_or_default()),
                Line::styled(c.next().unwrap_or_default(), style::dim()),
                Line::raw(c.next().unwrap_or_default()),
                Line::styled(c.next().unwrap_or_default(), style::phase(w.phase())),
                Line::raw(c.next().unwrap_or_default()),
                Line::raw(c.next().unwrap_or_default()),
                Line::styled(c.next().unwrap_or_default(), style::dim()),
            ])
        });
        let table = Table::new(rows, widths)
            .column_spacing(SPACING)
            .header(header)
            .block(block)
            .row_highlight_style(style::selected());
        let mut state = self.state;
        frame.render_stateful_widget(table, at, &mut state);
    }
}
