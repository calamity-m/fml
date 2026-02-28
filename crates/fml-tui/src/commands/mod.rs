// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

use crate::{app::AppState, event::AppEvent, theme::Theme};

/// A parsed, validated command ready to be executed by the app shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // Close the focused tab, if we are on main then we'll close the app
    Quit,
    // Regardless of focused tab, close the app
    Exit,
    // Display help
    Help,
    // Change theme
    Theme(String),
    // Toggle display of timestamps
    Timestamps,
    // Jump to end of log stream and unapuse
    Tail,
    // Set greed value directly
    Greed(u8),
}

impl Command {
    /// Parse a raw command string (the text after the `:` prefix).
    ///
    /// Returns `Ok(cmd)` on success, `Err(message)` on failure. An empty
    /// string returns `Err("")` as a sentinel meaning "close without acting".
    pub fn parse(input: &str) -> Result<Command, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err(String::new());
        }

        let (word, rest) = input
            .split_once(char::is_whitespace)
            .map(|(w, r)| (w, r.trim()))
            .unwrap_or((input, ""));

        match word {
            "q" | "quit" => Ok(Command::Quit),
            "q!" | "quit!" => Ok(Command::Exit),
            "help" => Ok(Command::Help),
            "ts" | "timestamps" => Ok(Command::Timestamps),
            "tail" => Ok(Command::Tail),
            "theme" => {
                if rest.is_empty() {
                    Err("usage: theme <default|gruvbox>".to_string())
                } else {
                    Ok(Command::Theme(rest.to_string()))
                }
            }
            "greed" => match rest.parse::<u8>() {
                Ok(n) if n <= 10 => Ok(Command::Greed(n)),
                Ok(_) => Err("greed must be 0–10".to_string()),
                Err(_) => Err("usage: greed <0-10>".to_string()),
            },
            other => Err(format!("unknown command: {other}")),
        }
    }
}

/// Execute a parsed [`Command`] against the application state.
pub fn execute_command(s: &mut AppState, cmd: Command) {
    match cmd {
        Command::Quit => {
            if s.active_tab == 0 {
                s.quit = true;
            } else {
                s.tabs.remove(s.active_tab);
                s.active_tab = s.active_tab.saturating_sub(1);
            }
        }
        Command::Exit => {
            s.quit = true;
        }
        Command::Help => {
            s.show_help = !s.show_help;
        }
        Command::Theme(name) => {
            s.theme = match name.to_ascii_lowercase().as_str() {
                "gruvbox" | "gruvbox_dark" | "gruvbox-dark" => Theme::load_gruvbox_dark(),
                _ => Theme::load_default(),
            };
        }
        Command::Timestamps => {
            let tab = &mut s.tabs[s.active_tab];
            tab.stream.show_timestamps = !tab.stream.show_timestamps;
        }
        Command::Tail => {
            let tab = &mut s.tabs[s.active_tab];
            tab.stream.handle(&AppEvent::ScrollToTail);
        }
        Command::Greed(n) => {
            s.tabs[s.active_tab].query.greed = n;
        }
    }
}
