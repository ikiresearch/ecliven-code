use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

use crate::swarm::domain::{SecuritySubMode, SwarmDomain};
use crate::swarm::SwarmAgentStatus;
use crate::tui::app::{App, ChatEntry, WorkspaceSection};
use crate::tui::eclipse::{self, EclipsePalette, ECLIPSE_HEIGHT};
use crate::tui::onboarding::{OnboardingState, OnboardingStep};
use crate::tui::theme::Theme;

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    if let Some(ref ob) = app.onboarding {
        render_onboarding_wizard(frame, ob, area);
        return;
    }

    let update_height = if app.available_update.is_some() { 1 } else { 0 };

    if app.is_landing_view() {
        render_landing(frame, app, area, update_height);
    } else {
        render_workspace(frame, app, area, update_height);
    }

    if app.pending_approval.is_some() {
        render_approval_modal(frame, app, area);
    } else if app.model_picker.is_some() {
        render_model_picker_modal(frame, app, area);
    } else if app.show_swarm_hub {
        render_swarm_hub_modal(frame, app, area);
    } else if app.show_help {
        render_help_modal(frame, area);
    }
}

fn render_landing(frame: &mut Frame<'_>, app: &App, area: Rect, update_height: u16) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(update_height),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(area);

    if app.available_update.is_some() {
        render_update_banner(frame, app, outer[0]);
    }

    let center_area = outer[1];
    let reserved_for_chrome: u16 = 12;
    let available_art = center_area.height.saturating_sub(reserved_for_chrome) as usize;
    let art_height = available_art.clamp(7, ECLIPSE_HEIGHT);

    let palette = match app.active_section {
        WorkspaceSection::Chat => EclipsePalette::solar(),
        WorkspaceSection::SwarmCode => EclipsePalette::code_swarm(),
        WorkspaceSection::SwarmSecurity => EclipsePalette::security_swarm(),
    };
    let eclipse_frame = eclipse::render_frame(app.elapsed_secs(), art_height, palette);

    let content_height = (art_height as u16) + 10;
    let top_pad = center_area.height.saturating_sub(content_height) / 2;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_pad),
            Constraint::Length(1),
            Constraint::Length(art_height as u16),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(center_area);

    let title_style = match app.active_section {
        WorkspaceSection::Chat => Theme::brand(),
        WorkspaceSection::SwarmCode => Theme::tab_active_code(),
        WorkspaceSection::SwarmSecurity => Theme::tab_active_security(),
    };
    let hero_title = match app.active_swarm_domain() {
        None => "E C L I V E N   C O D E",
        Some(SwarmDomain::Code) => "E C L I V E N   ·   C O D E   S W A R M",
        Some(SwarmDomain::Security(SecuritySubMode::Defense)) => {
            "E C L I V E N   ·   S E C U R I T Y   D E F E N S E"
        }
        Some(SwarmDomain::Security(SecuritySubMode::ThreatModel)) => {
            "E C L I V E N   ·   T H R E A T   M O D E L"
        }
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hero_title, title_style)).alignment(Alignment::Center),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(eclipse_frame.lines).alignment(Alignment::Center),
        rows[2],
    );

    let settings = app.config.active_settings();
    let workspace = app
        .config
        .workspace_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".");
    let scope_part = app
        .active_state()
        .target_scope
        .as_deref()
        .map(|s| format!(" · scope:{s}"))
        .unwrap_or_default();

    let mut meta_spans = vec![
        Span::styled(
            format!("v{}", env!("CARGO_PKG_VERSION")),
            Theme::footer_hint(),
        ),
        Span::styled("  ·  ", Theme::footer_hint()),
        Span::styled(
            format!("{}/{}", app.config.active_provider, settings.model),
            Theme::system_text(),
        ),
        Span::styled("  ·  ", Theme::footer_hint()),
        Span::styled(format!("{workspace}{scope_part}"), Theme::system_text()),
        Span::styled("  ·  ", Theme::footer_hint()),
    ];
    meta_spans.extend(build_context_spans(app));

    frame.render_widget(
        Paragraph::new(Line::from(meta_spans)).alignment(Alignment::Center),
        rows[3],
    );

    let pills = build_mode_pills(app);
    frame.render_widget(Paragraph::new(pills).alignment(Alignment::Center), rows[5]);

    let input_width = center_area.width.saturating_sub(6).clamp(36, 78);
    let input_x = center_area.x + center_area.width.saturating_sub(input_width) / 2;
    let input_rect = Rect::new(input_x, rows[6].y, input_width, rows[6].height);
    render_input_box(frame, app, input_rect, true);

    let sub_hint = if let Some(hint) = app.missing_key_hint() {
        Line::from(Span::styled(format!("! {hint}"), Theme::status_running()))
    } else if let Some(domain) = app.active_swarm_domain() {
        Line::from(Span::styled(domain.subtitle(), Theme::footer_hint()))
    } else {
        Line::from(Span::styled(
            "Ask anything · /connect codex|agy|claude · /swarm · /help",
            Theme::footer_hint(),
        ))
    };
    frame.render_widget(
        Paragraph::new(sub_hint).alignment(Alignment::Center),
        rows[7],
    );

    render_footer(frame, app, outer[2]);
}

