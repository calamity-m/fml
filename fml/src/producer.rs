//! Log producer trait and event reducer.
//!
//! A [`LogProducer`] ingests log lines from outside the app (e.g. a docker
//! container, a kube pod, a tailed file) and emits them as
//! [`ProducerEvent`]s onto the event bus. The [`App`] holds producers as
//! `Box<dyn LogProducer>`, calls [`LogProducer::start`] once after the TUI
//! spawns, and calls [`LogProducer::stop`] once during shutdown.
//!
//! ## Cancellation contract
//!
//! Both `start` and `stop` take `&self`, so a producer that spawns a
//! background task in `start` cannot move owned cancellation state into
//! the task and then mutate it from `stop`. Implementations must keep
//! cancellation state behind a shared handle (e.g. `Arc<AtomicBool>` or a
//! `tokio_util::sync::CancellationToken`) cloned into the spawned task;
//! `stop` flips/triggers the handle and the task observes it on its next
//! iteration and exits.
//!
//! [`App`]: crate::app::App

use tokio::sync::mpsc;
use tracing::debug;

use crate::{event::ProducerEvent, log::SourceId, state::AppState};

pub mod fake;

/// A log source ingester.
///
/// Implementations are required to be `Send + Sync` because `start` and
/// `stop` are invoked from the main async event loop while the producer's
/// spawned task may run on any executor thread.
///
/// See the [module docs](self) for the cancellation contract.
pub trait LogProducer: Send + Sync {
    /// Stable id identifying this producer's source. Must match the
    /// `source.id` of every [`NewLogEntry`] the producer emits so multi-source
    /// filters in tail/history/fuzzy stay meaningful.
    ///
    /// [`NewLogEntry`]: crate::log::NewLogEntry
    fn source_id(&self) -> SourceId;

    /// Begin producing events on `tx`. Implementations should emit a
    /// [`ProducerEvent::SourceFound`] before any
    /// [`ProducerEvent::StoreEvent`] so the app can register the source.
    ///
    /// `start` is expected to return promptly — long-running work belongs
    /// inside a task spawned from `start`, not inside `start` itself.
    fn start(&self, tx: mpsc::Sender<ProducerEvent>);

    /// Signal the spawned task to halt. See the
    /// [cancellation contract](self#cancellation-contract).
    fn stop(&self);
}

/// Apply a single [`ProducerEvent`] to the application state.
///
/// `SourceFound` and `SourceLost` mutate `state.producer.sources` so the
/// rest of the app sees an up-to-date list of live sources. `StoreEvent`
/// is inserted directly into the [`LogStore`].
///
/// [`LogStore`]: crate::store::LogStore
pub fn handle_producer_event(event: ProducerEvent, mut state: AppState) -> AppState {
    match event {
        ProducerEvent::SourceFound(source) => {
            debug!("received source found event - {:?}", source);
            if !state.producer.sources.iter().any(|s| s.id == source.id) {
                state.producer.sources.push(source);
            }
        }
        ProducerEvent::SourceLost(source_id) => {
            debug!("received source lost event - {}", source_id);
            state.producer.sources.retain(|s| s.id != source_id);
        }
        ProducerEvent::StoreEvent(entry) => {
            debug!("received new entry store event - {:?}", entry);
            state.store.insert(entry);
        }
    }

    state
}
