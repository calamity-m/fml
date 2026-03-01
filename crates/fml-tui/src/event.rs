//! Semantic application events — crossterm key events mapped to a
//! widget-agnostic vocabulary so widgets never touch crossterm directly.
//!
//! # Usage
//!
//! In the main event loop, call [`AppEvent::parse_event`] on every [`crossterm::event::Event`]
//! and match on the returned [`AppEvent`] instead of crossterm types.
//!
//! # Keybindings
//!
//! Defaults are hardcoded for Phase 2 and mirror the values documented in
//! `CLAUDE.md`. They will be wired to the user config file in Phase 7.
//!
//! | Key(s)                  | Event                      |
//! |-------------------------|----------------------------|
//! | `q`, `Ctrl+c`           | `Quit`                     |
//! | `Tab`                   | `FocusNext`                |
//! | `/`                     | `QueryFocus`               |
//! | `PageUp`, `Ctrl+u`      | `ScrollUp`                 |
//! | `PageDown`, `Ctrl+d`    | `ScrollDown`               |
//! | `G`                     | `ScrollToTail`             |
//! | `]`                     | `GreedUp`                  |
//! | `[`                     | `GreedDown`                |
//! | `↑` / `k`               | `TreeNav(Up)`              |
//! | `↓` / `j`               | `TreeNav(Down)`            |
//! | `←` / `h`               | `TreeNav(Left)`            |
//! | `→` / `l`               | `TreeNav(Right)`           |
//! | printable char          | `Char(c)`                  |
//! | `Backspace`             | `Backspace`                |
//! | `Enter`                 | `Enter`                    |
//! | terminal resize         | `Resize(w, h)`             |
//!
//! ## Insert mode
//!
//! When a text-input widget (query bar, command bar) is focused, the event
//! loop calls [`AppEvent::parse_event_insert`] instead. In insert mode:
//! - hjkl produce `Char` events instead of `TreeNav`
//! - `q`, `G`, `[`, `]` produce `Char` events
//! - Arrow keys still produce `TreeNav` for cursor movement
//! - Only `Ctrl+c`, `Escape`, `Enter`, `Tab`, and `Backspace` keep their
//!   special bindings

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// Cardinal direction for producer tree and log-stream navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// A semantic application event derived from a raw crossterm [`Event`].
///
/// Widgets receive `AppEvent` values — they never inspect crossterm types
/// directly. The App shell is responsible for routing events to the
/// appropriate widget based on the current focus state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// Exit the application (or close the focused non-main tab).
    Quit,
    /// Move keyboard focus to the next pane (Tab-cycle).
    FocusNext,
    /// Transfer focus to the query bar.
    QueryFocus,
    /// Scroll the log stream up.
    ScrollUp,
    /// Scroll the log stream down.
    ScrollDown,
    /// Jump to the tail (newest line) of the log stream.
    ScrollToTail,
    /// Increase the search greed level by one step.
    GreedUp,
    /// Decrease the search greed level by one step.
    GreedDown,
    /// Navigate within the producer tree (or scroll when log pane is focused).
    TreeNav(Direction),
    /// A printable character forwarded to the active text input.
    Char(char),
    /// Delete the character before the cursor in the active text input.
    Backspace,
    /// Confirm the active input or expand a tree node.
    Enter,
    /// The terminal was resized to the given (width, height).
    Resize(u16, u16),
    /// Dismiss the active modal (query bar focus, help popup).
    Escape,
    /// Regardless of focused tab, close the app
    Exit,
    /// Display help
    Help,
    /// Change theme
    Theme(String),
    /// Toggle display of timestamps
    Timestamps,
    /// Set greed value directly
    Greed(u8),
    /// Emitted when no handling is required
    NoOp,
}

