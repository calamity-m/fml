use std::sync::Arc;

use crate::{
    config::Config,
    error::FmlError,
    state::{
        events_bus::EventBus, producer_state::ProducerState, search_state::SearchState,
        tui_state::TuiState,
    },
    store::{LogStore, RingBufferStore},
    tui::widgets::{
        FmlPopupWidget, FmlWidget, field_picker::FieldPicker, help::Help, info_pane::InfoPane,
        log_pane::LogPane, preview_pane::PreviewPane, query_box::QueryBox,
        source_selector::SourceSelector, status_bar::StatusBar,
    },
};

pub mod events_bus;
pub mod producer_state;
pub mod search_state;
pub mod tui_state;

pub struct AppState {
    pub config: Config,
    pub widgets: Vec<Box<dyn FmlWidget>>,
    pub popup_widgets: Vec<Box<dyn FmlPopupWidget>>,
    pub event_bus: EventBus,
    pub store: Arc<dyn LogStore>,
    pub tui: TuiState,
    pub search: SearchState,
    pub producer: ProducerState,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, FmlError> {
        let store = RingBufferStore::new(config.store.clone());
        let event_bus = EventBus::new();
        let mut tui = TuiState::new(&config.tui, &config.search)?;
        tui.log_pane.set_store_stats(store.stats());

        Ok(AppState {
            config: config.clone(),
            widgets: vec![
                Box::new(QueryBox::new()),
                Box::new(StatusBar::new()),
                Box::new(LogPane::new()),
                Box::new(InfoPane::new()),
                Box::new(PreviewPane::new()),
            ],
            popup_widgets: vec![
                Box::new(SourceSelector::new()),
                Box::new(FieldPicker::new()),
                Box::new(Help::new()),
            ],
            event_bus,
            store,
            tui,
            search: SearchState::new(),
            producer: ProducerState::new(),
        })
    }
}
