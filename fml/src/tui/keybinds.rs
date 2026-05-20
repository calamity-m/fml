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
    /// Cycle the preview pane mode. Active in global scope.
    TogglePreviewMode,
    /// Show the detail popup for the selected log entry.
    /// Active in global scope but only fires when the log pane is focused.
    ShowInfo,
    /// Scroll top the top (head)
    ScrollHead,
    /// Scroll to the bottom (tail)
    ScrollTail,
    /// Toggle select mode: releases mouse capture so the terminal handles
    /// drag-selection and wheel scrollback. Active in global scope.
    ToggleSelectMode,
    /// Yank the selected log entry as JSON via OSC 52. Only fires when the log
    /// pane has focus and no popup is active.
    YankSelectedEntry,
    /// Toggle log-pane line-wrap mode (single-line truncated vs. multi-line
    /// wrapped with hanging-indent continuation). Only fires when the log pane
    /// has focus.
    ToggleLineWrap,
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
        label: "ctrl+c / q",
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
        title: "Preview mode",
        label: "ctrl+p",
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
    KeyActionHint {
        title: "Select mode",
        label: "F2",
        section: HelpSection::Global,
    },
    KeyActionHint {
        title: "Yank entry",
        label: "y",
        section: HelpSection::LogPane,
    },
    KeyActionHint {
        title: "Toggle wrap",
        label: "w",
        section: HelpSection::LogPane,
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
        (KeyCode::Char('q'), _) => StaticKeyAction::Quit,
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
        (KeyCode::Char('p'), m) if m.contains(KeyModifiers::CONTROL) => {
            CustomizedKeyAction::TogglePreviewMode
        }
        (KeyCode::Char('g') | KeyCode::Home, _) => CustomizedKeyAction::ScrollHead,
        (KeyCode::Char('G') | KeyCode::End, _) => CustomizedKeyAction::ScrollTail,
        (KeyCode::F(2), _) => CustomizedKeyAction::ToggleSelectMode,
        (KeyCode::Char('y'), _) => CustomizedKeyAction::YankSelectedEntry,
        (KeyCode::Char('w'), _) => CustomizedKeyAction::ToggleLineWrap,
        _ => CustomizedKeyAction::None,
    };

    (static_key, custom_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_pane_hints_include_toggle_wrap() {
        let hints: Vec<_> = hints_for_section(HelpSection::LogPane).collect();
        assert!(
            hints
                .iter()
                .any(|h| h.title == "Toggle wrap" && h.label == "w"),
            "help popup log-pane section should list toggle wrap binding"
        );
    }

    #[test]
    fn w_key_maps_to_toggle_line_wrap() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (_static, custom) = match_key(
            &KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            &Slot::Main,
        );
        assert_eq!(custom, CustomizedKeyAction::ToggleLineWrap);
    }
}
