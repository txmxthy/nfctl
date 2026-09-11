use crossterm::event::KeyCode;
use nfctl_core::model::{PipelineKey, TagCondition, TagOperator, VertexName};
use nfctl_core::service::PipelineView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::cards::CardView;
use crate::event::{Action, AppEvent};
use crate::panels::pressed;
use crate::style::Palette;
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, cards, style};

#[derive(Debug)]
pub struct DetailPanel {
    key: PipelineKey,
    view: Option<PipelineView>,
    error: Option<String>,
    selected: usize,
    expand_shards: bool,
    /// Leftmost visible card column. A `Cell` so drawing can pull it to the selection.
    scroll: std::cell::Cell<usize>,
    /// Scroll so the selected card is visible on the next draw.
    follow: bool,
    palette: Palette,
    bundling: nfctl_graph::layout::Bundling,
}

impl DetailPanel {
    pub fn new(key: PipelineKey) -> Self {
        Self {
            key,
            view: None,
            error: None,
            selected: 0,
            expand_shards: false,
            scroll: std::cell::Cell::new(0),
            follow: false,
            palette: Palette::default(),
            // A row per colour, so a line can be followed by its colour.
            bundling: nfctl_graph::layout::Bundling::Ribbon,
        }
    }

    /// Draw long edges as a ribbon of adjacent rows instead of letting each
    /// take the row that costs it least.
    #[must_use]
    pub fn with_bundling(mut self, bundling: nfctl_graph::layout::Bundling) -> Self {
        self.bundling = bundling;
        self
    }

