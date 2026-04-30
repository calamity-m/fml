use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::layout::Slot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpSection {
    Global,
    LogPane,
    QueryBox,
    SourceSelector,
    HelpPopup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyActionHint {
    pub title: &'static str,
    pub label: &'static str,
    pub section: HelpSection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CustomizedKeyAction {
    // --- Configurable actions ---
    /// Open or close the help popup. Active in global scope.
    ToggleHelp,
    /// Open or close the source selector popup. Active in global scope.
    ToggleSourceSelector,
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

pub const ACTION_HINTS: &[KeyActionHint] = &[
    KeyActionHint {
        title: "Quit",
        label: "ctrl+c",
        section: HelpSection::Global,
    },
    KeyActionHint {
        title: "Cycle focus",
        label: "tab",
        section: HelpSection::Global,
    },
    KeyActionHint {
        title: "Help",
        label: "?",
        section: HelpSection::Global,
    },
    KeyActionHint {
        title: "Sources",
        label: "ctrl+s",
        section: HelpSection::Global,
    },
    KeyActionHint {
        title: "Move up",
        label: "k / up",
        section: HelpSection::LogPane,
    },
    KeyActionHint {
        title: "Move down",
        label: "j / down",
        section: HelpSection::LogPane,
    },
    KeyActionHint {
        title: "Jump head",
        label: "g / home",
        section: HelpSection::LogPane,
    },
    KeyActionHint {
        title: "Jump tail",
        label: "G / end",
        section: HelpSection::LogPane,
    },
    KeyActionHint {
        title: "Entry detail",
        label: "enter",
        section: HelpSection::LogPane,
    },
    KeyActionHint {
        title: "Edit search",
        label: "type",
        section: HelpSection::QueryBox,
    },
    KeyActionHint {
        title: "Delete",
        label: "backspace",
        section: HelpSection::QueryBox,
    },
    KeyActionHint {
        title: "Toggle row",
        label: "space",
        section: HelpSection::SourceSelector,
    },
    KeyActionHint {
        title: "All sources",
        label: "a",
        section: HelpSection::SourceSelector,
    },
    KeyActionHint {
        title: "No sources",
        label: "n",
        section: HelpSection::SourceSelector,
    },
    KeyActionHint {
        title: "Close",
        label: "esc / enter",
        section: HelpSection::SourceSelector,
    },
    KeyActionHint {
        title: "Close",
        label: "esc / ?",
        section: HelpSection::HelpPopup,
    },
];

pub fn hints_for_section(section: HelpSection) -> impl Iterator<Item = &'static KeyActionHint> {
    ACTION_HINTS
        .iter()
        .filter(move |hint| hint.section == section)
}

pub fn primary_label(title: &str) -> Option<&'static str> {
    ACTION_HINTS
        .iter()
        .find(|hint| hint.title == title)
        .map(|hint| hint.label)
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
        (KeyCode::Char('?'), _) => CustomizedKeyAction::ToggleHelp,
        (KeyCode::Char('s'), m) if m.contains(KeyModifiers::CONTROL) => {
            CustomizedKeyAction::ToggleSourceSelector
        }
        (KeyCode::Char('g') | KeyCode::Home, _) => CustomizedKeyAction::ScrollHead,
        (KeyCode::Char('G') | KeyCode::End, _) => CustomizedKeyAction::ScrollTail,
        _ => CustomizedKeyAction::None,
    };

    (static_key, custom_key)
}
