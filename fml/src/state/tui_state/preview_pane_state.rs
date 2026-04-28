use std::{sync::Arc, time::Duration};

use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, error};

use crate::{
    event::{Query, SearchEvent, SearchTarget, SelectedEntry, TuiEvent},
    log::{LogEntry, SourceId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    Surrounding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    NoSelection,
    Loading,
    Ready,
    AnchorEvicted,
}

pub struct PreviewPaneState {
    pub mode: PreviewMode,
    pub status: PreviewStatus,
    pub anchor_seq: Option<u64>,
    pending_anchor_seq: Option<u64>,
    debounce_ms: u64,
    debounce_handle: Option<JoinHandle<()>>,
    items: Vec<Arc<LogEntry>>,
}

impl Default for PreviewPaneState {
    fn default() -> Self {
        Self::new(75)
    }
}

impl PreviewPaneState {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            mode: PreviewMode::Surrounding,
            status: PreviewStatus::NoSelection,
            anchor_seq: None,
            pending_anchor_seq: None,
            debounce_ms,
            debounce_handle: None,
            items: Vec::new(),
        }
    }

    pub fn selected_entry_changed(
        &mut self,
        selected_entry: Option<&SelectedEntry>,
        tui_tx: &mpsc::UnboundedSender<TuiEvent>,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        if let Some(selected_entry) = selected_entry {
            self.schedule_surrounding(selected_entry, tui_tx, search_tx);
        } else {
            self.cancel_pending();
            self.clear();
            if let Err(err) = search_tx.try_send(SearchEvent::Cancel {
                target: SearchTarget::PreviewPane,
            }) {
                error!("failed to cancel preview search: {err}");
            }
        }
    }

    pub fn dispatch_surrounding(
        &mut self,
        anchor_seq: u64,
        source_id: SourceId,
        selected_entry: Option<&SelectedEntry>,
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        let Some(selected_entry) = selected_entry else {
            return;
        };

        if selected_entry.entry.seq != anchor_seq || selected_entry.entry.source.id != source_id {
            return;
        }

        self.start_surrounding(anchor_seq);
        if let Err(err) = search_tx.try_send(SearchEvent::Search {
            target: SearchTarget::PreviewPane,
            query: Query::Surrounding {
                middle_seq_id: anchor_seq,
                buffer,
            },
            sources: vec![source_id],
        }) {
            error!("failed to dispatch preview surrounding search: {err}");
        }
    }

    fn schedule_surrounding(
        &mut self,
        selected_entry: &SelectedEntry,
        tui_tx: &mpsc::UnboundedSender<TuiEvent>,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        self.cancel_pending();

        let anchor_seq = selected_entry.entry.seq;
        let source_id = selected_entry.entry.source.id.clone();
        self.mode = PreviewMode::Surrounding;
        self.pending_anchor_seq = Some(anchor_seq);
        if let Err(err) = search_tx.try_send(SearchEvent::Cancel {
            target: SearchTarget::PreviewPane,
        }) {
            error!("failed to cancel pending preview search: {err}");
        }

        let tx = tui_tx.clone();
        let debounce_ms = self.debounce_ms;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            self.debounce_handle = Some(handle.spawn(async move {
                tokio::time::sleep(Duration::from_millis(debounce_ms)).await;
                if let Err(err) = tx.send(TuiEvent::DispatchPreviewSurrounding {
                    anchor_seq,
                    source_id,
                }) {
                    debug!("failed to schedule preview surrounding search - {err}");
                }
            }));
        } else if let Err(err) = tx.send(TuiEvent::DispatchPreviewSurrounding {
            anchor_seq,
            source_id,
        }) {
            debug!("failed to schedule preview surrounding search - {err}");
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(handle) = self.debounce_handle.take() {
            handle.abort();
        }
    }

    pub fn start_surrounding(&mut self, anchor_seq: u64) {
        self.mode = PreviewMode::Surrounding;
        self.status = PreviewStatus::Loading;
        self.anchor_seq = Some(anchor_seq);
        self.pending_anchor_seq = None;
        self.items.clear();
    }

    pub fn clear(&mut self) {
        self.status = PreviewStatus::NoSelection;
        self.anchor_seq = None;
        self.pending_anchor_seq = None;
        self.items.clear();
    }

    pub fn apply_surrounding(&mut self, anchor_seq: u64, items: Vec<Arc<LogEntry>>) {
        if self.pending_anchor_seq.is_some() {
            return;
        }

        if self.anchor_seq != Some(anchor_seq) {
            return;
        }

        let anchor_retained = items.iter().any(|entry| entry.seq == anchor_seq);
        self.items = items;
        self.status = if anchor_retained {
            PreviewStatus::Ready
        } else {
            PreviewStatus::AnchorEvicted
        };
    }

    pub fn items(&self) -> &[Arc<LogEntry>] {
        &self.items
    }
}
