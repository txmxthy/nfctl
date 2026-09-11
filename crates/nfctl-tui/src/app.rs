use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use futures::StreamExt;
use nfctl_core::model::Namespace;
use nfctl_core::ports::ClusterPort;
use nfctl_core::service::PipelineService;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;

use crate::event::{Action, AppEvent};
use crate::panels::detail::DetailPanel;
use crate::panels::logs::LogsPanel;
use crate::panels::pipelines::PipelinesPanel;
use crate::style::Palette;
use crate::worker::{Worker, WorkerMessage};
use crate::{Model, style};

enum Panel {
    Pipelines(Box<PipelinesPanel>),
    Detail(Box<DetailPanel>),
    Logs(Box<LogsPanel>),
}

impl Panel {
    fn model(&mut self) -> &mut dyn Model {
        match self {
            Panel::Pipelines(p) => p.as_mut(),
            Panel::Detail(p) => p.as_mut(),
            Panel::Logs(p) => p.as_mut(),
        }
    }

    fn model_ref(&self) -> &dyn Model {
        match self {
            Panel::Pipelines(p) => p.as_ref(),
            Panel::Detail(p) => p.as_ref(),
            Panel::Logs(p) => p.as_ref(),
        }
    }

    fn keys(&self) -> &'static str {
        match self {
            Panel::Pipelines(_) => "j/k move  enter detail  l logs  r refresh  q quit",
            Panel::Detail(_) => {
                "tab vertex  ↑/↓ scroll pane  click focus  +/- split  x shards  enter logs  l pipeline logs  esc back  q quit"
            }
            Panel::Logs(_) => "j/k scroll  G follow  esc back  q quit",
        }
    }
}

/// The UI task's whole state: a stack of panels and the worker channel.
struct App {
    stack: Vec<Panel>,
    ns: Option<Namespace>,
    palette: Palette,
    /// Show how long each load took, from `--timings`.
    timings: bool,
    tx: mpsc::Sender<WorkerMessage>,
}

impl App {
    fn top(&mut self) -> &mut Panel {
        self.stack
            .last_mut()
            .unwrap_or_else(|| unreachable!("the stack is never empty"))
    }

    async fn send(&self, msgs: Vec<WorkerMessage>) {
        for m in msgs {
            let _ = self.tx.send(m).await;
        }
    }

    async fn push(&mut self, mut panel: Panel) {
        let msgs = panel.model().on_enter();
        self.stack.push(panel);
        self.send(msgs).await;
    }

    /// Returns `false` when the app should exit.
    async fn handle(&mut self, ev: AppEvent) -> bool {
        let (action, msgs) = self.top().model().update(&ev);
        self.send(msgs).await;
        match action {
            None => true,
            Some(Action::Quit) => false,
            Some(Action::OpenDetail(key)) => {
                let panel = DetailPanel::new(key)
                    .with_palette(self.palette)
                    .with_timings(self.timings);
                self.push(Panel::Detail(Box::new(panel))).await;
                true
            }
            Some(Action::OpenLogs(key, vertex)) => {
                self.push(Panel::Logs(Box::new(LogsPanel::new(key, vertex))))
                    .await;
                true
            }
            Some(Action::Back) => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                    let msgs = self.top().model().on_enter();
                    self.send(msgs).await;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let [body, footer] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());
        if let Some(p) = self.stack.last() {
            p.model_ref().view(frame, body);
            let ns = self
                .ns
                .as_ref()
                .map_or_else(|| "all namespaces".to_owned(), ToString::to_string);
            let line = Line::from(vec![
                Span::styled(" nfctl ", style::key()),
                Span::styled(ns, style::dim()),
                Span::raw("   "),
                Span::styled(p.keys(), style::dim()),
            ]);
            frame.render_widget(Paragraph::new(line), footer);
        }
    }
}

/// How the UI is set up for a run.
#[derive(Debug, Clone)]
pub struct Options {
    /// The namespace to list, or every one.
    pub ns: Option<Namespace>,
    /// How often the cluster is asked again.
    pub tick: Duration,
    pub palette: Palette,
    /// Say how long each load took, from `--timings`.
    pub timings: bool,
    /// Where the pipelines are being read from: a context, or a fixture.
    pub source: String,
}

/// How often the screen is repainted for something that is moving.
const FRAME: Duration = Duration::from_millis(120);

/// Run the UI until the user quits. Takes over the terminal; restores it on exit.
pub async fn run(
    cluster: Arc<dyn ClusterPort>,
    service: PipelineService,
    opts: Options,
) -> std::io::Result<()> {
    let Options {
        ns,
        tick,
        palette,
        timings,
        source,
    } = opts;
    let (tx, mut replies) = Worker::spawn(cluster, service);
    let mut app = App {
        stack: Vec::new(),
        ns: ns.clone(),
        palette,
        timings,
        tx,
    };
    app.push(Panel::Pipelines(Box::new(
        PipelinesPanel::new(ns).with_timings(timings).from(source),
    )))
    .await;

    let mut terminal = ratatui::init();
    // Clicking a box focuses it and the divider can be dragged, so the
    // terminal has to report the mouse. Shift-Tab needs the enhanced
    // protocol on terminals that have one; both are undone on the way out.
    let _ = crossterm::execute!(
        std::io::stdout(),
        EnableMouseCapture,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
    );
    let mut keys = EventStream::new();
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Fast enough for a spinner to turn, and it fetches nothing.
    let mut frames = tokio::time::interval(FRAME);
    frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        if let Err(e) = terminal.draw(|f| app.draw(f)) {
            break Err(e);
        }
        let ev = tokio::select! {
            k = keys.next() => match k {
                Some(Ok(Event::Key(k))) => AppEvent::Key(k),
                Some(Ok(Event::Mouse(m))) => AppEvent::Mouse(m),
                Some(Ok(_)) => continue,
                Some(Err(e)) => break Err(e),
                None => break Ok(()),
            },
            Some(r) = replies.recv() => AppEvent::Worker(r),
            _ = ticker.tick() => AppEvent::Tick,
            _ = frames.tick() => AppEvent::Frame,
        };
        if !app.handle(ev).await {
            break Ok(());
        }
    };
    let _ = crossterm::execute!(
        std::io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture
    );
    ratatui::restore();
    result
}