fn render_workspace(frame: &mut Frame<'_>, app: &mut App, area: Rect, update_height: u16) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(update_height),
            Constraint::Min(6),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .split(area);

    render_top_bar(frame, app, chunks[0]);
    if app.available_update.is_some() {
        render_update_banner(frame, app, chunks[1]);
    }

    if let Some(domain) = app.active_swarm_domain() {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
            .split(chunks[2]);
        render_transcript(frame, app, body[0]);
        render_swarm_sidebar(frame, app, domain, body[1]);
    } else {
        render_transcript(frame, app, chunks[2]);
    }

    render_input_box(frame, app, chunks[3], false);
    render_footer(frame, app, chunks[4]);
}

fn render_update_banner(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref update) = app.available_update else {
        return;
    };
    let text = format!(
        " ★ UPDATE AVAILABLE: v{} -> v{} · Press [F5] or type /update to upgrade ",
        crate::updater::CURRENT_VERSION,
        update.version
    );
    let banner = Paragraph::new(text)
        .alignment(Alignment::Center)
        .style(Theme::update_banner());
    frame.render_widget(banner, area);
}

fn render_top_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let settings = app.config.active_settings();
    let workspace = app
        .config
        .workspace_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".");
    let icon = if app.is_busy {
        SPINNER[app.tick_count % SPINNER.len()]
    } else {
        "◐"
    };

    let mut spans = vec![
        Span::styled(format!(" {icon} ECLIVEN "), Theme::brand()),
        Span::styled("│ ", Theme::footer_hint()),
    ];
    spans.extend(build_mode_pills(app).spans);
    spans.push(Span::styled(
        format!(
            "  │  {}/{} · {}  │  ",
            app.config.active_provider, settings.model, workspace,
        ),
        Theme::system_text(),
    ));
    spans.extend(build_context_spans(app));

    let bar = Paragraph::new(Line::from(spans)).style(Theme::header_bar());
    frame.render_widget(bar, area);
}

fn build_context_spans(app: &App) -> Vec<Span<'static>> {
    let (used, max_ctx, pct) = app.context_usage();
    const BAR_WIDTH: usize = 10;
    let filled = ((pct as usize) * BAR_WIDTH).div_ceil(100).min(BAR_WIDTH);
    let empty = BAR_WIDTH.saturating_sub(filled);

    let fill_style = if pct >= 85 {
        Theme::error_text()
    } else if pct >= 65 {
        Theme::status_running()
    } else {
        Theme::brand()
    };

    vec![
        Span::styled("ctx ", Theme::footer_hint()),
        Span::styled("█".repeat(filled), fill_style),
        Span::styled("░".repeat(empty), Theme::footer_hint()),
        Span::styled(
            format!(" {pct}% ({}/{})", format_k(used), format_k(max_ctx)),
            Theme::system_text(),
        ),
    ]
}

fn format_k(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

fn build_mode_pills(app: &App) -> Line<'static> {
    let (c_dot, c_style) = if app.active_section == WorkspaceSection::Chat {
        ("●", Theme::tab_active_chat())
    } else {
        ("○", Theme::tab_inactive())
    };
    let (code_dot, code_style) = if app.active_section == WorkspaceSection::SwarmCode {
        ("●", Theme::tab_active_code())
    } else {
        ("○", Theme::tab_inactive())
    };
    let (sec_dot, sec_style) = if app.active_section == WorkspaceSection::SwarmSecurity {
        ("●", Theme::tab_active_security())
    } else {
        ("○", Theme::tab_inactive())
    };

    Line::from(vec![
        Span::styled(format!("{c_dot} Chat"), c_style),
        Span::styled("   ", Theme::footer_hint()),
        Span::styled(format!("{code_dot} Swarm: Code"), code_style),
        Span::styled("   ", Theme::footer_hint()),
        Span::styled(
            format!(
                "{sec_dot} Swarm: Security ({})",
                app.security_sub_mode.as_str()
            ),
            sec_style,
        ),
    ])
}

