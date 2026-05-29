use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::tui::KeybindingsConfig;
use crate::error::FmlError;
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
    /// Fallback display label for reserved / popup-local keys that are not
    /// user-configurable. For hints carrying an `action`, the resolved binding
    /// label takes precedence and this is unused.
    pub label: &'static str,
    pub section: HelpSection,
    /// The configurable action this hint represents, if any. When set, help and
    /// status surfaces display the resolved binding label and hide the hint
    /// entirely when the binding is disabled.
    pub action: Option<CustomizedKeyAction>,
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
    /// Open the field picker for the selected log entry.
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
        label: "ctrl+c",
        section: HelpSection::Global,
        action: None,
    },
    KeyActionHint {
        title: "Help",
        label: "?",
        section: HelpSection::Global,
        action: Some(CustomizedKeyAction::ToggleHelp),
    },
    KeyActionHint {
        title: "Sources",
        label: "ctrl+s",
        section: HelpSection::Global,
        action: Some(CustomizedKeyAction::ToggleSourceSelector),
    },
    KeyActionHint {
        title: "Preview mode",
        label: "ctrl+p",
        section: HelpSection::Global,
        action: Some(CustomizedKeyAction::TogglePreviewMode),
    },
    KeyActionHint {
        title: "Move up",
        label: "k / up",
        section: HelpSection::LogPane,
        action: None,
    },
    KeyActionHint {
        title: "Move down",
        label: "j / down",
        section: HelpSection::LogPane,
        action: None,
    },
    KeyActionHint {
        title: "Jump head",
        label: "g / home",
        section: HelpSection::LogPane,
        action: Some(CustomizedKeyAction::ScrollHead),
    },
    KeyActionHint {
        title: "Jump tail",
        label: "G / end",
        section: HelpSection::LogPane,
        action: Some(CustomizedKeyAction::ScrollTail),
    },
    KeyActionHint {
        title: "Show info",
        label: "i",
        section: HelpSection::LogPane,
        action: Some(CustomizedKeyAction::ShowInfo),
    },
    KeyActionHint {
        title: "Focus search",
        label: "enter",
        section: HelpSection::LogPane,
        action: None,
    },
    KeyActionHint {
        title: "Edit search",
        label: "type",
        section: HelpSection::QueryBox,
        action: None,
    },
    KeyActionHint {
        title: "Delete",
        label: "backspace",
        section: HelpSection::QueryBox,
        action: None,
    },
    KeyActionHint {
        title: "Clear query",
        label: "ctrl+k",
        section: HelpSection::QueryBox,
        action: None,
    },
    KeyActionHint {
        title: "Return to log",
        label: "enter",
        section: HelpSection::QueryBox,
        action: None,
    },
    KeyActionHint {
        title: "Toggle row",
        label: "space",
        section: HelpSection::SourceSelector,
        action: None,
    },
    KeyActionHint {
        title: "All sources",
        label: "a",
        section: HelpSection::SourceSelector,
        action: None,
    },
    KeyActionHint {
        title: "No sources",
        label: "n",
        section: HelpSection::SourceSelector,
        action: None,
    },
    KeyActionHint {
        title: "Close",
        label: "esc / backspace / enter",
        section: HelpSection::SourceSelector,
        action: None,
    },
    KeyActionHint {
        title: "Close",
        label: "esc / backspace / ?",
        section: HelpSection::HelpPopup,
        action: None,
    },
    KeyActionHint {
        title: "Select mode",
        label: "F2",
        section: HelpSection::Global,
        action: Some(CustomizedKeyAction::ToggleSelectMode),
    },
    KeyActionHint {
        title: "Yank entry",
        label: "y",
        section: HelpSection::LogPane,
        action: Some(CustomizedKeyAction::YankSelectedEntry),
    },
    KeyActionHint {
        title: "Toggle wrap",
        label: "w",
        section: HelpSection::Global,
        action: Some(CustomizedKeyAction::ToggleLineWrap),
    },
];

pub fn hints_for_section(section: HelpSection) -> impl Iterator<Item = &'static KeyActionHint> {
    ACTION_HINTS
        .iter()
        .filter(move |hint| hint.section == section)
}

/// Resolve the display label for a help/status hint against the active
/// bindings. Reserved / popup-local hints (`action == None`) keep their static
/// label. Configurable hints use the resolved binding label, or `None` when the
/// binding has been disabled so the surface can hide the hint.
pub fn hint_label(hint: &KeyActionHint, keybindings: &ResolvedKeybindings) -> Option<String> {
    match hint.action {
        Some(action) => keybindings.label_for(action).map(str::to_string),
        None => Some(hint.label.to_string()),
    }
}

