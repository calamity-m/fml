use tokio::{sync::mpsc, task::JoinHandle};
use tracing::debug;

use crate::{event::SearchEvent, log::SourceId};

/// Starts the background worker for a history window search.
///
/// `middle_seq_id` identifies the center of the requested history window and
/// `buffer` describes how much surrounding context should be retrieved. This
/// worker is intended to load a bounded slice of logs around that anchor for
/// the selected `sources`, returning a task handle that supports cancellation.
pub fn start_history_search(
    middle_seq_id: u64,
    buffer: u64,
    sources: Vec<SourceId>,
    request_id: u64,
    tx: mpsc::Sender<SearchEvent>,
) -> JoinHandle<()> {
    let join_handle = tokio::spawn(async move {
        debug!(
            "spawned history search - middle_seq_id: {}, buffer: {}, sources: {:?}",
            middle_seq_id, buffer, sources
        );
    });

    join_handle
}