fn render_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let active_domain = app.active_swarm_domain();
    let state = app.active_state_mut();
    let mut lines: Vec<Line<'_>> = Vec::new();

    if let Some(ref scope) = state.target_scope {
        lines.push(Line::from(vec![
            Span::styled("  scope ", Theme::footer_hint()),
            Span::styled(scope.clone(), Theme::tool_label()),
        ]));
        lines.push(Line::from(""));
    }

    for entry in &state.entries {
        match entry {
            ChatEntry::User(text) => {
                for (idx, l) in text.lines().enumerate() {
                    let prefix = if idx == 0 { "  ❯ " } else { "    " };
                    lines.push(Line::from(vec![
                        Span::styled(prefix, Theme::user_label()),
                        Span::styled(l.to_string(), Theme::text_primary()),
                    ]));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::Assistant(text) => {
                lines.push(Line::from(Span::styled(
                    "  ◐ Ecliven",
                    Theme::assistant_label(),
                )));
                for l in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("  ┃ ", Theme::border_inactive()),
                        Span::styled(l.to_string(), Theme::text_primary()),
                    ]));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::SwarmReport {
                domain,
                badge,
                agent_name,
                content,
            } => {
                lines.push(Line::from(vec![
                    Span::styled(format!("  [{badge}] "), Theme::swarm_badge(*domain)),
                    Span::styled(agent_name.clone(), Theme::system_text()),
                ]));
                for l in content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ", Theme::border_inactive()),
                        Span::styled(l.to_string(), Theme::system_text()),
                    ]));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::SwarmLead {
                domain,
                badge,
                content,
            } => {
                let heading = if matches!(domain, SwarmDomain::Code) {
                    format!("  ★ [{badge}] Combined Best Solution")
                } else {
                    format!("  ★ [{badge}] Consolidated Security Report")
                };
                lines.push(Line::from(Span::styled(heading, Theme::swarm_badge(*domain))));
                let rail_style = match domain {
                    SwarmDomain::Code => Theme::border_code(),
                    SwarmDomain::Security(_) => Theme::border_security(),
                };
                for l in content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("  ┃ ", rail_style),
                        Span::styled(l.to_string(), Theme::text_primary()),
                    ]));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::ToolCall { name, summary } => {
                lines.push(Line::from(vec![
                    Span::styled(format!("  ⚙ {name} "), Theme::tool_label()),
                    Span::styled(summary.clone(), Theme::system_text()),
                ]));
            }
            ChatEntry::ToolResult {
                name,
                output,
                diff,
                is_error,
            } => {
                let (sym, style) = if *is_error {
                    ("✗", Theme::error_text())
                } else {
                    ("✓", Theme::status_done())
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("    {sym} "), style),
                    Span::styled(name.clone(), Theme::system_text()),
                ]));
                if let Some(diff_text) = diff {
                    for dl in diff_text.lines() {
                        let d_style = if dl.starts_with('+') && !dl.starts_with("+++") {
                            Theme::diff_add()
                        } else if dl.starts_with('-') && !dl.starts_with("---") {
                            Theme::diff_del()
                        } else if dl.starts_with("@@") {
                            Theme::diff_hunk()
                        } else {
                            Theme::system_text()
                        };
                        lines.push(Line::from(Span::styled(format!("      {dl}"), d_style)));
                    }
                } else {
                    for ol in output.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("      {ol}"),
                            Theme::system_text(),
                        )));
                    }
                }
                lines.push(Line::from(""));
            }
            ChatEntry::System(text) => {
                for l in text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  · {l}"),
                        Theme::system_text(),
                    )));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::Error(text) => {
                lines.push(Line::from(Span::styled(
                    format!("  ✗ {text}"),
                    Theme::error_text(),
                )));
                lines.push(Line::from(""));
            }
        }
    }

    if !state.streaming_buffer.is_empty() {
        let label = match active_domain {
            Some(SwarmDomain::Code) => "  ★ [CHIEF] Combining best code...".to_string(),
            Some(domain) => format!("  ★ [{}] Synthesizing...", domain.lead_badge()),
            None => "  ◐ Ecliven".to_string(),
        };
        lines.push(Line::from(Span::styled(label, Theme::assistant_label())));
        for l in state.streaming_buffer.lines() {
            lines.push(Line::from(vec![
                Span::styled("  ┃ ", Theme::border_active()),
                Span::styled(l.to_string(), Theme::text_primary()),
            ]));
        }
    }

    let inner_height = area.height.saturating_sub(1) as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(inner_height) as u16;
    if state.auto_scroll || state.scroll_offset > max_scroll {
        state.scroll_offset = max_scroll;
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().padding(Padding::new(0, 1, 1, 0)))
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset, 0));

    frame.render_widget(paragraph, area);
}