    #[must_use]
    pub fn with_palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
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
        self.follow = true;
    }

    /// The scroll to draw with: the stored one, pulled so the selection is visible.
    fn scroll_for(&self, cards: &CardView, width: u16) -> usize {
        let cols = cards.layout.col_x.len();
        let mut scroll = self.scroll.get().min(cols.saturating_sub(1));
        if self.follow
            && let Some(col) = self
                .selected_vertex()
                .and_then(|v| cards.column_of(v.name.as_str()))
        {
            if col < scroll {
                scroll = col;
            }
            while cards.last_visible(scroll, width) < col && scroll < col {
                scroll += 1;
            }
        }
        self.scroll.set(scroll);
        scroll
    }

    /// Rows the panel needs at `width` to show everything: the border, the
    /// two header rows, the flow box round the drawing, the edge table and
    /// the warnings. A shorter panel clips the drawing and the table.
    #[must_use]
    pub fn height_for(&self, width: u16) -> u16 {
        let Some(v) = &self.view else { return 5 };
        let cards = CardView::new(
            &v.pipeline.spec.topology,
            width.saturating_sub(4),
            self.expand_shards,
            self.bundling,
        );
        let table = table_rows(v.edges.len());
        let warn = u16::try_from(v.warnings.len()).unwrap_or(0);
        let inner = 2u16
            .saturating_add(cards.height().saturating_add(2))
            .saturating_add(table)
            .saturating_add(warn)
            // The table takes at most half the panel, so a graph with far more
            // edges than vertices needs the room to show them all.
            .max(table.saturating_mul(2));
        inner.saturating_add(2)
    }

    fn columns(&self) -> usize {
        self.view
            .as_ref()
            .map_or(0, |v| v.pipeline.spec.topology.ranks().len())
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
                Some(KeyCode::Char('j') | KeyCode::Down) => {
                    self.move_by(1);
                    (None, vec![])
                }
                Some(KeyCode::Char('k') | KeyCode::Up) => {
                    self.move_by(-1);
                    (None, vec![])
                }
                Some(KeyCode::Right) => {
                    self.scroll
                        .set((self.scroll.get() + 1).min(self.columns().saturating_sub(1)));
                    self.follow = false;
                    (None, vec![])
                }
                Some(KeyCode::Left) => {
                    self.scroll.set(self.scroll.get().saturating_sub(1));
                    self.follow = false;
                    (None, vec![])
                }
                Some(KeyCode::Char('x')) => {
                    self.expand_shards = !self.expand_shards;
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
        // The flow and the edges each sit in a box of their own, so the width
        // available to the drawing is the panel's less those borders.
        let room = inner.width.saturating_sub(2);
        let cards = CardView::new(&p.spec.topology, room, self.expand_shards, self.bundling);
        let scroll = self.scroll_for(&cards, room);
        let cards_h = cards.height();
        // The edge table sits at the bottom, at most half the panel; the flow
        // takes whatever is left above it. Both counting their borders.
        let table_h = table_rows(v.edges.len()).min(inner.height / 2).max(6);
        let [head, dag, edges, warn] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(
                cards_h
                    .saturating_add(2)
                    .min(inner.height.saturating_sub(table_h)),
            ),
            Constraint::Length(table_h),
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

        let flow = section(frame, dag, " flow ");
        cards::render(
            frame,
            centre(flow, cards.layout.width, cards_h),
            v,
            &cards,
            scroll,
            self.selected_vertex().map(|x| x.name.as_str()),
            self.palette,
        );

        let table = section(frame, edges, " edges ");
        frame.render_widget(edge_table(v, table.width, self.palette), table);

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

/// Rows the edge table wants: a row an edge, a header and its own border.
fn table_rows(edges: usize) -> u16 {
    u16::try_from(edges)
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .max(6)
}

/// Draw a titled box over `area` and return the room left inside it.
fn section(frame: &mut Frame, area: Rect, title: &'static str) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style::dim())
        .title(Span::styled(title, style::dim()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// A `w` by `h` drawing placed in the middle of `area`, or as much of it as
/// fits: a drawing narrower or shorter than its box sits in the middle of the
/// room rather than in a corner of it.
fn centre(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// Tags and the arrow between the two names take the edge's colour, the same
/// one it is drawn in above.
fn edge_table(v: &PipelineView, width: u16, palette: Palette) -> Table<'_> {
    use nfctl_graph::layout::{EdgeColour, ViewGraph, colours};
    const HEADER: [&str; 6] = ["EDGE", "TAGS", "PENDING", "USAGE", "", "WATERMARK"];
    let topology = &v.pipeline.spec.topology;
    // Expanded view: one view edge per topology edge, in the same order.
    let colour_of: std::collections::HashMap<(&VertexName, &VertexName), Option<EdgeColour>> =
        topology
            .edges()
            .iter()
            .zip(colours(&ViewGraph::expanded(topology)))
            .map(|(e, c)| ((&e.from, &e.to), c))
            .collect();
    let tags = |from: &VertexName, to: &VertexName| -> String {
        v.pipeline
            .spec
            .topology
            .edges()
            .iter()
            .find(|t| &t.from == from && &t.to == to)
            .and_then(|t| t.conditions.as_ref())
            .map(tag_label)
            .unwrap_or_default()
    };
    let cells: Vec<Vec<String>> = v
        .edges
        .iter()
        .map(|e| {
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
            vec![
                format!("{} -> {}", e.from, e.to),
                tags(&e.from, &e.to),
                e.pending()
                    .map_or_else(|| "-".to_owned(), |n| n.to_string()),
                usage,
                if e.is_full() {
                    "FULL".to_owned()
                } else {
                    String::new()
                },
                wm,
            ]
        })
        .collect();
    let widths = crate::table::fill(&HEADER, &cells, width, 2, &[0, 1]);
    let rows = v.edges.iter().zip(cells).map(|(e, mut c)| {
        let colour = colour_of.get(&(&e.from, &e.to)).copied().flatten();
        let ink = palette.edge(colour);
        let edge = Line::from(vec![
            Span::raw(e.from.to_string()),
            Span::styled(" -> ", ink),
            Span::raw(e.to.to_string()),
        ]);
        let tags = Line::styled(std::mem::take(&mut c[1]), ink);
        let rest: Vec<Cell> = c.into_iter().skip(2).map(Cell::from).collect();
        Row::new(
            [Cell::from(edge), Cell::from(tags)]
                .into_iter()
                .chain(rest)
                .collect::<Vec<_>>(),
        )
    });
    Table::new(rows, widths)
        .column_spacing(2)
        .header(Row::new(HEADER).style(style::title()))
}

fn tag_label(c: &TagCondition) -> String {
    match c.operator {
        TagOperator::Or => c.values.join(", "),
        TagOperator::And => c.values.join(" & "),
        TagOperator::Not => c
            .values
            .iter()
            .map(|v| format!("not {v}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}
