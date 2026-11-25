pub mod app;
pub mod keybindings;
pub mod theme;
pub mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::agent::AgentEvent;
use crate::config::AppConfig;
use crate::tui::app::App;
use crate::tui::keybindings::{map_key, Action};

pub async fn run_tui(config: AppConfig) -> Result<()> {
    install_panic_hook();
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let run_result = event_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    run_result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut reader = EventStream::new();
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let mut tick = tokio::time::interval(Duration::from_millis(120));

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            _ = tick.tick() => {
                if app.is_busy {
                    app.tick_count = app.tick_count.wrapping_add(1);
                }
            }
            maybe_agent_event = agent_rx.recv() => {
                if let Some(ev) = maybe_agent_event {
                    app.handle_agent_event(ev);
                }
            }
            maybe_term_event = reader.next() => {
                match maybe_term_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        let action = map_key(key, app.pending_approval.is_some());
                        match action {
                            Action::Quit => app.should_quit = true,
                            Action::ClearSession => app.clear_session().await,
                            Action::ClearInput => app.clear_input(),
                            Action::Submit => app.submit_input(agent_tx.clone()).await,
                            Action::InsertNewline => app.insert_char('\n'),
                            Action::InsertChar(c) => app.insert_char(c),
                            Action::Backspace => app.backspace(),
                            Action::Delete => app.delete_char(),
                            Action::MoveLeft => app.move_left(),
                            Action::MoveRight => app.move_right(),
                            Action::MoveHome => app.move_home(),
                            Action::MoveEnd => app.move_end(),
                            Action::HistoryPrev => app.history_prev(),
                            Action::HistoryNext => app.history_next(),
                            Action::ScrollUp => app.scroll_up(5),
                            Action::ScrollDown => app.scroll_down(5),
                            Action::ToggleHelp => app.show_help = !app.show_help,
                            Action::ApproveYes => app.resolve_approval(true),
                            Action::ApproveNo => app.resolve_approval(false),
                            Action::None => {}
                        }
                    }
                    Some(Ok(Event::Mouse(mouse))) => match mouse.kind {
                        MouseEventKind::ScrollUp => app.scroll_up(3),
                        MouseEventKind::ScrollDown => app.scroll_down(3),
                        _ => {}
                    },
                    Some(Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