fn render_swarm_sidebar(frame: &mut Frame<'_>, app: &App, domain: SwarmDomain, area: Rect) {
    let state = app.active_state();
    let spin = SPINNER[app.tick_count % SPINNER.len()];

    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            domain.display_name(),
            Theme::swarm_badge(domain),
        )),
        Line::from(Span::styled(domain.subtitle(), Theme::system_text())),
    ];

    let scope_text = state
        .target_scope
        .as_deref()
        .unwrap_or("full workspace");
    lines.push(Line::from(vec![
        Span::styled("scope: ", Theme::footer_hint()),
        Span::styled(scope_text, Theme::text_primary()),
    ]));

    if domain.is_security() {
        lines.push(Line::from(Span::styled(
            "F4: Defense <-> Threat Model",
            Theme::footer_hint(),
        )));
    }

    lines.push(Line::from(""));
    let team_heading = if matches!(domain, SwarmDomain::Code) {
        "Candidate Coders"
    } else {
        "Security Specialists"
    };
    lines.push(Line::from(Span::styled(team_heading, Theme::tool_label())));

    for worker in &state.roster {
        let (dot, detail, style) = match &worker.status {
            SwarmAgentStatus::Idle => ("○", "idle".to_string(), Theme::footer_hint()),
            SwarmAgentStatus::Running { active_tool } => {
                let d = active_tool
                    .as_deref()
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "working".to_string());
                (spin, d, Theme::status_running())
            }
            SwarmAgentStatus::Done => ("✓", "done".to_string(), Theme::status_done()),
            SwarmAgentStatus::Failed(_) => ("✗", "error".to_string(), Theme::error_text()),
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{dot} "), style),
            Span::styled(format!("[{}] ", worker.badge), Theme::swarm_badge(domain)),
            Span::styled(worker.name.clone(), Theme::text_primary()),
            Span::styled(format!(" · {detail}"), style),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", worker.summary),
            Theme::footer_hint(),
        )));
    }

    lines.push(Line::from(""));
    let lead_heading = if matches!(domain, SwarmDomain::Code) {
        "Master Combiner"
    } else {
        "Security Coordinator"
    };
    lines.push(Line::from(Span::styled(lead_heading, Theme::tool_label())));

    let (lead_dot, lead_label, lead_style) = match &state.lead_status {
        SwarmAgentStatus::Idle => ("○", "idle".to_string(), Theme::footer_hint()),
        SwarmAgentStatus::Running { active_tool } => {
            let d = active_tool
                .as_deref()
                .map(|t| t.to_string())
                .unwrap_or_else(|| "combining".to_string());
            (spin, d, Theme::status_running())
        }
        SwarmAgentStatus::Done => ("✓", "done".to_string(), Theme::status_done()),
        SwarmAgentStatus::Failed(_) => ("✗", "error".to_string(), Theme::error_text()),
    };

    lines.push(Line::from(vec![
        Span::styled(format!("{lead_dot} "), lead_style),
        Span::styled(
            format!("[{}]", domain.lead_badge()),
            Theme::swarm_badge(domain),
        ),
        Span::styled(format!(" · {lead_label}"), lead_style),
    ]));

    let border_style = match domain {
        SwarmDomain::Code => Theme::border_code(),
        SwarmDomain::Security(_) => Theme::border_security(),
    };

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(border_style)
        .padding(Padding::new(1, 1, 1, 0));

    let panel = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

