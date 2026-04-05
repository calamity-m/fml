use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui_textarea::TextArea;

use crate::{
    config::tui::{ThemeConfig, TuiConfig},
    error::FmlError,
    tui::{
        layout::Slot,
        widgets::{
            FmlWidget,
            query_box::{self, QueryBox},
        },
    },
};

pub struct TuiState {
    pub focused: Slot,
    pub areas: HashMap<Slot, Rect>,
    pub selected_theme: ThemeConfig,
    pub query_box_textarea: TextArea<'static>,
}

impl TuiState {
    pub fn new(config: &TuiConfig) -> Result<Self, FmlError> {
        let selected_theme = config.resolved_theme()?;
        Ok(TuiState {
            focused: Slot::Main,
            areas: HashMap::new(),
            selected_theme,
            query_box_textarea: query_box::query_box_textarea(),
        })
    }
}
