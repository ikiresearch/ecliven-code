use ratatui::style::{Color, Modifier, Style};

use crate::swarm::domain::SwarmDomain;

pub struct Theme;

impl Theme {
    pub fn brand() -> Style {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }

    pub fn header_bar() -> Style {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    }

    pub fn update_banner() -> Style {
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_bar() -> Style {
        Style::default().bg(Color::Black).fg(Color::Gray)
    }

    pub fn tab_active_chat() -> Style {
        Style::default()
            .bg(Color::Cyan)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_code() -> Style {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_security() -> Style {
        Style::default()
            .bg(Color::Magenta)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_inactive() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub fn border_active() -> Style {
        Style::default().fg(Color::Cyan)
    }

    pub fn border_code() -> Style {
        Style::default().fg(Color::LightBlue)
    }

    pub fn border_security() -> Style {
        Style::default().fg(Color::Magenta)
    }

    pub fn border_inactive() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub fn swarm_badge(domain: SwarmDomain) -> Style {
        match domain {
            SwarmDomain::Code => Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
            SwarmDomain::Security(_) => Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn status_running() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_done() -> Style {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }

    pub fn user_label() -> Style {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }

    pub fn assistant_label() -> Style {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tool_label() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    pub fn system_text() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub fn error_text() -> Style {
        Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD)
    }

    pub fn diff_add() -> Style {
        Style::default().fg(Color::Green)
    }

    pub fn diff_del() -> Style {
        Style::default().fg(Color::Red)
    }

    pub fn diff_hunk() -> Style {
        Style::default().fg(Color::Cyan)
    }

    pub fn modal_border() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    pub fn footer_hint() -> Style {
        Style::default().fg(Color::DarkGray)
    }
}
