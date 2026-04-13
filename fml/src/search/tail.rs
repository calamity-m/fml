use tokio::{sync::mpsc, task::JoinHandle};
use tracing::debug;

use crate::{event::SearchEvent, log::SourceId};

/// Starts the background worker for a tail-oriented search request.
///
/// Tail searches are intended to resolve the most recent log entries for the
/// selected `sources`, producing results that keep the UI anchored to the live
/// end of the stream. The returned [`JoinHandle`] lets the caller cancel the
/// task when a newer request supersedes it.
pub fn start_tail_search(
    sources: Vec<SourceId>,
    request_id: u64,
    tx: mpsc::Sender<SearchEvent>,
) -> JoinHandle<()> {
    let join_handle = tokio::spawn(async move {
        debug!("spawned tail search - sources: {:?}", sources);
    });

    join_handle
}
