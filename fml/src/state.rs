use std::sync::Arc;

use crate::{
    config::{Config, tui::ThemeConfig},
    error::FmlError,
    state::{events_bus::EventBus, producer_state::ProducerState, search_state::SearchState},
    store::{LogStore, RingBufferStore},
    tui::workspace::Workspace,
};

pub mod events_bus;
pub mod producer_state;
pub mod search_state;

pub struct AppState {
    pub config: Config,
    /// Theme resolved once at startup from `config.tui.theme`.
    pub theme: ThemeConfig,
    pub event_bus: EventBus,
    pub store: Arc<dyn LogStore>,
    pub workspace: Workspace,
    pub search: SearchState,
    pub producer: ProducerState,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, FmlError> {
        let theme = config.tui.resolved_theme_with(&config.themes)?;
        let line_wrap = config.tui.line_wrap;
        Ok(AppState {
            store: RingBufferStore::new(config.store.clone()),
            config,
            theme,
            event_bus: EventBus::new(),
            workspace: Workspace::new(line_wrap),
            search: SearchState::new(),
            producer: ProducerState::new(),
        })
    }
}
