//! Runtime keybinding types compiled from [`KeybindingsConfig`].
//!
//! This module owns the runtime-only half of the keybinding model:
//! the stable [`KeyAction`] identifiers, the parsed [`KeyBinding`] matcher,
//! and the compiled [`ResolvedBindings`] struct that Part 02 uses to replace
//! open-coded `if key.code == ...` comparisons in the event path.
//!
//! The serde-visible configuration surface lives in

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::error::FmlError;

/// Stable identifiers for the currently active keyboard shortcuts.
///
/// Only actions with working runtime behavior are listed here. Placeholder
/// shortcuts for planned features such as source picker, preview mode, and
/// export are excluded until their runtime behavior exists.
///
/// Action IDs are display and lookup metadata only. Matched keys still flow
/// through the existing [`Event`](crate::Event)/[`Command`](crate::Command)
/// path — this enum does not introduce a second event bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyAction {
    // --- Configurable actions ---
    /// Open or close the help popup. Active in global scope.
    ToggleHelp,
    /// Show the detail popup for the selected log entry.
    /// Active in global scope but only fires when the log pane is focused.
    ShowInfo,
}

/// A parsed runtime key binding compiled from a key spec string.
///
/// Not stored in config. Derived via [`parse_key_spec`] from the strings in
/// [`KeybindingsConfig`](super::tui::KeybindingsConfig). Matches a
/// [`KeyEvent`] by exact code and modifier comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    /// Returns `true` if this binding matches the given key event.
    pub fn matches(&self, key: &KeyEvent) -> bool {
        if key.code != self.code {
            return false;
        }

        let key_modifiers = key.modifiers & supported_modifiers();
        if matches!(self.code, KeyCode::Char(_))
            && self.modifiers == KeyModifiers::NONE
            && key_modifiers == KeyModifiers::SHIFT
        {
            return true;
        }

        key_modifiers == self.modifiers
    }

    /// Produces a compact human-readable label for display in the status bar
    /// and help popup (e.g. `"?"`, `"ctrl+c"`, `"pgdn"`).
    pub fn label(&self) -> String {
        let key_part = match self.code {
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Esc => "esc".to_string(),
            KeyCode::Tab => "tab".to_string(),
            KeyCode::Up => "up".to_string(),
            KeyCode::Down => "down".to_string(),
            KeyCode::Left => "left".to_string(),
            KeyCode::Right => "right".to_string(),
            KeyCode::Home => "home".to_string(),
            KeyCode::End => "end".to_string(),
            KeyCode::PageUp => "pgup".to_string(),
            KeyCode::PageDown => "pgdn".to_string(),
            KeyCode::Backspace => "backspace".to_string(),
            KeyCode::Delete => "delete".to_string(),
            KeyCode::Insert => "insert".to_string(),
            KeyCode::F(n) => format!("f{n}"),
            KeyCode::Char(c) => c.to_string(),
            _ => "?".to_string(),
        };

        let mut parts: Vec<&str> = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("ctrl");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("alt");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("shift");
        }
        parts.push(&key_part);
        parts.join("+")
    }
}

/// Compiled runtime bindings ready for key-event matching.
///
/// Contains one parsed [`KeyBinding`] list per configurable action.
///
/// Reserved fallbacks (`ctrl+c`, `tab`, `esc`) are not included because the
/// app matches them before any binding lookup takes place.
#[derive(Clone, Debug)]
pub struct ResolvedBindings {
    pub toggle_help: Vec<KeyBinding>,
    pub show_info: Vec<KeyBinding>,
}

impl ResolvedBindings {
    /// Returns `true` if any binding for the given configurable action matches `key`.
    ///
    /// Always returns `false` for reserved fallback actions ([`KeyAction::Quit`],
    /// [`KeyAction::CycleFocus`], [`KeyAction::ClosePopup`]) because those are
    /// matched directly by the app before binding lookup.
    pub fn matches(&self, action: KeyAction, key: &KeyEvent) -> bool {
        self.bindings_for(action)
            .is_some_and(|bindings| bindings.iter().any(|b| b.matches(key)))
    }

    /// Return the first matching action from `actions`, preserving candidate order.
    pub fn match_action(&self, key: &KeyEvent, actions: &[KeyAction]) -> Option<KeyAction> {
        actions
            .iter()
            .copied()
            .find(|action| self.matches(*action, key))
    }

