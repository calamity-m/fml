use tokio::sync::mpsc;

use crate::event::{ProducerEvent, QuitEvent, SearchEvent, TuiEvent};

pub struct EventBus {
    pub tui_event_tx: mpsc::UnboundedSender<TuiEvent>,
    pub tui_event_rx: mpsc::UnboundedReceiver<TuiEvent>,

    pub search_event_tx: mpsc::Sender<SearchEvent>,
    pub search_event_rx: mpsc::Receiver<SearchEvent>,

    pub producer_event_tx: mpsc::Sender<ProducerEvent>,
    pub producer_event_rx: mpsc::Receiver<ProducerEvent>,

    /// Our quit sender
    pub quit_tx: mpsc::Sender<QuitEvent>,
    pub quit_rx: mpsc::Receiver<QuitEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tui_event_tx, tui_event_rx) = mpsc::unbounded_channel();
        let (search_event_tx, search_event_rx) = mpsc::channel(8092);
        let (producer_event_tx, producer_event_rx) = mpsc::channel(8092);

        let (quit_tx, quit_rx) = mpsc::channel(1);

        Self {
            tui_event_tx,
            tui_event_rx,
            quit_tx,
            quit_rx,
            search_event_tx,
            search_event_rx,
            producer_event_tx,
            producer_event_rx,
        }
    }
}