/// User-resolved key bindings for the configurable [`CustomizedKeyAction`] set.
///
/// Built once from [`KeybindingsConfig`] at startup. Holds the parsed
/// key-event match table (first match wins, in declaration order) plus the
/// display labels used by the help popup and status bar. An action with an
/// empty spec list is disabled: it never matches and has no label.
#[derive(Clone, Debug)]
pub struct ResolvedKeybindings {
    bindings: Vec<(KeyEvent, CustomizedKeyAction)>,
    labels: HashMap<CustomizedKeyAction, String>,
}

impl ResolvedKeybindings {
    /// Parse every configured key spec into the runtime match table. Returns a
    /// [`FmlError::Keybinding`] if any spec is malformed.
    pub fn from_config(cfg: &KeybindingsConfig) -> Result<Self, FmlError> {
        // Declaration order defines first-match-wins precedence between actions
        // that happen to share a key.
        let configured: [(CustomizedKeyAction, &[String]); 9] = [
            (CustomizedKeyAction::ToggleHelp, &cfg.toggle_help),
            (
                CustomizedKeyAction::ToggleSourceSelector,
                &cfg.toggle_source_selector,
            ),
            (
                CustomizedKeyAction::TogglePreviewMode,
                &cfg.toggle_preview_mode,
            ),
            (CustomizedKeyAction::ShowInfo, &cfg.show_info),
            (CustomizedKeyAction::ScrollHead, &cfg.scroll_head),
            (CustomizedKeyAction::ScrollTail, &cfg.scroll_tail),
            (
                CustomizedKeyAction::ToggleSelectMode,
                &cfg.toggle_select_mode,
            ),
            (
                CustomizedKeyAction::YankSelectedEntry,
                &cfg.yank_selected_entry,
            ),
            (CustomizedKeyAction::ToggleLineWrap, &cfg.toggle_line_wrap),
        ];

        let mut bindings = Vec::new();
        let mut labels = HashMap::new();
        for (action, specs) in configured {
            if !specs.is_empty() {
                labels.insert(action, specs.join(" / "));
            }
            for spec in specs {
                bindings.push((parse_key_spec(spec)?, action));
            }
        }

        Ok(Self { bindings, labels })
    }

    /// The configurable action bound to `key`, or [`CustomizedKeyAction::None`].
    pub fn action_for(&self, key: &KeyEvent) -> CustomizedKeyAction {
        self.bindings
            .iter()
            .find(|(spec, _)| key_matches(spec, key))
            .map(|(_, action)| *action)
            .unwrap_or(CustomizedKeyAction::None)
    }

    /// Display label (all specs joined with ` / `) for an action, or `None`
    /// when the binding is disabled.
    pub fn label_for(&self, action: CustomizedKeyAction) -> Option<&str> {
        self.labels.get(&action).map(String::as_str)
    }
}

impl Default for ResolvedKeybindings {
    fn default() -> Self {
        // Built-in defaults are always valid specs.
        Self::from_config(&KeybindingsConfig::default()).expect("default keybindings must parse")
    }
}

/// Match an incoming key against a configured spec. For printable characters the
/// `SHIFT` modifier is ignored because the shifted character (e.g. `?`, `G`) is
/// already encoded in the [`KeyCode::Char`] value; named keys and `ctrl`/`alt`
/// modifiers must match exactly.
fn key_matches(spec: &KeyEvent, incoming: &KeyEvent) -> bool {
    if spec.code != incoming.code {
        return false;
    }
    let normalize = |event: &KeyEvent| {
        let mut modifiers = event.modifiers;
        if matches!(event.code, KeyCode::Char(_)) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        modifiers
    };
    normalize(spec) == normalize(incoming)
}

/// Parse a key spec such as `"?"`, `"enter"`, `"ctrl+s"`, `"pgdn"`, or `"f2"`
/// into a [`KeyEvent`]. Optional `ctrl`/`alt`/`shift` modifier prefixes are
/// joined with `+`; printable characters are written directly and keep their
/// case (`"G"` is shift-g). The literal `+` key cannot be bound.
pub fn parse_key_spec(spec: &str) -> Result<KeyEvent, FmlError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(FmlError::Keybinding("empty key spec".to_string()));
    }

    let parts: Vec<&str> = trimmed.split('+').collect();
    // The final segment is the key; everything before it is a modifier.
    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);

    let mut modifiers = KeyModifiers::NONE;
    for part in modifier_parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            other => {
                return Err(FmlError::Keybinding(format!(
                    "unknown modifier '{other}' in key spec '{spec}'"
                )));
            }
        }
    }

    let code = parse_key_code(key_part[0], spec)?;
    Ok(KeyEvent::new(code, modifiers))
}

