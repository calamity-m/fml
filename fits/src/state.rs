use std::io::{Stdout, stdout};

use ratatui::{Terminal, prelude::CrosstermBackend};
use tokio::sync::mpsc;

use crate::{
    config::Config,
    error::FmlError,
    event::{QuitEvent, TuiEvent},
    message::Message,
    state::{events_bus::EventBus, tui_state::TuiState},
};

pub mod events_bus;
pub mod tui_state;

pub struct AppState {
    pub config: Config,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub events: EventBus,
    pub tui: TuiState,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, FmlError> {
        Ok(AppState {
            config,
            terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
            events: EventBus::new(),
            tui: TuiState::new(),
        })
    }
}
