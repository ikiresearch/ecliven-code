use ratatui::style::{Color, Modifier, Style};

use crate::swarm::domain::SwarmDomain;

pub struct Theme;

impl Theme {
    pub const SOLAR_GOLD: Color = Color::Rgb(255, 212, 154);
    pub const CORONA_AMBER: Color = Color::Rgb(255, 179, 92);
    pub const STELLAR_BLUE: Color = Color::Rgb(130, 198, 255);
    pub const SHIELD_ROSE: Color = Color::Rgb(255, 130, 176);
    pub const TEXT_PRIMARY: Color = Color::Rgb(235, 237, 245);
    pub const TEXT_MUTED: Color = Color::Rgb(132, 138, 156);
    pub const TEXT_DIM: Color = Color::Rgb(88, 94, 112);
    pub const SURFACE_DARK: Color = Color::Rgb(22, 24, 32);
    pub const BORDER_SUBTLE: Color = Color::Rgb(62, 67, 82);
    pub const DIFF_GREEN: Color = Color::Rgb(134, 239, 172);
    pub const DIFF_RED: Color = Color::Rgb(252, 129, 129);

    pub fn brand() -> Style {
        Style::default()
            .fg(Self::SOLAR_GOLD)
            .add_modifier(Modifier::BOLD)
    }

    pub fn header_bar() -> Style {
        Style::default().bg(Self::SURFACE_DARK).fg(Self::TEXT_MUTED)
    }

    pub fn update_banner() -> Style {
        Style::default()
            .bg(Self::SOLAR_GOLD)
            .fg(Color::Rgb(18, 18, 22))
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_bar() -> Style {
        Style::default().fg(Self::TEXT_MUTED)
    }

    pub fn tab_active_chat() -> Style {
        Style::default()
            .fg(Self::SOLAR_GOLD)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_code() -> Style {
        Style::default()
            .fg(Self::STELLAR_BLUE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_security() -> Style {
        Style::default()
            .fg(Self::SHIELD_ROSE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_inactive() -> Style {
        Style::default().fg(Self::TEXT_DIM)
    }

    pub fn border_active() -> Style {
        Style::default().fg(Self::SOLAR_GOLD)
    }

    pub fn border_code() -> Style {
        Style::default().fg(Self::STELLAR_BLUE)
    }

    pub fn border_security() -> Style {
        Style::default().fg(Self::SHIELD_ROSE)
    }

    pub fn border_inactive() -> Style {
        Style::default().fg(Self::BORDER_SUBTLE)
    }

    pub fn swarm_badge(domain: SwarmDomain) -> Style {
        match domain {
            SwarmDomain::Code => Style::default()
                .fg(Self::STELLAR_BLUE)
                .add_modifier(Modifier::BOLD),
            SwarmDomain::Security(_) => Style::default()
                .fg(Self::SHIELD_ROSE)
                .add_modifier(Modifier::BOLD),
            SwarmDomain::Competitive | SwarmDomain::Collaborative => Style::default()
                .fg(Self::SOLAR_GOLD)
                .add_modifier(Modifier::BOLD),
            SwarmDomain::RedEclipse => Style::default()
                .fg(Self::DIFF_RED)
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn status_running() -> Style {
        Style::default()
            .fg(Self::CORONA_AMBER)
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_done() -> Style {
        Style::default()
            .fg(Self::DIFF_GREEN)
            .add_modifier(Modifier::BOLD)
    }

    pub fn user_label() -> Style {
        Style::default()
            .fg(Self::SOLAR_GOLD)
            .add_modifier(Modifier::BOLD)
    }

    pub fn text_primary() -> Style {
        Style::default().fg(Self::TEXT_PRIMARY)
    }

    pub fn assistant_label() -> Style {
        Style::default()
            .fg(Self::CORONA_AMBER)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tool_label() -> Style {
        Style::default()
            .fg(Self::SOLAR_GOLD)
            .add_modifier(Modifier::BOLD)
    }

    pub fn system_text() -> Style {
        Style::default().fg(Self::TEXT_MUTED)
    }

    pub fn placeholder() -> Style {
        Style::default().fg(Self::TEXT_DIM)
    }

    pub fn error_text() -> Style {
        Style::default()
            .fg(Self::DIFF_RED)
            .add_modifier(Modifier::BOLD)
    }

    pub fn diff_add() -> Style {
        Style::default().fg(Self::DIFF_GREEN)
    }

    pub fn diff_del() -> Style {
        Style::default().fg(Self::DIFF_RED)
    }

    pub fn diff_hunk() -> Style {
        Style::default().fg(Self::STELLAR_BLUE)
    }

    pub fn modal_border() -> Style {
        Style::default()
            .fg(Self::SOLAR_GOLD)
            .add_modifier(Modifier::BOLD)
    }

    pub fn footer_hint() -> Style {
        Style::default().fg(Self::TEXT_DIM)
    }
}
