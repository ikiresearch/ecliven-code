use ratatui::style::{Color, Modifier, Style};

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

    pub fn border_active() -> Style {
        Style::default().fg(Color::Cyan)
    }

    pub fn border_inactive() -> Style {
        Style::default().fg(Color::Gray)
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