fn render_input_box(frame: &mut Frame<'_>, app: &App, area: Rect, is_landing: bool) {
    let border = if app.is_busy {
        Theme::border_inactive()
    } else {
        match app.active_section {
            WorkspaceSection::Chat => Theme::border_active(),
            WorkspaceSection::SwarmCode => Theme::border_code(),
            WorkspaceSection::SwarmSecurity => Theme::border_security(),
        }
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .padding(Padding::horizontal(1));

    if !is_landing {
        let title = if app.is_busy {
            format!(" {} working... ", app.active_section.label())
        } else {
            match app.active_swarm_domain() {
                None => " ❯ Prompt ".to_string(),
                Some(SwarmDomain::Code) => " ❯ Code Swarm (3 Coders -> CHIEF) ".to_string(),
                Some(SwarmDomain::Security(SecuritySubMode::Defense)) => {
                    " ❯ Security Defense (/scope <dir> · F4 switch) ".to_string()
                }
                Some(SwarmDomain::Security(SecuritySubMode::ThreatModel)) => {
                    " ❯ Security Threat Model (STRIDE · F4 switch) ".to_string()
                }
            }
        };
        block = block.title(title);
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.input.is_empty() && !app.is_busy {
        let placeholder = match app.active_swarm_domain() {
            None => "❯ Ask Ecliven anything...  (/connect codex|agy, /swarm, /help)",
            Some(SwarmDomain::Code) => {
                "❯ Describe the coding task for 3 parallel coders + Chief..."
            }
            Some(SwarmDomain::Security(SecuritySubMode::Defense)) => {
                "❯ Audit codebase or folder for vulnerabilities (/scope src)..."
            }
            Some(SwarmDomain::Security(SecuritySubMode::ThreatModel)) => {
                "❯ Run STRIDE threat modeling & hardening review..."
            }
        };
        frame.render_widget(
            Paragraph::new(Span::styled(placeholder, Theme::placeholder())),
            inner,
        );
    } else {
        let prompt_text = format!("❯ {}", app.input);
        frame.render_widget(
            Paragraph::new(prompt_text)
                .style(Theme::text_primary())
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    if !app.is_busy
        && app.pending_approval.is_none()
        && app.model_picker.is_none()
        && !app.show_help
        && !app.show_swarm_hub
    {
        let before_cursor = &app.input[..app.cursor_pos];
        let lines: Vec<&str> = before_cursor.split('\n').collect();
        let row = lines.len().saturating_sub(1) as u16;
        let col_offset = if row == 0 { 2 } else { 0 };
        let col = lines.last().map(|l| l.chars().count()).unwrap_or(0) as u16 + col_offset;

        let cx = (inner.x + col).min(inner.x + inner.width.saturating_sub(1));
        let cy = (inner.y + row).min(inner.y + inner.height.saturating_sub(1));
        frame.set_cursor_position((cx, cy));
    }
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let text = if app.pending_approval.is_some() {
        " [Y/Enter] Approve  ·  [N/Esc] Deny  ·  [Ctrl+Q] Quit"
    } else if app.model_picker.is_some() {
        " Type to filter  ·  [↑/↓] Navigate  ·  [Enter] Select  ·  [Ctrl+R] Refresh  ·  [Esc] Close"
    } else if app.show_swarm_hub {
        " [1] Code Swarm  ·  [2] Security: Defensa  ·  [3] Security: Threat Model  ·  [0] Chat  ·  [Esc] Close"
    } else {
        " F2: Mode  ·  F3: Swarm Hub  ·  Ctrl+P/F7: Models  ·  F6: Folder  ·  F1: Help  ·  Ctrl+Q: Quit"
    };

    let footer = Paragraph::new(text)
        .alignment(Alignment::Center)
        .style(Theme::footer_hint());
    frame.render_widget(footer, area);
}

fn render_model_picker_modal(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref picker) = app.model_picker else {
        return;
    };

    let modal_area = centered_rect(76, 72, area);
    frame.render_widget(Clear, modal_area);

    let loading_tag = if picker.loading {
        format!(" [{} fetching live models...]", SPINNER[app.tick_count % SPINNER.len()])
    } else {
        format!(" ({} models)", picker.models.len())
    };
    let title = format!(" ◐ Models · {}{loading_tag} ", picker.provider);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::modal_border())
        .padding(Padding::new(1, 1, 0, 0))
        .title(title);
    let inner = outer.inner(modal_area);
    frame.render_widget(outer, modal_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(inner);

    let filter_box = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_inactive())
        .padding(Padding::horizontal(1));
    let filter_text = if picker.filter.is_empty() {
        Line::from(Span::styled(
            "❯ Type to search models...",
            Theme::placeholder(),
        ))
    } else {
        Line::from(vec![
            Span::styled("❯ ", Theme::user_label()),
            Span::styled(picker.filter.clone(), Theme::text_primary()),
            Span::styled("_", Theme::brand()),
        ])
    };
    frame.render_widget(Paragraph::new(filter_text).block(filter_box), rows[0]);

    let filtered = picker.filtered_models();
    let active_model = app.config.active_settings().model.as_str();
    let max_visible = rows[1].height as usize;

    let mut lines: Vec<Line<'_>> = Vec::new();
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No models match your filter.",
            Theme::system_text(),
        )));
    } else {
        let start = if picker.selected >= max_visible && max_visible > 0 {
            picker.selected + 1 - max_visible
        } else {
            0
        };
        let end = (start + max_visible.max(1)).min(filtered.len());
        for (idx, model) in filtered.iter().enumerate().take(end).skip(start) {
            let is_sel = idx == picker.selected;
            let is_active = model.id == active_model;
            let cursor = if is_sel { "❯ " } else { "  " };
            let dot = if is_active { "● " } else { "○ " };
            let id_style = if is_sel {
                Theme::brand()
            } else if is_active {
                Theme::status_done()
            } else {
                Theme::text_primary()
            };

            let mut spans = vec![
                Span::styled(cursor, Theme::brand()),
                Span::styled(dot, if is_active { Theme::status_done() } else { Theme::footer_hint() }),
                Span::styled(model.id.clone(), id_style),
            ];
            if model.label != model.id {
                spans.push(Span::styled(
                    format!("  —  {}", model.label),
                    Theme::system_text(),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    frame.render_widget(Paragraph::new(lines), rows[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "[↑/↓] Move  ·  [Enter] Select  ·  [Ctrl+R] Refresh  ·  [Esc] Close",
            Theme::footer_hint(),
        ))
        .alignment(Alignment::Center),
        rows[2],
    );
}

fn render_swarm_hub_modal(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let modal_area = centered_rect(86, 66, area);
    frame.render_widget(Clear, modal_area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::modal_border())
        .title(" ◐ Swarm Hub — Choose Workspace ");
    let inner = outer.inner(modal_area);
    frame.render_widget(outer, modal_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(2)])
        .split(inner);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[0]);

    let code_active = if app.active_section == WorkspaceSection::SwarmCode {
        " ●"
    } else {
        ""
    };
    let code_lines = vec![
        Line::from(Span::styled(
            format!("[1] SWARM: CODIGO{code_active}"),
            Theme::swarm_badge(SwarmDomain::Code),
        )),
        Line::from(""),
        Line::from("3 agentes resuelven la MISMA tarea en paralelo:"),
        Line::from("  · [CLEAN]  Arquitectura limpia y modular"),
        Line::from("  · [PERF]   Velocidad, memoria y concurrencia"),
        Line::from("  · [ROBUST] Casos borde, seguridad y tests"),
        Line::from(""),
        Line::from(Span::styled(
            "  ★ [CHIEF] Agente Mayor:",
            Theme::tool_label(),
        )),
        Line::from("    Evalua las 3 propuestas y combina el MEJOR"),
        Line::from("    codigo posible en una sola solucion final."),
    ];
    let code_card = Paragraph::new(code_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Theme::border_code())
                .padding(Padding::horizontal(1))
                .title(" Codigo (Best-of-3) "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(code_card, cards[0]);

    let def_active = if app.active_section == WorkspaceSection::SwarmSecurity
        && app.security_sub_mode == SecuritySubMode::Defense
    {
        " ●"
    } else {
        ""
    };
    let threat_active = if app.active_section == WorkspaceSection::SwarmSecurity
        && app.security_sub_mode == SecuritySubMode::ThreatModel
    {
        " ●"
    } else {
        ""
    };
    let sec_lines = vec![
        Line::from(Span::styled(
            format!("[2] DEFENSA (Codigo / Carpeta){def_active}"),
            Theme::swarm_badge(SwarmDomain::security_defense()),
        )),
        Line::from("  Audita una carpeta o codigo (/scope <dir>):"),
        Line::from("  · [SURFACE] + [AUDIT] (OWASP/CWE) + [PATCH]"),
        Line::from("  ★ [DEFENSE LEAD]: vulnerabilidades y parches"),
        Line::from(""),
        Line::from(Span::styled(
            format!("[3] THREAT MODEL & HARDENING{threat_active}"),
            Theme::swarm_badge(SwarmDomain::security_threat()),
        )),
        Line::from("  Postura defensiva y arquitectura segura:"),
        Line::from("  · [STRIDE] + [CONFIG] (Deps/CI) + [BLUETEAM]"),
        Line::from("  ★ [SEC ARCHITECT]: matriz STRIDE y plan"),
    ];
    let sec_card = Paragraph::new(sec_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Theme::border_security())
                .padding(Padding::horizontal(1))
                .title(" Ciberseguridad (2 Modos) "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(sec_card, cards[1]);

    let bottom = Paragraph::new(
        "[1] Code Swarm  ·  [2] Sec: Defensa  ·  [3] Sec: Threat Model  ·  [0] Chat  ·  [Esc] Close",
    )
    .alignment(Alignment::Center)
    .style(Theme::brand());
    frame.render_widget(bottom, rows[1]);
}

fn render_approval_modal(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref pending) = app.pending_approval else {
        return;
    };

    let modal_area = centered_rect(80, 70, area);
    frame.render_widget(Clear, modal_area);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Tool: {}", pending.tool_name),
            Theme::tool_label(),
        )),
        Line::from(pending.preview.summary.clone()),
        Line::from(""),
    ];

    if let Some(ref diff) = pending.preview.diff {
        for dl in diff.lines().take(30) {
            let style = if dl.starts_with('+') && !dl.starts_with("+++") {
                Theme::diff_add()
            } else if dl.starts_with('-') && !dl.starts_with("---") {
                Theme::diff_del()
            } else if dl.starts_with("@@") {
                Theme::diff_hunk()
            } else {
                Theme::system_text()
            };
            lines.push(Line::from(Span::styled(dl.to_string(), style)));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "Press [Y] or [Enter] to approve · [N] or [Esc] to deny",
        Theme::brand(),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::modal_border())
        .padding(Padding::horizontal(1))
        .title(" Tool Approval Required ");

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, modal_area);
}

