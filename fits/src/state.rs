use std::io::{Stdout, stdout};

use ratatui::{Terminal, prelude::CrosstermBackend};

use crate::{
    config::Config,
    error::FmlError,
    state::{events_bus::EventBus, tui_state::TuiState},
    tui::widgets::{FmlWidget, log_pane::LogPane, query_box::QueryBox, status_bar::StatusBar},
};

pub mod events_bus;
pub mod tui_state;

pub struct AppState {
    pub config: Config,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub widgets: Vec<Box<dyn FmlWidget>>,
    pub event_bus: EventBus,
    pub tui: TuiState,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, FmlError> {
        Ok(AppState {
            config: config.clone(),
            terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
            widgets: vec![
                Box::new(QueryBox::new()),
                Box::new(StatusBar::new()),
                Box::new(LogPane::new()),
            ],
            event_bus: EventBus::new(),
            tui: TuiState::new(&config.tui)?,
        })
    }
}
