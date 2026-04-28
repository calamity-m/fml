use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::error;

use crate::{
    event::{Query, SearchEvent, SearchTarget, SelectedEntry},
    log::LogEntry,
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
    items: Vec<Arc<LogEntry>>,
}

impl Default for PreviewPaneState {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewPaneState {
    pub fn new() -> Self {
        Self {
            mode: PreviewMode::Surrounding,
            status: PreviewStatus::NoSelection,
            anchor_seq: None,
            pending_anchor_seq: None,
            items: Vec::new(),
        }
    }

    pub fn selected_entry_changed(
        &mut self,
        selected_entry: Option<&SelectedEntry>,
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        if let Some(selected_entry) = selected_entry {
            self.request_surrounding(selected_entry, buffer, search_tx);
        } else {
            self.clear();
            if let Err(err) = search_tx.try_send(SearchEvent::Cancel {
                target: SearchTarget::PreviewPane,
            }) {
                error!("failed to cancel preview search: {err}");
            }
        }
    }

    fn request_surrounding(
        &mut self,
        selected_entry: &SelectedEntry,
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        let anchor_seq = selected_entry.entry.seq;
        let source_id = selected_entry.entry.source.id.clone();
        self.mode = PreviewMode::Surrounding;
        self.pending_anchor_seq = Some(anchor_seq);

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
        if let Some(pending_anchor_seq) = self.pending_anchor_seq {
            if pending_anchor_seq != anchor_seq {
                return;
            }
        } else if self.anchor_seq != Some(anchor_seq) {
            return;
        }

        self.anchor_seq = Some(anchor_seq);
        self.pending_anchor_seq = None;

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
