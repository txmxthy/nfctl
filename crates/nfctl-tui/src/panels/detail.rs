use crossterm::event::KeyCode;
use nfctl_core::model::{PipelineKey, TagCondition, TagOperator, VertexName, WorkloadKey};
use nfctl_core::service::PipelineView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::cards::CardView;
use crate::event::{Action, AppEvent};
use crate::panels::{centre, pressed, spiral, waiting};
use crate::style::Palette;
use crate::worker::{WorkerMessage, WorkerReply};
use crate::{Model, cards, style};
use unicode_width::UnicodeWidthStr as _;

/// Rows a page of the flow scrolls by.
const SCROLL_ROWS: u16 = 5;
/// Which box the arrow keys move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Flow,
    Edges,
}
/// The edge table never shrinks below its border and a row of edges.
const MIN_TABLE: u16 = 4;
/// Nor the flow below its border and one card.
const MIN_FLOW: u16 = 7;
/// Columns kept for the scroll bar down the right of a box: the bar itself,
/// and a blank one so the text does not run into it.
const GUTTER: u16 = 2;

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct DetailPanel {
    key: PipelineKey,
    view: Option<PipelineView>,
    error: Option<String>,
    selected: usize,
    expand_shards: bool,
    /// Leftmost visible card column. A `Cell` so drawing can pull it to the selection.
    scroll: std::cell::Cell<usize>,
    /// Topmost visible row of the drawing, for a flow taller than its box.
    vscroll: std::cell::Cell<u16>,
    /// Scroll so the selected card is visible on the next draw.
    follow: bool,
    /// Rows the edge table is given beyond what it would take by itself.
    split: i16,
    palette: Palette,
    /// Show how long the last load took, from `--timings`.
    timings: bool,
    /// Frame of the spinner, moved on by every tick.
    spin: std::cell::Cell<usize>,
    /// The box the arrows move. Clicking one makes it the focus.
    focus: Focus,
    /// Topmost visible row of the edge table.
    rows: usize,
    /// What that was clamped to when last drawn, so scrolling stops at the end.
    rows_at: std::cell::Cell<usize>,
    /// Where each box was drawn last, so a click knows what it hit.
    hit: std::cell::Cell<(Rect, Rect)>,
    /// The line between the boxes is being dragged.
    dragging: bool,
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
            vscroll: std::cell::Cell::new(0),
            follow: false,
            split: 0,
            palette: Palette::default(),
            timings: false,
            spin: std::cell::Cell::new(0),
            focus: Focus::default(),
            rows: 0,
            rows_at: std::cell::Cell::new(0),
            hit: std::cell::Cell::new((Rect::ZERO, Rect::ZERO)),
            dragging: false,
        }
    }

    #[must_use]
    pub fn with_palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// Print how long the last load took under the header.
    #[must_use]
    pub fn with_timings(mut self, on: bool) -> Self {
        self.timings = on;
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
        let cols = cards.col_x.len();
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

    /// Phase, health and where the pipeline sits, with a spiral while the
    /// numbers are still on their way: the shape arrives before they do, so
    /// nothing has been asked of a daemon yet when neither step took time.
    fn header(&self, v: &PipelineView) -> Line<'static> {
        let p = &v.pipeline;
        let health = v.health.as_ref().map(|h| h.status);
        let mut line = Line::from(vec![
            Span::styled("phase ", style::dim()),
            Span::styled(p.status.phase.as_str(), style::phase(p.status.phase.into())),
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
        // The health message reads as part of the health, so it sits with it
        // rather than on a line of its own.
        let said = v
            .health
            .as_ref()
            .map(|h| h.message.clone())
            .or_else(|| p.status.message.clone())
            .unwrap_or_default();
        if !said.is_empty() {
            line.push_span(Span::styled(format!("   {said}"), style::dim()));
        }
        if v.timings.connect.is_zero() && v.timings.numbers.is_zero() {
            line.push_span(Span::styled(
                format!("   {} numbers", spiral(self.spin.get())),
                style::key(),
            ));
        }
        line
    }

    /// The flow box and the drawing inside it, scrolled and with its bar.
    /// Returns where the box was drawn, for the click that focuses it.
    fn draw_flow(
        &self,
        frame: &mut Frame,
        area: Rect,
        v: &PipelineView,
        cards: &CardView,
        scroll: usize,
    ) -> Rect {
        let flow = section(frame, area, " flow ", self.focus == Focus::Flow);
        let selected = self.selected_vertex().map(|x| x.name.as_str());
        let dy = self.vscroll_for(cards, flow.height, selected);
        let cards_h = cards.height();
        // A drawing that fits sits in the middle of the box; one that does
        // not starts at the top and scrolls.
        let room = Rect {
            width: flow.width.saturating_sub(GUTTER),
            ..flow
        };
        let width = u16::try_from(cards.drawing.size().0).unwrap_or(u16::MAX);
        let at = centre(room, width, cards_h.min(room.height));
        let paint = cards::Paint {
            scroll: cards::Scroll {
                column: scroll,
                row: i32::from(dy),
            },
            palette: self.palette,
            spin: self.spin.get(),
        };
        cards::render(frame, at, v, cards, paint, selected);
        bar(
            frame,
            flow,
            usize::from(cards_h),
            usize::from(flow.height),
            usize::from(dy),
        );
        flow
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
        );
        let table = table_rows(v.edges.len());
        let warn = u16::try_from(v.warnings.len()).unwrap_or(0);
        let inner = u16::from(self.timings)
            .saturating_add(1)
            .saturating_add(cards.height().saturating_add(2))
            .saturating_add(table)
            .saturating_add(warn)
            // The table takes at most half the panel, so a graph with far more
            // edges than vertices needs the room to show them all.
            .max(table.saturating_mul(2));
        inner.saturating_add(2)
    }

    /// The row to draw the flow from: the stored one, clamped to the drawing
    /// and pulled so the selected card is in view.
    fn vscroll_for(&self, cards: &CardView, room: u16, selected: Option<&str>) -> u16 {
        let over = cards.height().saturating_sub(room);
        let mut top = self.vscroll.get().min(over);
        if self.follow
            && let Some(card) = selected
                .and_then(|s| nfctl_core::model::VertexName::new(s).ok())
                .and_then(|n| cards.graph.node_of(&n))
                .and_then(|n| cards.card(n))
        {
            let (y, h) = (
                u16::try_from(card.rect.y.max(0)).unwrap_or(0),
                u16::try_from(card.rect.h.max(0)).unwrap_or(0),
            );
            top = top.min(y).max((y + h).saturating_sub(room));
        }
        let top = top.min(over);
        self.vscroll.set(top);
        top
    }

    /// A click focuses the box it landed in; dragging between the two moves
    /// the line between them, a row at a time.
    fn mouse(&mut self, m: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let (flow, edges) = self.hit.get();
        let inside =
            |r: Rect| m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom();
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if inside(flow) {
                    self.focus = Focus::Flow;
                } else if inside(edges) {
                    self.focus = Focus::Edges;
                }
                self.dragging = !inside(flow) && !inside(edges) && m.row > flow.y;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                // The line sits between the two boxes: dragging it up gives
                // the table the rows, down gives them to the flow.
                let line = flow.bottom();
                let by = i32::from(line) - i32::from(m.row);
                self.split = self.split.saturating_add(i16::try_from(by).unwrap_or(0));
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            MouseEventKind::ScrollDown => self.scroll_by(1),
            MouseEventKind::ScrollUp => self.scroll_by(-1),
            _ => {}
        }
    }

    /// Move the focused box by `delta` rows.
    fn scroll_by(&mut self, delta: i32) {
        match self.focus {
            Focus::Flow => {
                let at = i64::from(self.vscroll.get()).saturating_add(i64::from(delta));
                self.vscroll
                    .set(u16::try_from(at.max(0)).unwrap_or(u16::MAX));
                self.follow = false;
            }
            Focus::Edges => {
                // From where it actually stopped, so holding a key at the
                // end does not build up a number it has to unwind.
                let at = i64::try_from(self.rows_at.get()).unwrap_or(0) + i64::from(delta);
                self.rows = usize::try_from(at.max(0)).unwrap_or(0);
            }
        }
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
            AppEvent::Frame => {
                self.spin.set(self.spin.get().wrapping_add(1));
                (None, vec![])
            }
            AppEvent::Worker(WorkerReply::Shape(v)) => {
                // Draw the graph now; the numbers replace it when they land.
                if v.pipeline.key == self.key && self.view.is_none() {
                    self.view = Some(*v.clone());
                    self.error = None;
                }
                (None, vec![])
            }
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
            AppEvent::Mouse(m) => {
                self.mouse(*m);
                (None, vec![])
            }
            AppEvent::Key(k) => match pressed(k) {
                Some(KeyCode::Char('q')) => (Some(Action::Quit), vec![]),
                Some(KeyCode::Esc | KeyCode::Char('h') | KeyCode::Backspace) => {
                    (Some(Action::Back), vec![])
                }
                // Tab walks the vertices; the arrows move whichever box has
                // the focus, so the flow and the table scroll apart.
                Some(KeyCode::Tab) => {
                    self.move_by(1);
                    (None, vec![])
                }
                Some(KeyCode::BackTab) => {
                    self.move_by(-1);
                    (None, vec![])
                }
                Some(KeyCode::Char('j') | KeyCode::Down) => {
                    self.scroll_by(1);
                    (None, vec![])
                }
                Some(KeyCode::Char('k') | KeyCode::Up) => {
                    self.scroll_by(-1);
                    (None, vec![])
                }
                Some(KeyCode::PageDown) => {
                    self.scroll_by(i32::from(SCROLL_ROWS));
                    (None, vec![])
                }
                Some(KeyCode::PageUp) => {
                    self.scroll_by(-i32::from(SCROLL_ROWS));
                    (None, vec![])
                }
                // Sideways only means something in the flow: the table has
                // no columns off screen, it repacks into blocks instead.
                Some(k @ (KeyCode::Left | KeyCode::Right)) if self.focus == Focus::Flow => {
                    let at = self.scroll.get();
                    self.scroll.set(if k == KeyCode::Right {
                        (at + 1).min(self.columns().saturating_sub(1))
                    } else {
                        at.saturating_sub(1)
                    });
                    self.follow = false;
                    (None, vec![])
                }
                Some(KeyCode::Char('+' | '=')) => {
                    self.split = self.split.saturating_add(1);
                    (None, vec![])
                }
                Some(KeyCode::Char('-')) => {
                    self.split = self.split.saturating_sub(1);
                    (None, vec![])
                }
                Some(KeyCode::Char('x')) => {
                    self.expand_shards = !self.expand_shards;
                    (None, vec![])
                }
                Some(KeyCode::Char('r')) => (None, self.on_enter()),
                Some(KeyCode::Char('l')) => (
                    Some(Action::OpenLogs(WorkloadKey::from(&self.key), None)),
                    vec![],
                ),
                Some(KeyCode::Enter) => {
                    let v = self.selected_vertex().map(|v| v.name.clone());
                    (
                        Some(Action::OpenLogs(WorkloadKey::from(&self.key), v)),
                        vec![],
                    )
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
            match &self.error {
                Some(e) => {
                    let msg = format!("error: {e}");
                    let w = u16::try_from(msg.width()).unwrap_or(0);
                    frame.render_widget(Paragraph::new(msg), centre(inner, w, 1));
                }
                None => waiting(frame, inner, self.spin.get(), "reading the pipeline"),
            }
            return;
        };
        let p = &v.pipeline;
        // The flow and the edges each sit in a box of their own, so the width
        // available to the drawing is the panel's less those borders.
        let room = inner.width.saturating_sub(2);
        let cards = CardView::new(&p.spec.topology, room, self.expand_shards);
        let scroll = self.scroll_for(&cards, room);
        let cards_h = cards.height();
        let warnings = u16::try_from(v.warnings.len()).unwrap_or(0);
        // One line: phase, health and what health says. Two only with
        // `--timings`, which adds what the load cost underneath.
        let head_h = if self.timings { 2 } else { 1 };
        let spare = inner.height.saturating_sub(warnings + head_h);
        // The flow asks for enough to hold the whole drawing, the table for a
        // row an edge, and neither goes below its floor. What is left over
        // when both fit goes to the table, which is the one that grows with
        // the pipeline. `+` and `-` move the line between them.
        let wants = cards_h.saturating_add(2);
        let table_h = spare
            .saturating_sub(wants.max(MIN_FLOW))
            .max(MIN_TABLE)
            .min(table_rows(v.edges.len()).max(MIN_TABLE))
            .saturating_add_signed(self.split)
            .clamp(
                MIN_TABLE.min(spare),
                spare.saturating_sub(MIN_FLOW).max(MIN_TABLE),
            );
        let [head, dag, edges, warn] = Layout::vertical([
            Constraint::Length(head_h),
            Constraint::Min(3),
            Constraint::Length(table_h),
            Constraint::Length(warnings),
        ])
        .areas(inner);

        let mut lines = vec![self.header(v)];
        if self.timings {
            let t = v.timings;
            lines.push(Line::styled(
                format!(
                    "loaded in {:.2}s  (spec {:.2}s  daemon connect {:.2}s  numbers {:.2}s)",
                    t.total().as_secs_f32(),
                    t.spec.as_secs_f32(),
                    t.connect.as_secs_f32(),
                    t.numbers.as_secs_f32(),
                ),
                style::key(),
            ));
        }
        frame.render_widget(Paragraph::new(lines), head);

        let flow = self.draw_flow(frame, dag, v, &cards, scroll);

        let box_ = section(frame, edges, " edges ", self.focus == Focus::Edges);
        // The bar has the rightmost column to itself, so nothing is written
        // under it. What is left is the table's.
        let fits = usize::from(box_.height.saturating_sub(1));
        let rows = self.rows.min(v.edges.len().saturating_sub(1));
        self.rows_at.set(rows);
        let table = Rect {
            width: box_.width.saturating_sub(GUTTER),
            ..box_
        };
        frame.render_widget(edge_table(v, table, self.palette, rows), table);
        bar(frame, box_, v.edges.len(), fits, rows);
        self.hit.set((flow, box_));

        let warnings: Vec<Line> = v
            .warnings
            .iter()
            .map(|w| {
                Line::styled(
                    format!("warning: {w}"),
                    style::phase(nfctl_core::model::WorkloadPhase::Pausing),
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

/// A bar down the right of `area` saying where in `total` rows the `shown`
/// on screen start. Takes the rightmost column, which the caller leaves
/// free, so nothing is written under it.
fn bar(frame: &mut Frame, area: Rect, total: usize, shown: usize, at: usize) {
    use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
    if total <= shown || area.height == 0 {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(shown)).position(at);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .track_style(style::dim()),
        area,
        &mut state,
    );
}

/// Draw a titled box over `area` and return the room left inside it. The one
/// with the focus is drawn brighter, since it is the one the arrows move.
fn section(frame: &mut Frame, area: Rect, title: &'static str, focused: bool) -> Rect {
    let ink = if focused {
        Style::default()
    } else {
        style::dim()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(ink)
        .title(Span::styled(title, ink));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Tags and the arrow between the two names take the edge's colour, the same
/// one it is drawn in above.
fn edge_cells(v: &PipelineView, palette: Palette) -> (Vec<Vec<String>>, Vec<Vec<Cell<'static>>>) {
    use nfctl_graph::layout::{ViewGraph, to_graph};
    let topology = &v.pipeline.spec.topology;
    // Expanded view: one view edge per topology edge, in the same order.
    let colour_of: std::collections::HashMap<(&VertexName, &VertexName), Option<orthodag::Colour>> =
        topology
            .edges()
            .iter()
            .zip(orthodag::colour::of(&to_graph(&ViewGraph::expanded(
                topology,
            ))))
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
    // The styled cells, in the same order.
    let styled = v
        .edges
        .iter()
        .zip(&mut cells.clone())
        .map(|(e, c)| {
            let ink = palette.edge(colour_of.get(&(&e.from, &e.to)).copied().flatten());
            let edge = Line::from(vec![
                Span::raw(e.from.to_string()),
                Span::styled(" -> ", ink),
                Span::raw(e.to.to_string()),
            ]);
            let tags = Line::styled(std::mem::take(&mut c[1]), ink);
            [Cell::from(edge), Cell::from(tags)]
                .into_iter()
                .chain(c.iter().skip(2).map(|x| Cell::from(x.clone())))
                .collect()
        })
        .collect();
    (cells, styled)
}

/// The edge table: one edge a row, from `from`, as many as fit.
fn edge_table(v: &PipelineView, area: Rect, palette: Palette, from: usize) -> Table<'static> {
    const HEADER: [&str; 6] = ["EDGE", "TAGS", "PENDING", "USAGE", "", "WATERMARK"];
    let (mut cells, mut built) = edge_cells(v, palette);
    // Scrolled: whole edges are dropped off the top.
    let from = from.min(cells.len().saturating_sub(1));
    cells.drain(..from);
    built.drain(..from);
    let deep = usize::from(area.height.saturating_sub(1)).max(1);
    built.truncate(deep);
    // Every column is spaced from the next, so what the columns themselves
    // have is the width less all of that; `fill` adds it back.
    let widths = crate::table::fill(&HEADER, &cells, area.width, 2, &[0, 1]);
    let rows: Vec<Row> = built.into_iter().map(Row::new).collect();
    let header = Row::new(HEADER.map(Cell::from).to_vec()).style(style::title());
    Table::new(rows, widths).column_spacing(2).header(header)
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
