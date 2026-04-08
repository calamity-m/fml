use tokio::sync::mpsc;

use crate::{
    event::{QuitEvent, TuiEvent},
    log::NewLogEntry,
};

pub struct EventBus {
    pub tui_event_tx: mpsc::UnboundedSender<TuiEvent>,
    pub tui_event_rx: mpsc::UnboundedReceiver<TuiEvent>,
    /// Our quit sender
    pub quit_tx: mpsc::Sender<QuitEvent>,
    pub quit_rx: mpsc::Receiver<QuitEvent>,
    /// For our store
    pub store_tx: Option<mpsc::Sender<NewLogEntry>>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tui_event_tx, tui_event_rx) = mpsc::unbounded_channel();
        let (quit_tx, quit_rx) = mpsc::channel(1);

        Self {
            tui_event_tx,
            tui_event_rx,
            quit_tx,
            quit_rx,
            store_tx: None,
        }
    }

    pub fn register_store_tx(&mut self, store_tx: mpsc::Sender<NewLogEntry>) {
        self.store_tx = Some(store_tx);
    }
}
