use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::layout::Slot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CustomizedKeyAction {
    // --- Configurable actions ---
    /// Open or close the help popup. Active in global scope.
    ToggleHelp,
    /// Show the detail popup for the selected log entry.
    /// Active in global scope but only fires when the log pane is focused.
    ShowInfo,
    /// A non-match
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StaticKeyAction {
    /// Quit the app
    Quit,
    /// A non-match
    None,
    /// Scroll up
    ScrollUp,
    /// Scroll down
    ScrollDown,
}

pub fn match_key(key: &KeyEvent, _focus: &Slot) -> (StaticKeyAction, CustomizedKeyAction) {
    let static_key = match (key.code, key.modifiers) {
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => StaticKeyAction::Quit,
        (KeyCode::Up | KeyCode::Char('k'), _) => StaticKeyAction::ScrollUp,
        (KeyCode::Down | KeyCode::Char('j'), _) => StaticKeyAction::ScrollDown,
        // ... more arms as needed
        _ => StaticKeyAction::None,
    };

    let custom_key = CustomizedKeyAction::None;

    (static_key, custom_key)
}