fn parse_key_code(key: &str, spec: &str) -> Result<KeyCode, FmlError> {
    let lower = key.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "space" => KeyCode::Char(' '),
        _ => {
            if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                if (1..=12).contains(&n) {
                    return Ok(KeyCode::F(n));
                }
            }
            // A single printable character keeps its original case.
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => {
                    return Err(FmlError::Keybinding(format!(
                        "unrecognized key '{key}' in spec '{spec}'"
                    )));
                }
            }
        }
    };
    Ok(code)
}

pub fn match_key(
    key: &KeyEvent,
    _focus: &Slot,
    keybindings: &ResolvedKeybindings,
) -> (StaticKeyAction, CustomizedKeyAction) {
    // Reserved fallbacks are matched first and cannot be remapped via config.
    let static_key = match (key.code, key.modifiers) {
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => StaticKeyAction::Quit,
        (KeyCode::Tab, _) => StaticKeyAction::FocusCycle,
        (KeyCode::Up | KeyCode::Char('k'), _) => StaticKeyAction::ScrollUp,
        (KeyCode::Down | KeyCode::Char('j'), _) => StaticKeyAction::ScrollDown,
        // ... more arms as needed
        _ => StaticKeyAction::None,
    };

    let custom_key = keybindings.action_for(key);

    (static_key, custom_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> ResolvedKeybindings {
        ResolvedKeybindings::default()
    }

    #[test]
    fn global_hints_include_toggle_wrap() {
        let hints: Vec<_> = hints_for_section(HelpSection::Global).collect();
        assert!(
            hints.iter().any(|h| h.title == "Toggle wrap"
                && h.action == Some(CustomizedKeyAction::ToggleLineWrap)),
            "help popup global section should list toggle wrap binding"
        );
    }

    #[test]
    fn w_key_maps_to_toggle_line_wrap() {
        let (_static, custom) = match_key(
            &KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            &Slot::Main,
            &defaults(),
        );
        assert_eq!(custom, CustomizedKeyAction::ToggleLineWrap);
    }

    #[test]
    fn q_key_does_not_quit() {
        let (static_key, custom) = match_key(
            &KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &Slot::Main,
            &defaults(),
        );
        assert_eq!(static_key, StaticKeyAction::None);
        assert_eq!(custom, CustomizedKeyAction::None);
    }

    #[test]
    fn parses_modifier_and_named_specs() {
        assert_eq!(
            parse_key_spec("ctrl+s").unwrap(),
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            parse_key_spec("f2").unwrap(),
            KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key_spec("pgdn").unwrap(),
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key_spec("?").unwrap(),
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key_spec("G").unwrap(),
            KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn rejects_unknown_specs() {
        assert!(parse_key_spec("").is_err());
        assert!(parse_key_spec("hyper+x").is_err());
        assert!(parse_key_spec("nope").is_err());
    }

    #[test]
    fn shift_is_ignored_for_printable_chars() {
        let bindings = defaults();
        // `?` typed with SHIFT (as many terminals report it) still matches help.
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            CustomizedKeyAction::ToggleHelp
        );
    }

    #[test]
    fn remapped_binding_replaces_default() {
        let cfg = KeybindingsConfig {
            toggle_line_wrap: vec!["ctrl+w".to_string()],
            ..KeybindingsConfig::default()
        };
        let bindings = ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            CustomizedKeyAction::ToggleLineWrap
        );
        // Old default no longer triggers the action.
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)),
            CustomizedKeyAction::None
        );
    }

    #[test]
    fn disabled_binding_is_inert_and_unlabeled() {
        let cfg = KeybindingsConfig {
            toggle_help: vec![],
            ..KeybindingsConfig::default()
        };
        let bindings = ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            CustomizedKeyAction::None
        );
        assert_eq!(bindings.label_for(CustomizedKeyAction::ToggleHelp), None);
    }

    #[test]
    fn invalid_config_spec_surfaces_error() {
        let cfg = KeybindingsConfig {
            toggle_help: vec!["definitely-not-a-key".to_string()],
            ..KeybindingsConfig::default()
        };
        assert!(ResolvedKeybindings::from_config(&cfg).is_err());
    }
}
