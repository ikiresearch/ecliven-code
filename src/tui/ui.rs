use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{App, ChatEntry};
use crate::tui::theme::Theme;

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, chunks[0]);
    render_transcript(frame, app, chunks[1]);
    render_input(frame, app, chunks[2]);
    render_footer(frame, app, chunks[3]);

    if app.pending_approval.is_some() {
        render_approval_modal(frame, app, area);
    } else if app.show_help {
        render_help_modal(frame, area);
    }
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
        format!(" [{} thinking]", SPINNER[app.tick_count % SPINNER.len()])
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(" ECLIVEN CODE ", Theme::brand()),
        Span::raw(format!(
            "| {}/{} | dir: {} | mode: {:?} | tok: {}/{}{}",
            app.config.active_provider,
            settings.model,
            workspace,
            app.config.approval_mode,
            app.total_input_tokens,
            app.total_output_tokens,
            spinner
        )),
    ]);

    let bar = Paragraph::new(line).style(Theme::header_bar());
    frame.render_widget(bar, area);
}

fn render_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    for entry in &app.entries {
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
                    lines.push(Line::from(Span::styled(l.to_string(), Theme::system_text())));
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

    if !app.streaming_buffer.is_empty() {
        lines.push(Line::from(Span::styled(
            "Ecliven (streaming):",
            Theme::assistant_label(),
        )));
        for l in app.streaming_buffer.lines() {
            lines.push(Line::from(format!("  {l}")));
        }
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(inner_height) as u16;
    if app.auto_scroll || app.scroll_offset > max_scroll {
        app.scroll_offset = max_scroll;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_inactive())
        .title(" Conversation ");

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll_offset, 0));

    frame.render_widget(paragraph, area);
}

fn render_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = if app.is_busy {
        " Working... (wait for response) "
    } else {
        " Prompt (Enter: Send | Ctrl+J: Newline | /help) "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.is_busy {
            Theme::border_inactive()
        } else {
            Theme::border_active()
        })
        .title(title);

    let paragraph = Paragraph::new(app.input.as_str())
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);

    if !app.is_busy && app.pending_approval.is_none() && !app.show_help {
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
    } else {
        " Ctrl+Q: Quit | Ctrl+L: Clear | Ctrl+U: Clear input | PgUp/PgDn: Scroll | F1: Help | /provider /model"
    };

    let footer = Paragraph::new(text).style(Theme::footer_hint());
    frame.render_widget(footer, area);
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
    let modal_area = centered_rect(70, 60, area);
    frame.render_widget(Clear, modal_area);

    let help = "\
Keyboard Shortcuts (Standard Ctrl+key):
  Enter         Send prompt
  Ctrl+J        Insert newline
  Ctrl+U        Clear input box
  Ctrl+L        Clear conversation history
  Up / Down     Navigate prompt history
  PgUp / PgDn   Scroll transcript
  F1            Toggle this help panel
  Ctrl+Q        Quit Ecliven Code

Slash Commands:
  /provider <anthropic|openai|openrouter|ollama>
  /model <model-name>
  /approval <ask|auto-edit|auto-all>
  /status | /clear | /help | /quit

Press F1 again to close.";

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
