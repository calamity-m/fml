use std::sync::Arc;

use tokio::{sync::mpsc, task::JoinHandle};
use tracing::debug;

use crate::{event::SearchEvent, log::SourceId, store::LogStore};

/// Starts the background worker for a fuzzy text search.
///
/// Fuzzy searches are intended to scan the selected `sources` for entries that
/// approximately match `term`, producing ranked hits that can be surfaced in
/// the search-oriented UI mode. As with the other workers, the returned
/// [`JoinHandle`] is used to cancel superseded work.
pub fn start_fuzzy_search(
    term: String,
    sources: Vec<SourceId>,
    store: Arc<dyn LogStore>,
    request_id: u64,
    tx: mpsc::Sender<SearchEvent>,
) -> JoinHandle<()> {
    let join_handle = tokio::spawn(async move {
        debug!(
            "spawned tail search - term: {}, sources: {:?}, request_id: {}",
            term, sources, request_id
        );
    });

    join_handle
}