fn render_onboarding_wizard(frame: &mut Frame<'_>, ob: &OnboardingState, area: Rect) {
    let modal_area = centered_rect(82, 76, area);
    frame.render_widget(Clear, modal_area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::modal_border())
        .padding(Padding::new(2, 2, 1, 1))
        .title(" ◐ E C L I V E N   C O D E  ·  Configuracion inicial ");
    let inner = outer.inner(modal_area);
    frame.render_widget(outer, modal_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    let step_style = |step: OnboardingStep| {
        if ob.step == step {
            Theme::brand()
        } else {
            Theme::footer_hint()
        }
    };
    let steps_line = Line::from(vec![
        Span::styled(
            if ob.step == OnboardingStep::Terms {
                "● 1. Terminos y Condiciones"
            } else {
                "✓ 1. Terminos y Condiciones"
            },
            step_style(OnboardingStep::Terms),
        ),
        Span::styled("   ·   ", Theme::footer_hint()),
        Span::styled(
            if ob.step == OnboardingStep::Telemetry {
                "● 2. Telemetria"
            } else if ob.step == OnboardingStep::Workspace {
                "✓ 2. Telemetria"
            } else {
                "○ 2. Telemetria"
            },
            step_style(OnboardingStep::Telemetry),
        ),
        Span::styled("   ·   ", Theme::footer_hint()),
        Span::styled(
            if ob.step == OnboardingStep::Workspace {
                "● 3. Carpeta de trabajo"
            } else {
                "○ 3. Carpeta de trabajo"
            },
            step_style(OnboardingStep::Workspace),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(steps_line).alignment(Alignment::Center),
        rows[0],
    );

    match ob.step {
        OnboardingStep::Terms => {
            let lines = vec![
                Line::from(Span::styled(
                    "Terminos y Condiciones de Uso",
                    Theme::brand(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Antes de usar Ecliven Code, revisa y acepta los siguientes terminos:",
                    Theme::text_primary(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "1. Asistencia con IA y revision humana: Ecliven puede leer/editar archivos y ejecutar",
                    Theme::system_text(),
                )),
                Line::from(Span::styled(
                    "   comandos en tu workspace. Eres responsable de revisar diffs y aprobar cambios.",
                    Theme::system_text(),
                )),
                Line::from(Span::styled(
                    "2. Ciberseguridad defensiva: El modo Security esta disenado exclusivamente para",
                    Theme::system_text(),
                )),
                Line::from(Span::styled(
                    "   auditorias defensivas y codigo propio o expresamente autorizado.",
                    Theme::system_text(),
                )),
                Line::from(Span::styled(
                    "3. Privacidad de credenciales: Tus API keys y CLIs conectados (codex, agy, claude)",
                    Theme::system_text(),
                )),
                Line::from(Span::styled(
                    "   se ejecutan localmente y nunca se suben a servidores de Ecliven.",
                    Theme::system_text(),
                )),
                Line::from(Span::styled(
                    "4. Sin garantia: El software se entrega 'tal cual' (AS-IS).",
                    Theme::system_text(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[1]);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "[Enter / A] Aceptar y continuar   ·   [Esc / Ctrl+Q] Rechazar y salir",
                    Theme::brand(),
                ))
                .alignment(Alignment::Center),
                rows[2],
            );
        }
        OnboardingStep::Telemetry => {
            let (on_dot, on_style, off_dot, off_style) = if ob.telemetry_choice {
                (
                    "●",
                    Theme::brand(),
                    "○",
                    Theme::system_text(),
                )
            } else {
                (
                    "○",
                    Theme::system_text(),
                    "●",
                    Theme::brand(),
                )
            };

            let lines = vec![
                Line::from(Span::styled(
                    "Preferencia de Telemetria de Datos",
                    Theme::brand(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Elige si deseas compartir metricas anonimas para mejorar Ecliven Code.",
                    Theme::text_primary(),
                )),
                Line::from(Span::styled(
                    "Nunca se recolecta codigo fuente, prompts, rutas privadas ni API keys.",
                    Theme::system_text(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {on_dot}  [1] Activar telemetria anonima"),
                    on_style,
                )),
                Line::from(Span::styled(
                    "       Reporta solo version, sistema operativo y fallos tecnicos anonimos.",
                    Theme::footer_hint(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {off_dot}  [2] Desactivar telemetria (100% privado)"),
                    off_style,
                )),
                Line::from(Span::styled(
                    "       Cero envio de datos de uso. Puedes cambiarlo luego con /telemetry.",
                    Theme::footer_hint(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[1]);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "[↑/↓ / Space / 1 / 2] Elegir   ·   [Enter] Continuar",
                    Theme::brand(),
                ))
                .alignment(Alignment::Center),
                rows[2],
            );
        }
        OnboardingStep::Workspace => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Selecciona la Carpeta de Trabajo",
                    Theme::brand(),
                )),
                Line::from(vec![
                    Span::styled("Actual: ", Theme::footer_hint()),
                    Span::styled(ob.browse_dir.display().to_string(), Theme::text_primary()),
                ]),
                Line::from(""),
            ];

            if ob.manual_path_mode {
                lines.push(Line::from(Span::styled(
                    "Escribe o pega la ruta completa de la carpeta y presiona Enter:",
                    Theme::system_text(),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("  ❯ ", Theme::user_label()),
                    Span::styled(ob.manual_path_input.clone(), Theme::text_primary()),
                    Span::styled("_", Theme::brand()),
                ]));
            } else {
                let max_visible = rows[1].height.saturating_sub(5) as usize;
                let total = ob.total_workspace_items();
                let start = if ob.selected_index >= max_visible && max_visible > 0 {
                    ob.selected_index + 1 - max_visible
                } else {
                    0
                };
                let end = (start + max_visible.max(1)).min(total);

                for idx in start..end {
                    let selected = idx == ob.selected_index;
                    let prefix = if selected { "❯ " } else { "  " };
                    let style = if selected {
                        Theme::brand()
                    } else {
                        Theme::text_primary()
                    };

                    let label = if idx == 0 {
                        "★ Usar esta carpeta".to_string()
                    } else if idx == 1 {
                        "↩ .. (carpeta superior)".to_string()
                    } else if let Some(sub) = ob.subdirs.get(idx - 2) {
                        let name = sub
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?");
                        format!("▸ {name}/")
                    } else {
                        String::new()
                    };

                    lines.push(Line::from(Span::styled(format!("{prefix}{label}"), style)));
                }
            }

            if let Some(ref err) = ob.error_msg {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("✗ {err}"),
                    Theme::error_text(),
                )));
            }

            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[1]);

            let hint = if ob.manual_path_mode {
                "[Enter] Confirmar ruta   ·   [Esc] Volver al explorador"
            } else {
                "[↑/↓] Mover  ·  [Enter] Elegir/Entrar  ·  [←/Bksp] Subir  ·  [P] Escribir ruta"
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, Theme::brand())).alignment(Alignment::Center),
                rows[2],
            );
        }
    }
}