impl AppEvent {
    /// Parse a command-bar input string into an [`AppEvent`].
    ///
    /// Returns `Err(String::new())` for empty input (close bar silently),
    /// or `Err(message)` for unknown/invalid commands (display to the user).
    pub fn parse_str(input: &str) -> Result<AppEvent, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err(String::new());
        }

        let (word, rest) = input
            .split_once(char::is_whitespace)
            .map(|(w, r)| (w, r.trim()))
            .unwrap_or((input, ""));

        match word {
            "q" | "quit" => Ok(AppEvent::Quit),
            "q!" | "quit!" => Ok(AppEvent::Exit),
            "?" | "help" => Ok(AppEvent::Help),
            "ts" | "timestamps" => Ok(AppEvent::Timestamps),
            "tail" => Ok(AppEvent::ScrollToTail),
            "theme" => {
                if rest.is_empty() {
                    Err("usage: theme <default|gruvbox>".to_string())
                } else {
                    Ok(AppEvent::Theme(rest.to_string()))
                }
            }
            "greed" => match rest.parse::<u8>() {
                Ok(n) if n <= 10 => Ok(AppEvent::Greed(n)),
                Ok(_) => Err("greed must be 0-10".to_string()),
                Err(_) => Err("usage: greed <0-10>".to_string()),
            },
            other => Err(format!("unknown command: {other}")),
        }
    }

    /// Key mapping for text-input / insert mode.
    ///
    /// All printable characters (with or without Shift) forward as `Char`.
    /// Arrow keys produce `TreeNav` so ← / → still move the text cursor.
    /// Navigation shortcuts (hjkl, q, G, [, ]) yield their literal characters.
    fn parse_key_event(input: KeyEvent) -> Option<AppEvent> {
        match input.code {
            // Quit — q (normal mode) or Ctrl+c anywhere
            KeyCode::Char('q') if input.modifiers == KeyModifiers::NONE => Some(AppEvent::Quit),
            KeyCode::Char('c') if input.modifiers == KeyModifiers::CONTROL => Some(AppEvent::Quit),

            // Focus cycling
            KeyCode::Tab if input.modifiers == KeyModifiers::NONE => Some(AppEvent::FocusNext),

            // Query bar
            KeyCode::Char('/') if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::QueryFocus)
            }

            // Scroll — page keys and vim-style Ctrl bindings.
            // Arrow keys / hjkl are reserved for TreeNav so both panes share them;
            // the App shell re-interprets TreeNav(Up/Down) as ScrollUp/Down when
            // the log stream pane is focused.
            KeyCode::PageUp => Some(AppEvent::ScrollUp),
            KeyCode::PageDown => Some(AppEvent::ScrollDown),
            KeyCode::Char('u') if input.modifiers == KeyModifiers::CONTROL => {
                Some(AppEvent::ScrollUp)
            }
            KeyCode::Char('d') if input.modifiers == KeyModifiers::CONTROL => {
                Some(AppEvent::ScrollDown)
            }

            // Scroll to tail — 'G' (uppercase, so SHIFT may or may not be set
            // depending on the terminal; match on the code alone)
            KeyCode::Char('G') => Some(AppEvent::ScrollToTail),

            // Greed adjustment
            KeyCode::Char(']') if input.modifiers == KeyModifiers::NONE => Some(AppEvent::GreedUp),
            KeyCode::Char('[') if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::GreedDown)
            }

            // Tree / list navigation
            KeyCode::Up | KeyCode::Char('k') if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::TreeNav(Direction::Up))
            }
            KeyCode::Down | KeyCode::Char('j') if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::TreeNav(Direction::Down))
            }
            KeyCode::Left | KeyCode::Char('h') if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::TreeNav(Direction::Left))
            }
            KeyCode::Right | KeyCode::Char('l') if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::TreeNav(Direction::Right))
            }

            // Text input — forward printable characters (including shifted ones,
            // e.g. uppercase letters while typing in the query bar)
            KeyCode::Char(c)
                if input.modifiers == KeyModifiers::NONE
                    || input.modifiers == KeyModifiers::SHIFT =>
            {
                Some(AppEvent::Char(c))
            }

            KeyCode::Backspace if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::Backspace)
            }
            KeyCode::Enter if input.modifiers == KeyModifiers::NONE => Some(AppEvent::Enter),
            KeyCode::Esc => Some(AppEvent::Escape),

            _ => None,
        }
    }

    fn parse_insert_key_event(input: KeyEvent) -> Option<AppEvent> {
        match input.code {
            // Ctrl+c always quits, even while typing
            KeyCode::Char('c') if input.modifiers == KeyModifiers::CONTROL => Some(AppEvent::Quit),

            // Arrow keys move the text cursor (widgets interpret TreeNav Left/Right)
            KeyCode::Up => Some(AppEvent::TreeNav(Direction::Up)),
            KeyCode::Down => Some(AppEvent::TreeNav(Direction::Down)),
            KeyCode::Left => Some(AppEvent::TreeNav(Direction::Left)),
            KeyCode::Right => Some(AppEvent::TreeNav(Direction::Right)),

            // Tab exits the text input (focus-cycle behaviour)
            KeyCode::Tab if input.modifiers == KeyModifiers::NONE => Some(AppEvent::FocusNext),

            // Every printable character — including letters that are nav shortcuts
            // in normal mode — is forwarded verbatim
            KeyCode::Char(c)
                if input.modifiers == KeyModifiers::NONE
                    || input.modifiers == KeyModifiers::SHIFT =>
            {
                Some(AppEvent::Char(c))
            }

            KeyCode::Backspace if input.modifiers == KeyModifiers::NONE => {
                Some(AppEvent::Backspace)
            }
            KeyCode::Enter if input.modifiers == KeyModifiers::NONE => Some(AppEvent::Enter),
            KeyCode::Esc => Some(AppEvent::Escape),

            _ => None,
        }
    }

    /// Map a raw crossterm [`Event`] to an [`AppEvent`] for normal (non-insert) mode.
    ///
    /// Returns `None` for events that have no binding (e.g. function keys).
    pub fn parse_event(input: Event) -> Option<AppEvent> {
        match input {
            Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
            Event::Key(key) => AppEvent::parse_key_event(key),
            _ => None,
        }
    }

    /// Map a raw crossterm [`Event`] to an [`AppEvent`] for text-input ("insert") mode.
    ///
    /// In insert mode, alphabetic navigation shortcuts (hjkl, q, G, \[, \]) are
    /// forwarded as [`AppEvent::Char`] so the user can type freely. Arrow keys
    /// still produce [`AppEvent::TreeNav`] so `←`/`→` move the text cursor.
    /// Only `Ctrl+c`, `Escape`, `Enter`, `Tab`, and `Backspace` keep their
    /// special bindings.
    ///
    /// Call this variant whenever a text-input widget (query bar, command bar)
    /// has focus.
    pub fn parse_insert_event(input: Event) -> Option<AppEvent> {
        match input {
            Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
            Event::Key(key) => AppEvent::parse_insert_key_event(key),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn press(code: KeyCode) -> Event {
        key(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> Event {
        key(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn quit_keys() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('q'))),
            Some(AppEvent::Quit)
        );
        assert_eq!(
            AppEvent::parse_event(ctrl(KeyCode::Char('c'))),
            Some(AppEvent::Quit)
        );
    }

    #[test]
    fn focus_next() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Tab)),
            Some(AppEvent::FocusNext)
        );
    }

    #[test]
    fn query_focus() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('/'))),
            Some(AppEvent::QueryFocus)
        );
    }

    #[test]
    fn scroll_to_tail() {
        // Uppercase G — terminal may or may not send SHIFT modifier
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('G'))),
            Some(AppEvent::ScrollToTail)
        );
        assert_eq!(
            AppEvent::parse_event(key(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            Some(AppEvent::ScrollToTail)
        );
    }

    #[test]
    fn greed_keys() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char(']'))),
            Some(AppEvent::GreedUp)
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('['))),
            Some(AppEvent::GreedDown)
        );
    }

    #[test]
    fn tree_nav_arrows() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Up)),
            Some(AppEvent::TreeNav(Direction::Up))
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Down)),
            Some(AppEvent::TreeNav(Direction::Down))
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Left)),
            Some(AppEvent::TreeNav(Direction::Left))
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Right)),
            Some(AppEvent::TreeNav(Direction::Right))
        );
    }

    #[test]
    fn tree_nav_hjkl() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('k'))),
            Some(AppEvent::TreeNav(Direction::Up))
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('j'))),
            Some(AppEvent::TreeNav(Direction::Down))
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('h'))),
            Some(AppEvent::TreeNav(Direction::Left))
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('l'))),
            Some(AppEvent::TreeNav(Direction::Right))
        );
    }

    #[test]
    fn scroll_page_keys() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::PageUp)),
            Some(AppEvent::ScrollUp)
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::PageDown)),
            Some(AppEvent::ScrollDown)
        );
    }

    #[test]
    fn scroll_ctrl_ud() {
        assert_eq!(
            AppEvent::parse_event(ctrl(KeyCode::Char('u'))),
            Some(AppEvent::ScrollUp)
        );
        assert_eq!(
            AppEvent::parse_event(ctrl(KeyCode::Char('d'))),
            Some(AppEvent::ScrollDown)
        );
    }

    #[test]
    fn char_forwarding() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Char('a'))),
            Some(AppEvent::Char('a'))
        );
        // Uppercase (SHIFT held)
        assert_eq!(
            AppEvent::parse_event(key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Some(AppEvent::Char('A'))
        );
    }

    #[test]
    fn backspace_and_enter() {
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Backspace)),
            Some(AppEvent::Backspace)
        );
        assert_eq!(
            AppEvent::parse_event(press(KeyCode::Enter)),
            Some(AppEvent::Enter)
        );
    }

    #[test]
    fn resize_event() {
        use crossterm::event::Event;
        assert_eq!(
            AppEvent::parse_event(Event::Resize(120, 40)),
            Some(AppEvent::Resize(120, 40))
        );
    }

    #[test]
    fn unbound_key_returns_none() {
        assert_eq!(AppEvent::parse_event(press(KeyCode::F(5))), None);
    }

    // ── Insert mode ────────────────────────────────────────────────────────

    #[test]
    fn insert_mode_nav_letters_are_chars() {
        // hjkl and q must type their literal characters in insert mode
        for ch in ['h', 'j', 'k', 'l', 'q', 'G', '[', ']'] {
            let ev = press(KeyCode::Char(ch));
            assert_eq!(
                AppEvent::parse_insert_event(ev),
                Some(AppEvent::Char(ch)),
                "insert mode: '{ch}' should produce Char, not a nav event"
            );
        }
    }

    #[test]
    fn insert_mode_arrow_keys_are_treenav() {
        assert_eq!(
            AppEvent::parse_insert_event(press(KeyCode::Left)),
            Some(AppEvent::TreeNav(Direction::Left))
        );
        assert_eq!(
            AppEvent::parse_insert_event(press(KeyCode::Right)),
            Some(AppEvent::TreeNav(Direction::Right))
        );
    }

    #[test]
    fn insert_mode_ctrl_c_still_quits() {
        assert_eq!(
            AppEvent::parse_insert_event(ctrl(KeyCode::Char('c'))),
            Some(AppEvent::Quit)
        );
    }

    // ── parse_str ─────────────────────────────────────────────────────────────

    #[test]
    fn parse_str_quit_short_and_long() {
        assert_eq!(AppEvent::parse_str("q"), Ok(AppEvent::Quit));
        assert_eq!(AppEvent::parse_str("quit"), Ok(AppEvent::Quit));
        assert_eq!(AppEvent::parse_str("  quit  "), Ok(AppEvent::Quit));
    }

    #[test]
    fn parse_str_force_quit() {
        assert_eq!(AppEvent::parse_str("q!"), Ok(AppEvent::Exit));
        assert_eq!(AppEvent::parse_str("quit!"), Ok(AppEvent::Exit));
    }

    #[test]
    fn parse_str_help() {
        assert_eq!(AppEvent::parse_str("help"), Ok(AppEvent::Help));
        assert_eq!(AppEvent::parse_str("?"), Ok(AppEvent::Help));
    }

    #[test]
    fn parse_str_timestamps() {
        assert_eq!(AppEvent::parse_str("ts"), Ok(AppEvent::Timestamps));
        assert_eq!(AppEvent::parse_str("timestamps"), Ok(AppEvent::Timestamps));
    }

    #[test]
    fn parse_str_tail() {
        assert_eq!(AppEvent::parse_str("tail"), Ok(AppEvent::ScrollToTail));
    }

    #[test]
    fn parse_str_theme_with_arg() {
        assert_eq!(
            AppEvent::parse_str("theme gruvbox"),
            Ok(AppEvent::Theme("gruvbox".to_string()))
        );
        assert_eq!(
            AppEvent::parse_str("theme default"),
            Ok(AppEvent::Theme("default".to_string()))
        );
    }

    #[test]
    fn parse_str_theme_without_arg_is_err() {
        assert!(AppEvent::parse_str("theme").is_err());
        let err = AppEvent::parse_str("theme").unwrap_err();
        assert!(err.contains("usage"));
    }

    #[test]
    fn parse_str_greed_valid_range() {
        assert_eq!(AppEvent::parse_str("greed 0"), Ok(AppEvent::Greed(0)));
        assert_eq!(AppEvent::parse_str("greed 5"), Ok(AppEvent::Greed(5)));
        assert_eq!(AppEvent::parse_str("greed 10"), Ok(AppEvent::Greed(10)));
    }

    #[test]
    fn parse_str_greed_out_of_range_is_err() {
        assert!(AppEvent::parse_str("greed 11").is_err());
    }

    #[test]
    fn parse_str_greed_non_numeric_is_err() {
        assert!(AppEvent::parse_str("greed abc").is_err());
    }

    #[test]
    fn parse_str_empty_returns_sentinel_err() {
        assert_eq!(AppEvent::parse_str(""), Err(String::new()));
        assert_eq!(AppEvent::parse_str("   "), Err(String::new()));
    }

    #[test]
    fn parse_str_unknown_command_is_err() {
        let err = AppEvent::parse_str("frobnicate").unwrap_err();
        assert!(err.contains("frobnicate"));
    }
}
