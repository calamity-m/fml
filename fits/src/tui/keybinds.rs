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
    /// Scroll top the top (head)
    ScrollHead,
    /// Scroll to the bottom (tail)
    ScrollTail,
    /// A non-match
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StaticKeyAction {
    /// Quit the app
    Quit,
    /// Cycle the focus
    FocusCycle,
    /// Scroll up
    ScrollUp,
    /// Scroll down
    ScrollDown,
    /// A non-match
    None,
}

pub fn match_key(key: &KeyEvent, _focus: &Slot) -> (StaticKeyAction, CustomizedKeyAction) {
    let static_key = match (key.code, key.modifiers) {
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => StaticKeyAction::Quit,
        (KeyCode::Tab, _) => StaticKeyAction::FocusCycle,
        (KeyCode::Up | KeyCode::Char('k'), _) => StaticKeyAction::ScrollUp,
        (KeyCode::Down | KeyCode::Char('j'), _) => StaticKeyAction::ScrollDown,
        // ... more arms as needed
        _ => StaticKeyAction::None,
    };

    let custom_key = match (key.code, key.modifiers) {
        (KeyCode::Char('g') | KeyCode::Home, _) => CustomizedKeyAction::ScrollHead,
        (KeyCode::Char('G') | KeyCode::End, _) => CustomizedKeyAction::ScrollTail,
        _ => CustomizedKeyAction::None,
    };

    (static_key, custom_key)
}
