use nfctl_core::model::{Health, PipelinePhase};
use ratatui::style::{Color, Modifier, Style};

pub fn phase(p: PipelinePhase) -> Style {
    let c = match p {
        PipelinePhase::Running => Color::Green,
        PipelinePhase::Paused | PipelinePhase::Pausing => Color::Yellow,
        PipelinePhase::Failed => Color::Red,
        PipelinePhase::Deleting | PipelinePhase::Unknown => Color::DarkGray,
    };
    Style::default().fg(c)
}

pub fn health(h: Option<Health>) -> Style {
    let c = match h {
        Some(Health::Healthy) => Color::Green,
        Some(Health::Warning) => Color::Yellow,
        Some(Health::Critical | Health::Unhealthy) => Color::Red,
        None | Some(Health::Inactive | Health::Deleting | Health::Unknown) => Color::DarkGray,
    };
    Style::default().fg(c)
}

pub fn title() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

pub fn key() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// Edge colours by tag combination; monochrome when colour is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    colour: bool,
}

impl Default for Palette {
    fn default() -> Self {
        Self { colour: true }
    }
}

impl Palette {
    const HUES: [Color; 6] = [
        Color::Cyan,
        Color::Magenta,
        Color::Yellow,
        Color::Green,
        Color::Blue,
        Color::Red,
    ];

    /// `no_color` is the CLI flag; `NO_COLOR` in the environment also disables.
    #[must_use]
    pub fn detect(no_color: bool) -> Self {
        let env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self {
            colour: !(no_color || env),
        }
    }

    #[must_use]
    pub fn monochrome() -> Self {
        Self { colour: false }
    }

    /// Untagged edges are dim; tagged edges take their slot's hue.
    #[must_use]
    pub fn edge(self, c: Option<nfctl_graph::layout::EdgeColour>) -> Style {
        match c {
            Some(c) if self.colour => {
                Style::default().fg(Self::HUES[usize::from(c.0) % Self::HUES.len()])
            }
            Some(_) => Style::default().add_modifier(Modifier::BOLD),
            None => dim(),
        }
    }
}
