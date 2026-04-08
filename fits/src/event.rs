pub struct QuitEvent {}

#[derive(Debug)]
pub enum TuiEvent {
    /// A render tick has been requested
    Render,
    /// The terminal gained focus.
    FocusGained,
    /// The terminal lost focus.
    FocusLost,
    /// A mouse action occurred.
    Mouse(crossterm::event::MouseEvent),
    /// The terminal was resized to `(columns, rows)`.
    Resize(u16, u16),
    /// Text was pasted from the clipboard.
    Paste(String),
    /// A scroll action was requested in the given direction.
    Scroll(ratatui::widgets::ScrollDirection),
    /// A scroll action to set a cursor to head was done
    ScrollHead,
    /// A scroll action to set a cursor to tail was done
    ScrollTail,
    /// A user input key event was received.
    Input(crossterm::event::KeyEvent),
    /// An error occurred in the event stream.
    Error(String),
}