fn render_help_modal(frame: &mut Frame<'_>, area: Rect) {
    let modal_area = centered_rect(74, 68, area);
    frame.render_widget(Clear, modal_area);

    let help = "\
Keyboard Shortcuts:
  F2            Switch mode (Chat -> Swarm: Code -> Swarm: Security)
  F3            Open Swarm Hub (Code vs Security 2-mode picker)
  F4            Toggle Security sub-mode (Defensa <-> Threat Model)
  F5            Install available update immediately
  Enter         Send prompt
  Ctrl+J        Insert newline
  Ctrl+U        Clear input box
  Ctrl+L        Clear current section (returns to Eclipse home)
  Up / Down     Prompt history
  PgUp / PgDn   Scroll transcript
  F1            Toggle help
  Ctrl+Q        Quit Ecliven Code

Slash Commands:
  /connect | /connect codex | /connect agy | /connect claude | /connect opencode
  /connect custom <command> [args...]
  /swarm | /swarm code [folder]
  /swarm security defense [folder] | /swarm security threat [folder]
  /swarm off | /scope <folder|off>
  /update [check] | /provider <name> | /model <name>
  /approval <ask|auto-edit|auto-all> | /status | /clear | /quit

Press F1 or Esc to close.";

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_active())
        .padding(Padding::horizontal(1))
        .title(" ◐ Ecliven Code Help ");

    let paragraph = Paragraph::new(help).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, modal_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
