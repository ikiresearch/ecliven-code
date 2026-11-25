use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    ApproveYes,
    ApproveNo,
    None,
}

pub fn map_key(key: KeyEvent, awaiting_approval: bool) -> Action {
    if awaiting_approval {
        return match (key.code, key.modifiers) {
            (KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::NONE)
            | (KeyCode::Enter, KeyModifiers::NONE) => Action::ApproveYes,
            (KeyCode::Char('n') | KeyCode::Char('N'), KeyModifiers::NONE)
            | (KeyCode::Esc, KeyModifiers::NONE) => Action::ApproveNo,
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => Action::Quit,
            (KeyCode::PageUp, _) => Action::ScrollUp,
            (KeyCode::PageDown, _) => Action::ScrollDown,
            _ => Action::None,
        };
    }

    match (key.code, key.modifiers) {
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
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(c),
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_standard_ctrl_shortcuts() {
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(map_key(quit, false), Action::Quit);

        let clear = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert_eq!(map_key(clear, false), Action::ClearSession);

        let newline = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(map_key(newline, false), Action::InsertNewline);
    }

    #[test]
    fn maps_approval_modal_keys() {
        let yes = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(map_key(yes, true), Action::ApproveYes);

        let no = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_key(no, true), Action::ApproveNo);
    }
}
