use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalState {
    None,
    OnboardingTerms,
    OnboardingTelemetry,
    OnboardingWorkspace,
    OnboardingWorkspaceInput,
    Approval,
    Question,
    QuestionInput,
    SwarmHub,
    ModelPicker,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ClearSession,
    ClearInput,
    Submit,
    InsertNewline,
    InsertChar(char),
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    HistoryPrev,
    HistoryNext,
    ScrollUp,
    ScrollDown,
    ToggleHelp,
    CycleSection,
    ToggleSwarmHub,
    ToggleSecuritySubMode,
    QuickUpdate,
    OpenFolderPicker,
    OpenModelPicker,
    ModelPickerUp,
    ModelPickerDown,
    ModelPickerSelect,
    ModelPickerRefresh,
    ModelPickerChar(char),
    ModelPickerBackspace,
    SelectChatSection,
    SelectSwarmCode,
    SelectSwarmSecurityDefense,
    SelectSwarmSecurityThreat,
    CloseModal,
    ApproveYes,
    ApproveNo,
    QuestionUp,
    QuestionDown,
    QuestionToggle,
    QuestionConfirm,
    QuestionCancel,
    QuestionDigit(usize),
    QuestionInputChar(char),
    QuestionInputBackspace,
    QuestionFocusOptions,
    OnboardingConfirm,
    OnboardingCancel,
    OnboardingUp,
    OnboardingDown,
    OnboardingOpenSubdir,
    OnboardingParent,
    OnboardingToggleManualPath,
    OnboardingTelemetryOn,
    OnboardingTelemetryOff,
    OnboardingInputChar(char),
    OnboardingInputBackspace,
    None,
}

