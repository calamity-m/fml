use std::{
    io::{Stdout, stdout},
    sync::Arc,
};

use ratatui::{Terminal, prelude::CrosstermBackend};

use crate::{
    config::Config,
    error::FmlError,
    state::{
        events_bus::EventBus, producer_state::ProducerState, search_state::SearchState,
        tui_state::TuiState,
    },
    store::{LogStore, RingBufferStore},
    tui::widgets::{
        FmlWidget, info_pane::InfoPane, log_pane::LogPane, preview_pane::PreviewPane,
        query_box::QueryBox, status_bar::StatusBar,
    },
};

pub mod events_bus;
pub mod producer_state;
pub mod search_state;
pub mod tui_state;

pub struct AppState {
    pub config: Config,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub widgets: Vec<Box<dyn FmlWidget>>,
    pub event_bus: EventBus,
    pub store: Arc<dyn LogStore>,
    pub tui: TuiState,
    pub search: SearchState,
    pub producer: ProducerState,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, FmlError> {
        let (store, store_tx) = RingBufferStore::new(config.store.clone());
        let mut event_bus = EventBus::new();
        event_bus.register_store_tx(store_tx);

        Ok(AppState {
            config: config.clone(),
            terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
            widgets: vec![
                Box::new(QueryBox::new()),
                Box::new(StatusBar::new()),
                Box::new(LogPane::new()),
                Box::new(InfoPane::new()),
                Box::new(PreviewPane::new()),
            ],
            event_bus: event_bus,
            store: store,
            tui: TuiState::new(&config.tui)?,
            search: SearchState::new(&config.search)?,
            producer: ProducerState::new(),
        })
    }
}
