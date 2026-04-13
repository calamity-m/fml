use tokio::sync::mpsc;

use crate::{
    event::{QuitEvent, SearchEvent, TuiEvent},
    log::NewLogEntry,
};

pub struct EventBus {
    pub tui_event_tx: mpsc::UnboundedSender<TuiEvent>,
    pub tui_event_rx: mpsc::UnboundedReceiver<TuiEvent>,

    pub search_event_tx: mpsc::Sender<SearchEvent>,
    pub search_event_rx: mpsc::Receiver<SearchEvent>,
    /// Our quit sender
    pub quit_tx: mpsc::Sender<QuitEvent>,
    pub quit_rx: mpsc::Receiver<QuitEvent>,
    /// For our store
    pub store_tx: Option<mpsc::Sender<NewLogEntry>>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tui_event_tx, tui_event_rx) = mpsc::unbounded_channel();
        let (search_event_tx, search_event_rx) = mpsc::channel(8092);
        let (quit_tx, quit_rx) = mpsc::channel(1);

        Self {
            tui_event_tx,
            tui_event_rx,
            quit_tx,
            quit_rx,
            store_tx: None,
            search_event_tx: search_event_tx,
            search_event_rx: search_event_rx,
        }
    }

    pub fn register_store_tx(&mut self, store_tx: mpsc::Sender<NewLogEntry>) {
        self.store_tx = Some(store_tx);
    }
}