pub fn map_key(key: KeyEvent, modal: ModalState) -> Action {
    match modal {
        ModalState::OnboardingTerms => match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::NONE)
            | (KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::NONE) => {
                Action::OnboardingConfirm
            }
            (KeyCode::Esc, _)
            | (KeyCode::Char('n') | KeyCode::Char('N'), KeyModifiers::NONE)
            | (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::OnboardingCancel,
            _ => Action::None,
        },
        ModalState::OnboardingTelemetry => match (key.code, key.modifiers) {
            (KeyCode::Up | KeyCode::Left, _) | (KeyCode::Char(' '), KeyModifiers::NONE) => {
                Action::OnboardingUp
            }
            (KeyCode::Down | KeyCode::Right, _) => Action::OnboardingDown,
            (KeyCode::Char('1') | KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::NONE) => {
                Action::OnboardingTelemetryOn
            }
            (KeyCode::Char('2') | KeyCode::Char('n') | KeyCode::Char('N'), KeyModifiers::NONE) => {
                Action::OnboardingTelemetryOff
            }
            (KeyCode::Enter, KeyModifiers::NONE) => Action::OnboardingConfirm,
            (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                Action::OnboardingCancel
            }
            _ => Action::None,
        },
        ModalState::OnboardingWorkspace => match (key.code, key.modifiers) {
            (KeyCode::Up, _) => Action::OnboardingUp,
            (KeyCode::Down, _) => Action::OnboardingDown,
            (KeyCode::Right | KeyCode::Tab, _) => Action::OnboardingOpenSubdir,
            (KeyCode::Backspace | KeyCode::Left, _) => Action::OnboardingParent,
            (KeyCode::Char('p') | KeyCode::Char('P') | KeyCode::Char('e') | KeyCode::Char('E'), KeyModifiers::NONE) => {
                Action::OnboardingToggleManualPath
            }
            (KeyCode::Enter, KeyModifiers::NONE) => Action::OnboardingConfirm,
            (KeyCode::Esc, _) => Action::OnboardingCancel,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            _ => Action::None,
        },
        ModalState::OnboardingWorkspaceInput => match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::NONE) => Action::OnboardingConfirm,
            (KeyCode::Esc, _) => Action::OnboardingToggleManualPath,
            (KeyCode::Backspace, _) => Action::OnboardingInputBackspace,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::OnboardingInputChar(c)
            }
            _ => Action::None,
        },
        ModalState::Approval => match (key.code, key.modifiers) {
            (KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::NONE)
            | (KeyCode::Enter, KeyModifiers::NONE) => Action::ApproveYes,
            (KeyCode::Char('n') | KeyCode::Char('N'), KeyModifiers::NONE)
            | (KeyCode::Esc, KeyModifiers::NONE) => Action::ApproveNo,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::PageUp, _) => Action::ScrollUp,
            (KeyCode::PageDown, _) => Action::ScrollDown,
            _ => Action::None,
        },
        ModalState::Question => match (key.code, key.modifiers) {
            (KeyCode::Up, _) => Action::QuestionUp,
            (KeyCode::Down, _) => Action::QuestionDown,
            (KeyCode::Char(' '), KeyModifiers::NONE) => Action::QuestionToggle,
            (KeyCode::Enter, KeyModifiers::NONE) => Action::QuestionConfirm,
            (KeyCode::Esc, _) => Action::QuestionCancel,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::Char(c @ '1'..='9'), KeyModifiers::NONE) => {
                Action::QuestionDigit((c as u8 - b'1') as usize)
            }
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::QuestionInputChar(c)
            }
            (KeyCode::Backspace, _) => Action::QuestionInputBackspace,
            _ => Action::None,
        },
        ModalState::QuestionInput => match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::NONE) => Action::QuestionConfirm,
            (KeyCode::Esc, _) => Action::QuestionFocusOptions,
            (KeyCode::Up, _) => Action::QuestionFocusOptions,
            (KeyCode::Backspace, _) => Action::QuestionInputBackspace,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::QuestionInputChar(c)
            }
            _ => Action::None,
        },
        ModalState::SwarmHub => match (key.code, key.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Action::SelectSwarmCode,
            (KeyCode::Char('2'), KeyModifiers::NONE) => Action::SelectSwarmSecurityDefense,
            (KeyCode::Char('3'), KeyModifiers::NONE) => Action::SelectSwarmSecurityThreat,
            (KeyCode::Char('0'), KeyModifiers::NONE) => Action::SelectChatSection,
            (KeyCode::Esc, _) | (KeyCode::F(3), _) => Action::CloseModal,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            _ => Action::None,
        },
        ModalState::ModelPicker => match (key.code, key.modifiers) {
            (KeyCode::Up, _) => Action::ModelPickerUp,
            (KeyCode::Down, _) => Action::ModelPickerDown,
            (KeyCode::Enter, KeyModifiers::NONE) => Action::ModelPickerSelect,
            (KeyCode::Esc, _) | (KeyCode::F(7), _) => Action::CloseModal,
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => Action::ModelPickerRefresh,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::Backspace, _) => Action::ModelPickerBackspace,
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::ModelPickerChar(c)
            }
            _ => Action::None,
        },
        ModalState::Help => match (key.code, key.modifiers) {
            (KeyCode::Esc, _) | (KeyCode::F(1), _) => Action::CloseModal,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            _ => Action::None,
        },
        ModalState::None => match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::ClearInput,
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => Action::ClearSession,
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearInput,
            (KeyCode::Char('m'), KeyModifiers::CONTROL)
            | (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::OpenModelPicker,
            (KeyCode::Char('j'), KeyModifiers::CONTROL) => Action::InsertNewline,
            (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Enter, KeyModifiers::ALT) => {
                Action::InsertNewline
            }
            (KeyCode::Enter, KeyModifiers::NONE) => Action::Submit,
            (KeyCode::Backspace, _) => Action::Backspace,
            (KeyCode::Delete, _) => Action::Delete,
            (KeyCode::Left, _) => Action::MoveLeft,
            (KeyCode::Right, _) => Action::MoveRight,
            (KeyCode::Home, _) => Action::MoveHome,
            (KeyCode::End, _) => Action::MoveEnd,
            (KeyCode::Up, _) => Action::HistoryPrev,
            (KeyCode::Down, _) => Action::HistoryNext,
            (KeyCode::PageUp, _) => Action::ScrollUp,
            (KeyCode::PageDown, _) => Action::ScrollDown,
            (KeyCode::F(1), _) => Action::ToggleHelp,
            (KeyCode::F(2), _) => Action::CycleSection,
            (KeyCode::F(3), _) => Action::ToggleSwarmHub,
            (KeyCode::F(4), _) => Action::ToggleSecuritySubMode,
            (KeyCode::F(5), _) => Action::QuickUpdate,
            (KeyCode::F(6), _) => Action::OpenFolderPicker,
            (KeyCode::F(7), _) => Action::OpenModelPicker,
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(c),
            _ => Action::None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_standard_ctrl_shortcuts() {
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(map_key(quit, ModalState::None), Action::Quit);

        let clear = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert_eq!(map_key(clear, ModalState::None), Action::ClearSession);

        let newline = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(map_key(newline, ModalState::None), Action::InsertNewline);

        let cycle = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
        assert_eq!(map_key(cycle, ModalState::None), Action::CycleSection);

        let hub = KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE);
        assert_eq!(map_key(hub, ModalState::None), Action::ToggleSwarmHub);

        let sub = KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE);
        assert_eq!(map_key(sub, ModalState::None), Action::ToggleSecuritySubMode);

        let folder = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
        assert_eq!(map_key(folder, ModalState::None), Action::OpenFolderPicker);
    }

    #[test]
    fn maps_onboarding_and_approval_modal_keys() {
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            map_key(enter, ModalState::OnboardingTerms),
            Action::OnboardingConfirm
        );

        let one = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE);
        assert_eq!(
            map_key(one, ModalState::OnboardingTelemetry),
            Action::OnboardingTelemetryOn
        );

        let two = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(
            map_key(two, ModalState::OnboardingTelemetry),
            Action::OnboardingTelemetryOff
        );

        let yes = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(map_key(yes, ModalState::Approval), Action::ApproveYes);
    }
}
