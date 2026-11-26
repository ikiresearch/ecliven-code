use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const ECLIPSE_WIDTH: usize = 63;
pub const ECLIPSE_HEIGHT: usize = 25;
const RADIUS: f64 = 8.6;
const DURATION_SECS: f64 = 14.0;

#[derive(Debug, Clone, Copy)]
pub struct EclipsePalette {
    pub sun_core: Color,
    pub sun_rim: Color,
    pub corona_inner: Color,
    pub corona_rays: Color,
    pub stars: Color,
}

impl EclipsePalette {
    pub fn solar() -> Self {
        Self {
            sun_core: Color::Rgb(255, 212, 154),
            sun_rim: Color::Rgb(255, 232, 190),
            corona_inner: Color::Rgb(255, 184, 108),
            corona_rays: Color::Rgb(215, 150, 88),
            stars: Color::Rgb(110, 118, 142),
        }
    }

    pub fn code_swarm() -> Self {
        Self {
            sun_core: Color::Rgb(150, 212, 255),
            sun_rim: Color::Rgb(205, 234, 255),
            corona_inner: Color::Rgb(105, 178, 255),
            corona_rays: Color::Rgb(84, 136, 214),
            stars: Color::Rgb(100, 114, 148),
        }
    }

    pub fn security_swarm() -> Self {
        Self {
            sun_core: Color::Rgb(255, 158, 194),
            sun_rim: Color::Rgb(255, 206, 224),
            corona_inner: Color::Rgb(255, 118, 168),
            corona_rays: Color::Rgb(198, 92, 140),
            stars: Color::Rgb(126, 108, 138),
        }
    }
}

pub struct EclipseFrame {
    pub lines: Vec<Line<'static>>,
    pub phase: &'static str,
    pub progress_bar: String,
}

pub fn render_frame(
    elapsed_secs: f64,
    max_height: usize,
    palette: EclipsePalette,
) -> EclipseFrame {
    let t = (elapsed_secs % DURATION_SECS) / DURATION_SECS;
    let mx = if t < 0.10 {
        -21.0
    } else if t < 0.43 {
        -21.0 + 21.0 * (t - 0.10) / 0.33
    } else if t < 0.57 {
        0.0
    } else if t < 0.90 {
        21.0 * (t - 0.57) / 0.33
    } else {
        21.0
    };

    let mut all_lines: Vec<Line<'static>> = Vec::with_capacity(ECLIPSE_HEIGHT);
    let half_w = (ECLIPSE_WIDTH - 1) as f64 / 2.0;
    let half_h = (ECLIPSE_HEIGHT - 1) as f64 / 2.0;

    for j in 0..ECLIPSE_HEIGHT {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_style = Style::default();
        let mut current_run = String::with_capacity(ECLIPSE_WIDTH);

        for i in 0..ECLIPSE_WIDTH {
            let x = (i as f64 - half_w) * 0.5;
            let y = j as f64 - half_h;
            let r = x.hypot(y);
            let a = y.atan2(x);
            let moon = (x - mx).hypot(y);
            let ray = (a * 6.0 + 0.25).cos().abs().powi(12);
            let ray2 = (a * 11.0 - 0.5).cos().abs().powi(19);

            let (ch, color) = if moon < 8.75 {
                (' ', palette.stars)
            } else if r < RADIUS {
                let v = 0.5 + 0.4 * (x * 1.7 + y * 0.8).sin() * (y * 2.1 - x * 0.4).cos();
                if r > RADIUS - 0.5 {
                    ('#', palette.sun_rim)
                } else if v > 0.72 {
                    ('@', palette.sun_core)
                } else if v > 0.40 {
                    ('%', palette.sun_core)
                } else {
                    ('#', palette.corona_inner)
                }
            } else if r < 9.35 {
                if r < 9.0 {
                    ('*', palette.sun_rim)
                } else {
                    ('+', palette.corona_inner)
                }
            } else if r < 9.7 + 2.6 * ray + 1.1 * ray2 {
                if ray > 0.65 {
                    let c = if a.sin().abs() > 0.85 {
                        '|'
                    } else if a.cos().abs() > 0.9 {
                        '-'
                    } else if a.sin() * a.cos() > 0.0 {
                        '\\'
                    } else {
                        '/'
                    };
                    (c, palette.corona_rays)
                } else {
                    let c = if r < 10.1 { '.' } else { '\'' };
                    (c, palette.corona_rays)
                }
            } else if (i * 127 + j * 43 + i * j * 13) % 211 == 0 && r > 11.0 {
                ('.', palette.stars)
            } else {
                (' ', palette.stars)
            };

            let style = if ch == ' ' {
                Style::default()
            } else if ch == '@' || ch == '*' {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };

            if style != current_style && !current_run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut current_run), current_style));
            }
            current_style = style;
            current_run.push(ch);
        }

        if !current_run.is_empty() {
            spans.push(Span::styled(current_run, current_style));
        }
        all_lines.push(Line::from(spans));
    }

    let lines = if max_height >= ECLIPSE_HEIGHT {
        all_lines
    } else if max_height == 0 {
        Vec::new()
    } else {
        let trim = ECLIPSE_HEIGHT - max_height;
        let top = trim / 2;
        all_lines.into_iter().skip(top).take(max_height).collect()
    };

    let phase = if mx.abs() < 0.25 {
        "TOTALITY"
    } else if mx.abs() > 17.5 {
        "FULL SUN"
    } else if mx < 0.0 {
        "INGRESS"
    } else {
        "EGRESS"
    };

    let n = (t * 24.0).round() as usize;
    let filled = n.min(24);
    let progress_bar = format!(
        "[{}{}]",
        "=".repeat(filled),
        "-".repeat(24usize.saturating_sub(filled))
    );

    EclipseFrame {
        lines,
        phase,
        progress_bar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_expected_eclipse_phases_across_cycle() {
        let full = render_frame(0.5, 25, EclipsePalette::solar());
        assert_eq!(full.phase, "FULL SUN");
        assert_eq!(full.lines.len(), 25);

        let ingress = render_frame(3.5, 25, EclipsePalette::solar());
        assert_eq!(ingress.phase, "INGRESS");

        let totality = render_frame(7.0, 25, EclipsePalette::solar());
        assert_eq!(totality.phase, "TOTALITY");

        let egress = render_frame(10.5, 25, EclipsePalette::solar());
        assert_eq!(egress.phase, "EGRESS");
    }

    #[test]
    fn crops_gracefully_for_short_terminals() {
        let cropped = render_frame(7.0, 13, EclipsePalette::code_swarm());
        assert_eq!(cropped.lines.len(), 13);
    }
}
