use std::sync::Arc;

use crate::log::LogEntry;

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
            items: Vec::new(),
        }
    }

    pub fn start_surrounding(&mut self, anchor_seq: u64) {
        self.mode = PreviewMode::Surrounding;
        self.status = PreviewStatus::Loading;
        self.anchor_seq = Some(anchor_seq);
        self.items.clear();
    }

    pub fn clear(&mut self) {
        self.status = PreviewStatus::NoSelection;
        self.anchor_seq = None;
        self.items.clear();
    }

    pub fn apply_surrounding(&mut self, anchor_seq: u64, items: Vec<Arc<LogEntry>>) {
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
