pub mod app;
pub mod eclipse;
pub mod keybindings;
pub mod onboarding;
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

use crate::config::preferences::UserPreferences;
use crate::config::AppConfig;
use crate::swarm::domain::{SecuritySubMode, SwarmDomain};
use crate::tui::app::{App, UiEvent, WorkspaceSection};
use crate::tui::keybindings::{map_key, Action};
use crate::updater::{self, UpdateStatus};

pub async fn run_tui(
    config: AppConfig,
    initial_swarm: Option<SwarmDomain>,
    initial_scope: Option<String>,
    force_setup: bool,
) -> Result<()> {
    install_panic_hook();
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let prefs = UserPreferences::load_default();
    let prefs_path = UserPreferences::default_path();
    let mut app = App::with_preferences(
        config,
        initial_swarm,
        initial_scope,
        prefs,
        prefs_path,
        force_setup,
    );
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
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiEvent>();
    let mut tick = tokio::time::interval(Duration::from_millis(65));

    let update_tx = ui_tx.clone();
    tokio::spawn(async move {
        if let Ok(UpdateStatus::Available { latest, .. }) = updater::check_for_update().await {
            let _ = update_tx.send(UiEvent::UpdateAvailable(latest));
        }
    });

    app.prefetch_models(ui_tx.clone());

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            _ = tick.tick() => {
                let picker_loading = app.model_picker.as_ref().is_some_and(|p| p.loading);
                if app.is_busy || app.is_landing_view() || app.onboarding.is_some() || picker_loading {
                    app.tick_count = app.tick_count.wrapping_add(1);
                }
            }
            maybe_ui_event = ui_rx.recv() => {
                if let Some(ev) = maybe_ui_event {
                    app.handle_ui_event(ev);
                }
            }
            maybe_term_event = reader.next() => {
                match maybe_term_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        let action = map_key(key, app.modal_state());
                        match action {
                            Action::Quit => app.should_quit = true,
                            Action::ClearSession => app.clear_session().await,
                            Action::ClearInput => app.clear_input(),
                            Action::Submit => app.submit_input(ui_tx.clone()).await,
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
                            Action::CycleSection => app.cycle_section(),
                            Action::ToggleSwarmHub => app.show_swarm_hub = !app.show_swarm_hub,
                            Action::ToggleSecuritySubMode => app.toggle_security_sub_mode(),
                            Action::QuickUpdate => {
                                if app.available_update.is_some() {
                                    app.trigger_quick_update(ui_tx.clone());
                                }
                            }
                            Action::OpenFolderPicker => app.open_folder_picker(),
                            Action::OpenModelPicker => app.open_model_picker(ui_tx.clone()),
                            Action::ModelPickerUp => app.model_picker_up(),
                            Action::ModelPickerDown => app.model_picker_down(),
                            Action::ModelPickerSelect => app.model_picker_select(),
                            Action::ModelPickerRefresh => app.refresh_model_picker(ui_tx.clone()),
                            Action::ModelPickerChar(c) => app.model_picker_char(c),
                            Action::ModelPickerBackspace => app.model_picker_backspace(),
                            Action::SelectChatSection => app.switch_section(WorkspaceSection::Chat),
                            Action::SelectSwarmCode => app.switch_section(WorkspaceSection::SwarmCode),
                            Action::SelectSwarmSecurityDefense => {
                                app.set_security_sub_mode(SecuritySubMode::Defense)
                            }
                            Action::SelectSwarmSecurityThreat => {
                                app.set_security_sub_mode(SecuritySubMode::ThreatModel)
                            }
                            Action::CloseModal => app.close_modals(),
                            Action::ApproveYes => app.resolve_approval(true),
                            Action::ApproveNo => app.resolve_approval(false),
                            Action::OnboardingConfirm => app.onboarding_confirm(),
                            Action::OnboardingCancel => app.onboarding_cancel(),
                            Action::OnboardingUp => app.onboarding_up(),
                            Action::OnboardingDown => app.onboarding_down(),
                            Action::OnboardingOpenSubdir => app.onboarding_open_subdir(),
                            Action::OnboardingParent => app.onboarding_parent(),
                            Action::OnboardingToggleManualPath => app.onboarding_toggle_manual_path(),
                            Action::OnboardingTelemetryOn => app.onboarding_set_telemetry(true),
                            Action::OnboardingTelemetryOff => app.onboarding_set_telemetry(false),
                            Action::OnboardingInputChar(c) => app.onboarding_input_char(c),
                            Action::OnboardingInputBackspace => app.onboarding_input_backspace(),
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
