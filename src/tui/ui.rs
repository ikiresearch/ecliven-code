use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::swarm::domain::{SecuritySubMode, SwarmDomain};
use crate::swarm::SwarmAgentStatus;
use crate::tui::app::{App, ChatEntry, WorkspaceSection};
use crate::tui::theme::Theme;

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let update_height = if app.available_update.is_some() { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(update_height),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, chunks[0]);
    if app.available_update.is_some() {
        render_update_banner(frame, app, chunks[1]);
    }
    render_mode_tabs(frame, app, chunks[2]);

    if let Some(domain) = app.active_swarm_domain() {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(chunks[3]);
        render_transcript(frame, app, body[0]);
        render_swarm_sidebar(frame, app, domain, body[1]);
    } else {
        render_transcript(frame, app, chunks[3]);
    }

    render_input(frame, app, chunks[4]);
    render_footer(frame, app, chunks[5]);

    if app.pending_approval.is_some() {
        render_approval_modal(frame, app, area);
    } else if app.show_swarm_hub {
        render_swarm_hub_modal(frame, app, area);
    } else if app.show_help {
        render_help_modal(frame, area);
    }
}

fn render_update_banner(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref update) = app.available_update else {
        return;
    };
    let text = format!(
        " UPDATE AVAILABLE: v{} is outdated -> v{} ready! Press [F5] or type /update to upgrade now ",
        crate::updater::CURRENT_VERSION,
        update.version
    );
    let banner = Paragraph::new(text).style(Theme::update_banner());
    frame.render_widget(banner, area);
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let settings = app.config.active_settings();
    let workspace = app
        .config
        .workspace_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".");

    let spinner = if app.is_busy {
        format!(" [{} running]", SPINNER[app.tick_count % SPINNER.len()])
    } else {
        String::new()
    };

    let mut spans = vec![
        Span::styled(" ECLIVEN CODE ", Theme::brand()),
        Span::raw(format!(
            "| {}/{} | dir: {} | approval: {:?} | tok: {}/{}{}",
            app.config.active_provider,
            settings.model,
            workspace,
            app.config.approval_mode,
            app.total_input_tokens,
            app.total_output_tokens,
            spinner
        )),
    ];

    if let Some(ref update) = app.available_update {
        spans.push(Span::styled(
            format!(" | [Update v{} available: /update]", update.version),
            Theme::status_running(),
        ));
    }

    let line = Line::from(spans);

    let bar = Paragraph::new(line).style(Theme::header_bar());
    frame.render_widget(bar, area);
}

fn render_mode_tabs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chat_style = if app.active_section == WorkspaceSection::Chat {
        Theme::tab_active_chat()
    } else {
        Theme::tab_inactive()
    };
    let code_style = if app.active_section == WorkspaceSection::SwarmCode {
        Theme::tab_active_code()
    } else {
        Theme::tab_inactive()
    };
    let sec_style = if app.active_section == WorkspaceSection::SwarmSecurity {
        Theme::tab_active_security()
    } else {
        Theme::tab_inactive()
    };

    let sec_label = format!(" [3] Swarm: Security ({}) ", app.security_sub_mode.as_str());

    let line = Line::from(vec![
        Span::styled(" Sections: ", Theme::system_text()),
        Span::styled(" [1] Chat ", chat_style),
        Span::raw("  "),
        Span::styled(" [2] Swarm: Code ", code_style),
        Span::raw("  "),
        Span::styled(sec_label, sec_style),
        Span::styled("   (F2: Tab | F3: Hub | F4: Sec Mode)", Theme::footer_hint()),
    ]);

    let bar = Paragraph::new(line).style(Theme::tab_bar());
    frame.render_widget(bar, area);
}

