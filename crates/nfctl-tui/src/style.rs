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
