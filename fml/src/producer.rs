use tokio::sync::mpsc;
use tracing::debug;

use crate::{event::ProducerEvent, log::NewLogEntry, state::AppState};

pub mod fake;

pub trait LogProducer {
    fn start(&self, tx: mpsc::Sender<ProducerEvent>);
    fn stop(&self);
}

pub fn handle_producer_event(event: ProducerEvent, mut state: AppState) -> AppState {
    let new_state = match event {
        ProducerEvent::SourceFound(source) => {
            debug!("received source found event - {:?}", source);

            state
        }
        ProducerEvent::SourceLost(source_id) => {
            debug!("received source lost event - {}", source_id);

            state
        }
        ProducerEvent::StoreEvent(new_log_entry) => {
            debug!("received new entry store event - {:?}", new_log_entry);

            state
        }
    };

    new_state
}