    /// Returns the primary display label for a configurable action.
    ///
    /// The primary label is the first configured binding's [`label()`](KeyBinding::label)
    /// string. Returns `None` if the action has no configured bindings (i.e. the
    /// user set an empty list to disable it), or if passed a reserved action.
    pub fn primary_label(&self, action: KeyAction) -> Option<String> {
        self.bindings_for(action)
            .and_then(|bindings| bindings.first())
            .map(KeyBinding::label)
    }

    /// Returns every configured display label for a configurable action.
    pub fn labels(&self, action: KeyAction) -> Vec<String> {
        self.bindings_for(action)
            .map(|bindings| bindings.iter().map(KeyBinding::label).collect())
            .unwrap_or_default()
    }

    fn bindings_for(&self, action: KeyAction) -> Option<&[KeyBinding]> {
        match action {
            KeyAction::ToggleHelp => Some(&self.toggle_help),
            KeyAction::ShowInfo => Some(&self.show_info),
        }
    }
}

/// Parse a key spec string into a [`KeyBinding`].
///
/// # Format
///
/// A key spec is a lowercase string composed of optional modifier prefixes
/// separated by `+`, followed by a key name:
///
/// - Modifiers: `ctrl`, `alt`, `shift` (any combination, in any order)
/// - Named keys: `enter`, `esc`, `tab`, `up`, `down`, `left`, `right`,
///   `home`, `end`, `pgup`, `pgdn`, `backspace`, `delete`, `insert`
/// - Function keys: `f1` through `f12`
/// - Single printable character: `a`, `?`, `/`, etc.
///
/// Printable characters are specified directly without shift encoding.
/// Use `"?"` rather than `"shift+/"`.
///
/// # Examples
///
/// ```
/// # use fml_core::keybindings::parse_key_spec;
/// assert!(parse_key_spec("?").is_ok());
/// assert!(parse_key_spec("ctrl+c").is_ok());
/// assert!(parse_key_spec("pgdn").is_ok());
/// assert!(parse_key_spec("j").is_ok());
/// ```
pub fn parse_key_spec(spec: &str) -> Result<KeyBinding, FmlError> {
    if spec.is_empty() {
        return Err(FmlError::Keybinding(
            "key spec must not be empty".to_string(),
        ));
    }

    let parts: Vec<&str> = spec.split('+').collect();
    // The last token is the key name; everything before it is a modifier.
    // split_last returns (last, rest_without_last).
    let (key_part, modifier_parts) = parts
        .split_last()
        .expect("split on non-empty string always yields at least one element");

    let mut modifiers = KeyModifiers::NONE;
    for m in modifier_parts {
        match *m {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            other => {
                return Err(FmlError::Keybinding(format!(
                    "unknown modifier {other:?} in key spec {spec:?}"
                )));
            }
        }
    }

    let code = parse_key_code(key_part, spec)?;
    Ok(KeyBinding { code, modifiers })
}

fn supported_modifiers() -> KeyModifiers {
    KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT
}

fn parse_key_code(name: &str, spec: &str) -> Result<KeyCode, FmlError> {
    match name {
        "enter" => Ok(KeyCode::Enter),
        "esc" => Ok(KeyCode::Esc),
        "tab" => Ok(KeyCode::Tab),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pgup" => Ok(KeyCode::PageUp),
        "pgdn" => Ok(KeyCode::PageDown),
        "backspace" => Ok(KeyCode::Backspace),
        "delete" => Ok(KeyCode::Delete),
        "insert" => Ok(KeyCode::Insert),
        "f1" => Ok(KeyCode::F(1)),
        "f2" => Ok(KeyCode::F(2)),
        "f3" => Ok(KeyCode::F(3)),
        "f4" => Ok(KeyCode::F(4)),
        "f5" => Ok(KeyCode::F(5)),
        "f6" => Ok(KeyCode::F(6)),
        "f7" => Ok(KeyCode::F(7)),
        "f8" => Ok(KeyCode::F(8)),
        "f9" => Ok(KeyCode::F(9)),
        "f10" => Ok(KeyCode::F(10)),
        "f11" => Ok(KeyCode::F(11)),
        "f12" => Ok(KeyCode::F(12)),
        single if single.chars().count() == 1 => {
            let c = single.chars().next().expect("count == 1 guarantees a char");
            Ok(KeyCode::Char(c))
        }
        _ => Err(FmlError::Keybinding(format!(
            "unrecognized key name {name:?} in spec {spec:?}"
        ))),
    }
}

/// Parse a list of key spec strings into a list of [`KeyBinding`]s.
pub fn parse_specs(specs: &[String]) -> Result<Vec<KeyBinding>, FmlError> {
    specs.iter().map(|s| parse_key_spec(s)).collect()
}