fn render_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let active_section = app.active_section;
    let active_domain = app.active_swarm_domain();
    let border_style = match active_section {
        WorkspaceSection::Chat => Theme::border_inactive(),
        WorkspaceSection::SwarmCode => Theme::border_code(),
        WorkspaceSection::SwarmSecurity => Theme::border_security(),
    };

    let state = app.active_state_mut();
    let scope_suffix = state
        .target_scope
        .as_deref()
        .map(|s| format!(" [scope: {s}]"))
        .unwrap_or_default();

    let title = match active_domain {
        None => " Conversation ".to_string(),
        Some(SwarmDomain::Code) => {
            format!(" Swarm: Code (Best-of-3 -> CHIEF){scope_suffix} ")
        }
        Some(SwarmDomain::Security(sub)) => {
            format!(" Swarm: Security — {}{scope_suffix} ", sub.display_name())
        }
    };

    let mut lines: Vec<Line<'_>> = Vec::new();

    for entry in &state.entries {
        match entry {
            ChatEntry::User(text) => {
                lines.push(Line::from(Span::styled("You:", Theme::user_label())));
                for l in text.lines() {
                    lines.push(Line::from(format!("  {l}")));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::Assistant(text) => {
                lines.push(Line::from(Span::styled(
                    "Ecliven:",
                    Theme::assistant_label(),
                )));
                for l in text.lines() {
                    lines.push(Line::from(format!("  {l}")));
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
                    Span::styled(format!("[{badge}] "), Theme::swarm_badge(*domain)),
                    Span::styled(format!("{agent_name}:"), Theme::tool_label()),
                ]));
                for l in content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {l}"),
                        Theme::system_text(),
                    )));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::SwarmLead {
                domain,
                badge,
                content,
            } => {
                let heading = if matches!(domain, SwarmDomain::Code) {
                    format!("[{badge}] Combined Best Solution:")
                } else {
                    format!("[{badge}] Consolidated Security Report:")
                };
                lines.push(Line::from(Span::styled(heading, Theme::swarm_badge(*domain))));
                for l in content.lines() {
                    lines.push(Line::from(format!("  {l}")));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::ToolCall { name, summary } => {
                lines.push(Line::from(vec![
                    Span::styled(format!("[tool:{name}] "), Theme::tool_label()),
                    Span::raw(summary.clone()),
                ]));
            }
            ChatEntry::ToolResult {
                name,
                output,
                diff,
                is_error,
            } => {
                let status = if *is_error { "error" } else { "ok" };
                lines.push(Line::from(Span::styled(
                    format!("  -> {name} ({status})"),
                    if *is_error {
                        Theme::error_text()
                    } else {
                        Theme::system_text()
                    },
                )));
                if let Some(diff_text) = diff {
                    for dl in diff_text.lines() {
                        let style = if dl.starts_with('+') && !dl.starts_with("+++") {
                            Theme::diff_add()
                        } else if dl.starts_with('-') && !dl.starts_with("---") {
                            Theme::diff_del()
                        } else if dl.starts_with("@@") {
                            Theme::diff_hunk()
                        } else {
                            Theme::system_text()
                        };
                        lines.push(Line::from(Span::styled(format!("     {dl}"), style)));
                    }
                } else {
                    for ol in output.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("     {ol}"),
                            Theme::system_text(),
                        )));
                    }
                }
                lines.push(Line::from(""));
            }
            ChatEntry::System(text) => {
                for l in text.lines() {
                    lines.push(Line::from(Span::styled(
                        l.to_string(),
                        Theme::system_text(),
                    )));
                }
                lines.push(Line::from(""));
            }
            ChatEntry::Error(text) => {
                lines.push(Line::from(Span::styled(
                    format!("Error: {text}"),
                    Theme::error_text(),
                )));
                lines.push(Line::from(""));
            }
        }
    }

    if !state.streaming_buffer.is_empty() {
        let label = match active_domain {
            Some(SwarmDomain::Code) => "[CHIEF] Combining best code (streaming):".to_string(),
            Some(domain) => format!("[{}] Synthesizing (streaming):", domain.lead_badge()),
            None => "Ecliven (streaming):".to_string(),
        };
        lines.push(Line::from(Span::styled(label, Theme::assistant_label())));
        for l in state.streaming_buffer.lines() {
            lines.push(Line::from(format!("  {l}")));
        }
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(inner_height) as u16;
    if state.auto_scroll || state.scroll_offset > max_scroll {
        state.scroll_offset = max_scroll;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let paragraph = Paragraph::new(lines)
        .block(block)
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
        .unwrap_or("(full workspace)");
    lines.push(Line::from(vec![
        Span::styled("Scope: ", Theme::tool_label()),
        Span::raw(scope_text),
    ]));

    if domain.is_security() {
        lines.push(Line::from(Span::styled(
            "F4: switch Defense <-> Threat Model",
            Theme::footer_hint(),
        )));
    }

    lines.push(Line::from(""));
    let team_heading = if matches!(domain, SwarmDomain::Code) {
        "Candidate Coders (Same Task):"
    } else {
        "Parallel Security Specialists:"
    };
    lines.push(Line::from(Span::styled(team_heading, Theme::tool_label())));

    for worker in &state.roster {
        let (status_label, status_style) = match &worker.status {
            SwarmAgentStatus::Idle => ("idle".to_string(), Theme::system_text()),
            SwarmAgentStatus::Running { active_tool } => {
                let detail = active_tool
                    .as_deref()
                    .map(|t| format!("{spin} {t}"))
                    .unwrap_or_else(|| format!("{spin} running"));
                (detail, Theme::status_running())
            }
            SwarmAgentStatus::Done => ("done".to_string(), Theme::status_done()),
            SwarmAgentStatus::Failed(_) => ("error".to_string(), Theme::error_text()),
        };

        lines.push(Line::from(vec![
            Span::styled(format!("[{}] ", worker.badge), Theme::swarm_badge(domain)),
            Span::raw(format!("{} ", worker.name)),
            Span::styled(format!("({status_label})"), status_style),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", worker.summary),
            Theme::system_text(),
        )));
    }

    lines.push(Line::from(""));
    let lead_heading = if matches!(domain, SwarmDomain::Code) {
        "Master Agent (Best-Code Combiner):"
    } else {
        "Security Coordinator:"
    };
    lines.push(Line::from(Span::styled(lead_heading, Theme::tool_label())));

    let (lead_label, lead_style) = match &state.lead_status {
        SwarmAgentStatus::Idle => ("idle".to_string(), Theme::system_text()),
        SwarmAgentStatus::Running { active_tool } => {
            let detail = active_tool
                .as_deref()
                .map(|t| format!("{spin} {t}"))
                .unwrap_or_else(|| format!("{spin} combining"));
            (detail, Theme::status_running())
        }
        SwarmAgentStatus::Done => ("done".to_string(), Theme::status_done()),
        SwarmAgentStatus::Failed(_) => ("error".to_string(), Theme::error_text()),
    };

    lines.push(Line::from(vec![
        Span::styled(
            format!("[{}] ", domain.lead_badge()),
            Theme::swarm_badge(domain),
        ),
        Span::styled(format!("({lead_label})"), lead_style),
    ]));

    let border_style = match domain {
        SwarmDomain::Code => Theme::border_code(),
        SwarmDomain::Security(_) => Theme::border_security(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Swarm Roster ");

    let panel = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

fn render_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = if app.is_busy {
        format!(" {} running... ", app.active_section.label())
    } else {
        match app.active_swarm_domain() {
            None => " Prompt (Enter: Send | Ctrl+J: Newline | /swarm) ".to_string(),
            Some(SwarmDomain::Code) => {
                " Swarm: Code Prompt (3 coders -> CHIEF combines best code | /scope) ".to_string()
            }
            Some(SwarmDomain::Security(SecuritySubMode::Defense)) => {
                " Security [Defensa] Prompt (Audit folder/code for vulns | /scope <dir> | F4) "
                    .to_string()
            }
            Some(SwarmDomain::Security(SecuritySubMode::ThreatModel)) => {
                " Security [Threat Model] Prompt (STRIDE / Config / Deps | /scope <dir> | F4) "
                    .to_string()
            }
        }
    };

    let border = if app.is_busy {
        Theme::border_inactive()
    } else {
        match app.active_section {
            WorkspaceSection::Chat => Theme::border_active(),
            WorkspaceSection::SwarmCode => Theme::border_code(),
            WorkspaceSection::SwarmSecurity => Theme::border_security(),
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(title);

    let paragraph = Paragraph::new(app.input.as_str())
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);

    if !app.is_busy && app.pending_approval.is_none() && !app.show_help && !app.show_swarm_hub {
        let before_cursor = &app.input[..app.cursor_pos];
        let lines: Vec<&str> = before_cursor.split('\n').collect();
        let row = lines.len().saturating_sub(1) as u16;
        let col = lines.last().map(|l| l.chars().count()).unwrap_or(0) as u16;

        let cx = (area.x + 1 + col).min(area.x + area.width.saturating_sub(2));
        let cy = (area.y + 1 + row).min(area.y + area.height.saturating_sub(2));
        frame.set_cursor_position((cx, cy));
    }
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let text = if app.pending_approval.is_some() {
        " Approval required: [Y/Enter] Approve  |  [N/Esc] Deny  |  [Ctrl+Q] Quit"
    } else if app.show_swarm_hub {
        " Swarm Hub: [1] Code Swarm  |  [2] Security: Defensa  |  [3] Security: Threat Model  |  [0] Chat  |  [Esc] Close"
    } else {
        " F2: Tab | F3: Swarm Hub | F4: Sec Sub-mode | /scope <dir> | Ctrl+L: Clear | F1: Help | Ctrl+Q: Quit"
    };

    let footer = Paragraph::new(text).style(Theme::footer_hint());
    frame.render_widget(footer, area);
}

fn render_swarm_hub_modal(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let modal_area = centered_rect(88, 70, area);
    frame.render_widget(Clear, modal_area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::brand())
        .title(" Swarm Hub — Apartados Independientes ");
    let inner = outer.inner(modal_area);
    frame.render_widget(outer, modal_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(3)])
        .split(inner);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[0]);

    let code_active = if app.active_section == WorkspaceSection::SwarmCode {
        " [ACTIVE]"
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
        Line::from("  - [CLEAN]  Clean Coder (arquitectura y claridad)"),
        Line::from("  - [PERF]   Systems & Perf (velocidad y eficiencia)"),
        Line::from("  - [ROBUST] Edge-Case & Tests (seguridad y casos borde)"),
        Line::from(""),
        Line::from(Span::styled(
            "  => [CHIEF] Agente Mayor:",
            Theme::tool_label(),
        )),
        Line::from("     Evalua las 3 propuestas y combina el MEJOR"),
        Line::from("     codigo posible en una sola solucion final."),
    ];
    let code_card = Paragraph::new(code_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_code())
                .title(" Apartado 1: Codigo (Best-of-3) "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(code_card, cards[0]);

    let def_active = if app.active_section == WorkspaceSection::SwarmSecurity
        && app.security_sub_mode == SecuritySubMode::Defense
    {
        " [ACTIVE]"
    } else {
        ""
    };
    let threat_active = if app.active_section == WorkspaceSection::SwarmSecurity
        && app.security_sub_mode == SecuritySubMode::ThreatModel
    {
        " [ACTIVE]"
    } else {
        ""
    };
    let sec_lines = vec![
        Line::from(Span::styled(
            format!("[2] DEFENSA (Codigo / Carpeta){def_active}"),
            Theme::swarm_badge(SwarmDomain::security_defense()),
        )),
        Line::from("  Trabaja sobre una carpeta o codigo fuente (/scope):"),
        Line::from("  - [SURFACE] + [AUDIT] (OWASP/CWE) + [PATCH]"),
        Line::from("  => [DEFENSE LEAD]: vulnerabilidades y parches"),
        Line::from(""),
        Line::from(Span::styled(
            format!("[3] THREAT MODEL & HARDENING{threat_active}"),
            Theme::swarm_badge(SwarmDomain::security_threat()),
        )),
        Line::from("  Postura defensiva y arquitectura segura:"),
        Line::from("  - [STRIDE] + [CONFIG] (Deps/CI) + [BLUETEAM]"),
        Line::from("  => [SEC ARCHITECT]: matriz STRIDE y plan"),
    ];
    let sec_card = Paragraph::new(sec_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_security())
                .title(" Apartado 2: Ciberseguridad (2 Modos) "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(sec_card, cards[1]);

    let bottom = Paragraph::new(
        "[1] Swarm: Codigo  |  [2] Sec: Defensa  |  [3] Sec: Threat Model  |  [0] Chat Normal  |  [Esc] Cerrar",
    )
    .style(Theme::brand())
    .block(Block::default().borders(Borders::TOP));
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
        "Press [Y] or [Enter] to approve, [N] or [Esc] to deny.",
        Theme::brand(),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::modal_border())
        .title(" Tool Approval Required ");

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, modal_area);
}

fn render_help_modal(frame: &mut Frame<'_>, area: Rect) {
    let modal_area = centered_rect(76, 70, area);
    frame.render_widget(Clear, modal_area);

    let help = "\
Keyboard Shortcuts (Standard Ctrl+key):
  F2            Cycle tabs (Chat -> Swarm: Code -> Swarm: Security)
  F3            Open Swarm Hub (Code vs Security 2-mode picker)
  F4            Toggle Security sub-mode (Defensa <-> Threat Model)
  Enter         Send prompt
  Ctrl+J        Insert newline
  Ctrl+U        Clear input box
  Ctrl+L        Clear active section history
  Up / Down     Navigate prompt history
  PgUp / PgDn   Scroll transcript
  F1            Toggle this help panel
  Ctrl+Q        Quit Ecliven Code

Slash Commands:
  /swarm | /swarm code [folder]
  /swarm security defense [folder] | /swarm security threat [folder]
  /swarm off | /scope <folder|off>
  /provider <anthropic|openai|openrouter|ollama> | /model <name>
  /approval <ask|auto-edit|auto-all> | /status | /clear | /quit

Press F1 or Esc to close.";

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_active())
        .title(" Ecliven Code Help ");

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
