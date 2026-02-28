//! Semantic application events — crossterm key events mapped to a
//! widget-agnostic vocabulary so widgets never touch crossterm directly.
//!
//! # Usage
//!
//! In the main event loop, call [`to_app_event`] on every [`crossterm::event::Event`]
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
//! loop calls [`to_app_event_insert`] instead. In insert mode:
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
}

/// Map a raw crossterm [`Event`] to an [`AppEvent`] (normal / navigation mode).
///
/// Returns `None` for events that carry no semantic meaning for the
/// application (mouse events, key-release events on terminals that emit
/// them, unbound keys).
///
/// Keybindings are hardcoded to their defaults for Phase 2.
pub fn to_app_event(event: Event) -> Option<AppEvent> {
    match event {
        Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
        Event::Key(key) => map_key(key),
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
pub fn to_app_event_insert(event: Event) -> Option<AppEvent> {
    match event {
        Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
        Event::Key(key) => map_key_insert(key),
        _ => None,
    }
}

fn map_key(key: KeyEvent) -> Option<AppEvent> {
    use KeyCode::*;
    use KeyModifiers as Mod;

    match key.code {
        // Quit — q (normal mode) or Ctrl+c anywhere
        Char('q') if key.modifiers == Mod::NONE => Some(AppEvent::Quit),
        Char('c') if key.modifiers == Mod::CONTROL => Some(AppEvent::Quit),

        // Focus cycling
        Tab if key.modifiers == Mod::NONE => Some(AppEvent::FocusNext),

        // Query bar
        Char('/') if key.modifiers == Mod::NONE => Some(AppEvent::QueryFocus),

        // Scroll — page keys and vim-style Ctrl bindings.
        // Arrow keys / hjkl are reserved for TreeNav so both panes share them;
        // the App shell re-interprets TreeNav(Up/Down) as ScrollUp/Down when
        // the log stream pane is focused.
        PageUp => Some(AppEvent::ScrollUp),
        PageDown => Some(AppEvent::ScrollDown),
        Char('u') if key.modifiers == Mod::CONTROL => Some(AppEvent::ScrollUp),
        Char('d') if key.modifiers == Mod::CONTROL => Some(AppEvent::ScrollDown),

        // Scroll to tail — 'G' (uppercase, so SHIFT may or may not be set
        // depending on the terminal; match on the code alone)
        Char('G') => Some(AppEvent::ScrollToTail),

        // Greed adjustment
        Char(']') if key.modifiers == Mod::NONE => Some(AppEvent::GreedUp),
        Char('[') if key.modifiers == Mod::NONE => Some(AppEvent::GreedDown),

        // Tree / list navigation
        Up | Char('k') if key.modifiers == Mod::NONE => {
            Some(AppEvent::TreeNav(Direction::Up))
        }
        Down | Char('j') if key.modifiers == Mod::NONE => {
            Some(AppEvent::TreeNav(Direction::Down))
        }
        Left | Char('h') if key.modifiers == Mod::NONE => {
            Some(AppEvent::TreeNav(Direction::Left))
        }
        Right | Char('l') if key.modifiers == Mod::NONE => {
            Some(AppEvent::TreeNav(Direction::Right))
        }

        // Text input — forward printable characters (including shifted ones,
        // e.g. uppercase letters while typing in the query bar)
        Char(c) if key.modifiers == Mod::NONE || key.modifiers == Mod::SHIFT => {
            Some(AppEvent::Char(c))
        }

        Backspace if key.modifiers == Mod::NONE => Some(AppEvent::Backspace),
        Enter if key.modifiers == Mod::NONE => Some(AppEvent::Enter),
        Esc => Some(AppEvent::Escape),

        _ => None,
    }
}

/// Key mapping for text-input / insert mode.
///
/// All printable characters (with or without Shift) forward as `Char`.
/// Arrow keys produce `TreeNav` so ← / → still move the text cursor.
/// Navigation shortcuts (hjkl, q, G, [, ]) yield their literal characters.
fn map_key_insert(key: KeyEvent) -> Option<AppEvent> {
    use KeyCode::*;
    use KeyModifiers as Mod;

    match key.code {
        // Ctrl+c always quits, even while typing
        Char('c') if key.modifiers == Mod::CONTROL => Some(AppEvent::Quit),

        // Arrow keys move the text cursor (widgets interpret TreeNav Left/Right)
        Up => Some(AppEvent::TreeNav(Direction::Up)),
        Down => Some(AppEvent::TreeNav(Direction::Down)),
        Left => Some(AppEvent::TreeNav(Direction::Left)),
        Right => Some(AppEvent::TreeNav(Direction::Right)),

        // Tab exits the text input (focus-cycle behaviour)
        Tab if key.modifiers == Mod::NONE => Some(AppEvent::FocusNext),

        // Every printable character — including letters that are nav shortcuts
        // in normal mode — is forwarded verbatim
        Char(c) if key.modifiers == Mod::NONE || key.modifiers == Mod::SHIFT => {
            Some(AppEvent::Char(c))
        }

        Backspace if key.modifiers == Mod::NONE => Some(AppEvent::Backspace),
        Enter if key.modifiers == Mod::NONE => Some(AppEvent::Enter),
        Esc => Some(AppEvent::Escape),

        _ => None,
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
        assert_eq!(to_app_event(press(KeyCode::Char('q'))), Some(AppEvent::Quit));
        assert_eq!(to_app_event(ctrl(KeyCode::Char('c'))), Some(AppEvent::Quit));
    }

    #[test]
    fn focus_next() {
        assert_eq!(to_app_event(press(KeyCode::Tab)), Some(AppEvent::FocusNext));
    }

    #[test]
    fn query_focus() {
        assert_eq!(
            to_app_event(press(KeyCode::Char('/'))),
            Some(AppEvent::QueryFocus)
        );
    }

    #[test]
    fn scroll_to_tail() {
        // Uppercase G — terminal may or may not send SHIFT modifier
        assert_eq!(
            to_app_event(press(KeyCode::Char('G'))),
            Some(AppEvent::ScrollToTail)
        );
        assert_eq!(
            to_app_event(key(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            Some(AppEvent::ScrollToTail)
        );
    }

    #[test]
    fn greed_keys() {
        assert_eq!(
            to_app_event(press(KeyCode::Char(']'))),
            Some(AppEvent::GreedUp)
        );
        assert_eq!(
            to_app_event(press(KeyCode::Char('['))),
            Some(AppEvent::GreedDown)
        );
    }

    #[test]
    fn tree_nav_arrows() {
        assert_eq!(
            to_app_event(press(KeyCode::Up)),
            Some(AppEvent::TreeNav(Direction::Up))
        );
        assert_eq!(
            to_app_event(press(KeyCode::Down)),
            Some(AppEvent::TreeNav(Direction::Down))
        );
        assert_eq!(
            to_app_event(press(KeyCode::Left)),
            Some(AppEvent::TreeNav(Direction::Left))
        );
        assert_eq!(
            to_app_event(press(KeyCode::Right)),
            Some(AppEvent::TreeNav(Direction::Right))
        );
    }

    #[test]
    fn tree_nav_hjkl() {
        assert_eq!(
            to_app_event(press(KeyCode::Char('k'))),
            Some(AppEvent::TreeNav(Direction::Up))
        );
        assert_eq!(
            to_app_event(press(KeyCode::Char('j'))),
            Some(AppEvent::TreeNav(Direction::Down))
        );
        assert_eq!(
            to_app_event(press(KeyCode::Char('h'))),
            Some(AppEvent::TreeNav(Direction::Left))
        );
        assert_eq!(
            to_app_event(press(KeyCode::Char('l'))),
            Some(AppEvent::TreeNav(Direction::Right))
        );
    }

    #[test]
    fn scroll_page_keys() {
        assert_eq!(to_app_event(press(KeyCode::PageUp)), Some(AppEvent::ScrollUp));
        assert_eq!(
            to_app_event(press(KeyCode::PageDown)),
            Some(AppEvent::ScrollDown)
        );
    }

    #[test]
    fn scroll_ctrl_ud() {
        assert_eq!(
            to_app_event(ctrl(KeyCode::Char('u'))),
            Some(AppEvent::ScrollUp)
        );
        assert_eq!(
            to_app_event(ctrl(KeyCode::Char('d'))),
            Some(AppEvent::ScrollDown)
        );
    }

    #[test]
    fn char_forwarding() {
        assert_eq!(
            to_app_event(press(KeyCode::Char('a'))),
            Some(AppEvent::Char('a'))
        );
        // Uppercase (SHIFT held)
        assert_eq!(
            to_app_event(key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Some(AppEvent::Char('A'))
        );
    }

    #[test]
    fn backspace_and_enter() {
        assert_eq!(
            to_app_event(press(KeyCode::Backspace)),
            Some(AppEvent::Backspace)
        );
        assert_eq!(to_app_event(press(KeyCode::Enter)), Some(AppEvent::Enter));
    }

    #[test]
    fn resize_event() {
        use crossterm::event::Event;
        assert_eq!(
            to_app_event(Event::Resize(120, 40)),
            Some(AppEvent::Resize(120, 40))
        );
    }

    #[test]
    fn unbound_key_returns_none() {
        assert_eq!(to_app_event(press(KeyCode::F(5))), None);
    }

    // ── Insert mode ────────────────────────────────────────────────────────

    #[test]
    fn insert_mode_nav_letters_are_chars() {
        // hjkl and q must type their literal characters in insert mode
        for ch in ['h', 'j', 'k', 'l', 'q', 'G', '[', ']'] {
            let ev = press(KeyCode::Char(ch));
            assert_eq!(
                to_app_event_insert(ev),
                Some(AppEvent::Char(ch)),
                "insert mode: '{ch}' should produce Char, not a nav event"
            );
        }
    }

    #[test]
    fn insert_mode_arrow_keys_are_treenav() {
        assert_eq!(
            to_app_event_insert(press(KeyCode::Left)),
            Some(AppEvent::TreeNav(Direction::Left))
        );
        assert_eq!(
            to_app_event_insert(press(KeyCode::Right)),
            Some(AppEvent::TreeNav(Direction::Right))
        );
    }

    #[test]
    fn insert_mode_ctrl_c_still_quits() {
        assert_eq!(
            to_app_event_insert(ctrl(KeyCode::Char('c'))),
            Some(AppEvent::Quit)
        );
    }
}