/// Check that no two configurable actions share an identical binding.
///
/// Bindings are compared by `(KeyCode, KeyModifiers)`. The check is flat
/// across all provided actions; scope-aware precedence is the responsibility
/// of the runtime input path (Part 02), not the config model.
pub fn validate_conflicts(actions: &[(&str, &[KeyBinding])]) -> Result<(), FmlError> {
    let mut seen: HashMap<(KeyCode, KeyModifiers), &str> = HashMap::new();

    for (action_name, bindings) in actions {
        for binding in *bindings {
            let key = (binding.code.clone(), binding.modifiers);
            match seen.get(&key) {
                Some(first) if *first != *action_name => {
                    return Err(FmlError::Keybinding(format!(
                        "keybinding conflict: the same binding is assigned to both \
                         '{first}' and '{action_name}'"
                    )));
                }
                _ => {
                    seen.insert(key, action_name);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use crate::tui::keybindings::{parse_key_spec, validate_conflicts};

    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // --- parse_key_spec ---

    #[test]
    fn parse_printable_char() {
        let b = parse_key_spec("?").unwrap();
        assert_eq!(b.code, KeyCode::Char('?'));
        assert_eq!(b.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(parse_key_spec("enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key_spec("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key_spec("tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key_spec("up").unwrap().code, KeyCode::Up);
        assert_eq!(parse_key_spec("down").unwrap().code, KeyCode::Down);
        assert_eq!(parse_key_spec("pgup").unwrap().code, KeyCode::PageUp);
        assert_eq!(parse_key_spec("pgdn").unwrap().code, KeyCode::PageDown);
        assert_eq!(parse_key_spec("home").unwrap().code, KeyCode::Home);
        assert_eq!(parse_key_spec("end").unwrap().code, KeyCode::End);
    }

    #[test]
    fn parse_ctrl_modifier() {
        let b = parse_key_spec("ctrl+c").unwrap();
        assert_eq!(b.code, KeyCode::Char('c'));
        assert_eq!(b.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_function_key() {
        assert_eq!(parse_key_spec("f1").unwrap().code, KeyCode::F(1));
        assert_eq!(parse_key_spec("f12").unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn parse_empty_spec_fails() {
        assert!(parse_key_spec("").is_err());
    }

    #[test]
    fn parse_unknown_modifier_fails() {
        assert!(parse_key_spec("super+c").is_err());
    }

    #[test]
    fn parse_unknown_key_name_fails() {
        assert!(parse_key_spec("numpad0").is_err());
    }

    // --- KeyBinding::matches ---

    #[test]
    fn binding_matches_exact_code_and_modifiers() {
        let binding = parse_key_spec("ctrl+c").unwrap();
        let matching = key_event(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let wrong_code = key_event(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let wrong_mod = key_event(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(binding.matches(&matching));
        assert!(!binding.matches(&wrong_code));
        assert!(!binding.matches(&wrong_mod));
    }

    #[test]
    fn printable_binding_matches_shifted_punctuation_event() {
        let binding = parse_key_spec("?").unwrap();
        let key = key_event(KeyCode::Char('?'), KeyModifiers::SHIFT);

        assert!(binding.matches(&key));
    }

    // --- KeyBinding::label ---

    #[test]
    fn label_round_trips_through_parse() {
        for spec in &["?", "enter", "ctrl+c", "pgdn", "j", "f1", "alt+up"] {
            let binding = parse_key_spec(spec).unwrap();
            assert_eq!(
                &binding.label(),
                spec,
                "label should round-trip for {spec:?}"
            );
        }
    }

    // --- validate_conflicts ---

    #[test]
    fn no_conflict_in_disjoint_bindings() {
        let a = vec![
            parse_key_spec("j").unwrap(),
            parse_key_spec("down").unwrap(),
        ];
        let b = vec![parse_key_spec("k").unwrap(), parse_key_spec("up").unwrap()];
        assert!(validate_conflicts(&[("scroll_down", &a), ("scroll_up", &b)]).is_ok());
    }

    #[test]
    fn conflict_detected_across_actions() {
        let a = vec![parse_key_spec("j").unwrap()];
        let b = vec![parse_key_spec("j").unwrap()];
        assert!(validate_conflicts(&[("action_a", &a), ("action_b", &b)]).is_err());
    }

    #[test]
    fn duplicate_within_same_action_is_allowed() {
        // A user who lists the same key twice for one action gets redundant
        // bindings but no error — it is harmless.
        let a = vec![parse_key_spec("j").unwrap(), parse_key_spec("j").unwrap()];
        assert!(validate_conflicts(&[("action_a", &a)]).is_ok());
    }
}
