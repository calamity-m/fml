use crate::tui::layout::Slot;

pub struct TuiState {
    pub focused: Slot,
}

impl TuiState {
    pub fn new() -> Self {
        TuiState {
            focused: Slot::Main,
        }
    }
}
