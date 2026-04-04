use std::collections::HashMap;

use ratatui::layout::Rect;

use crate::{
    config::tui::{ThemeConfig, TuiConfig},
    error::FmlError,
    tui::layout::Slot,
};

pub struct TuiState {
    pub focused: Slot,
    pub areas: HashMap<Slot, Rect>,

    pub selected_theme: ThemeConfig,
}

impl TuiState {
    pub fn new(config: &TuiConfig) -> Result<Self, FmlError> {
        let selected_theme = config.resolved_theme()?;
        Ok(TuiState {
            focused: Slot::Main,
            areas: HashMap::new(),
            selected_theme,
        })
    }
}
