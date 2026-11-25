use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalState {
    None,
    Approval,
    SwarmHub,
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
    SelectChatSection,
    SelectSwarmCode,
    SelectSwarmSecurityDefense,
    SelectSwarmSecurityThreat,
    CloseModal,
    ApproveYes,
    ApproveNo,
    None,
}

pub fn map_key(key: KeyEvent, modal: ModalState) -> Action {
    match modal {
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
        ModalState::SwarmHub => match (key.code, key.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Action::SelectSwarmCode,
            (KeyCode::Char('2'), KeyModifiers::NONE) => Action::SelectSwarmSecurityDefense,
            (KeyCode::Char('3'), KeyModifiers::NONE) => Action::SelectSwarmSecurityThreat,
            (KeyCode::Char('0'), KeyModifiers::NONE) => Action::SelectChatSection,
            (KeyCode::Esc, _) | (KeyCode::F(3), _) => Action::CloseModal,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
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
    }

    #[test]
    fn maps_approval_and_swarm_hub_modal_keys() {
        let yes = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(map_key(yes, ModalState::Approval), Action::ApproveYes);

        let no = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_key(no, ModalState::Approval), Action::ApproveNo);

        let code = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE);
        assert_eq!(map_key(code, ModalState::SwarmHub), Action::SelectSwarmCode);

        let sec_def = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(
            map_key(sec_def, ModalState::SwarmHub),
            Action::SelectSwarmSecurityDefense
        );

        let sec_threat = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
        assert_eq!(
            map_key(sec_threat, ModalState::SwarmHub),
            Action::SelectSwarmSecurityThreat
        );
    }
}
