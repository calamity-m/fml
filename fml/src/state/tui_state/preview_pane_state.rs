use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::error;

use crate::{
    event::{FieldPredicate, Query, SearchEvent, SearchTarget, SelectedEntry},
    log::LogEntry,
};

/// Display/search mode for the preview pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewMode {
    /// Entries around the selected log, in retained-sequence order.
    Surrounding,
    /// Entries matching selected field predicates from the anchor log.
    FieldMatched { predicates: Vec<FieldPredicate> },
}

/// Outcome of a preview mode cycle request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewModeCycle {
    /// The mode changed immediately.
    Applied,
    /// Field-matched mode needs the field picker before it can apply.
    NeedsFieldSelection,
    /// Cycling requires a selected log entry; mode left unchanged.
    NoSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    NoSelection,
    Loading,
    Ready,
    AnchorEvicted,
    NoMatches,
}

pub struct PreviewPaneState {
    pub mode: PreviewMode,
    pub status: PreviewStatus,
    pub anchor_seq: Option<u64>,
    pending_anchor_seq: Option<u64>,
    field_selection_previous_mode: Option<PreviewMode>,
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
            field_selection_previous_mode: None,
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
            self.dispatch_active_mode(selected_entry, buffer, search_tx);
        } else {
            self.clear();
            if let Err(err) = search_tx.try_send(SearchEvent::Cancel {
                target: SearchTarget::PreviewPane,
            }) {
                error!("failed to cancel preview search: {err}");
            }
        }
    }

    pub fn cycle_mode(
        &mut self,
        selected_entry: Option<&SelectedEntry>,
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) -> PreviewModeCycle {
        match &self.mode {
            PreviewMode::Surrounding => {
                if selected_entry.is_none() {
                    return PreviewModeCycle::NoSelection;
                }
                self.open_field_selection();
                PreviewModeCycle::NeedsFieldSelection
            }
            PreviewMode::FieldMatched { .. } => {
                self.mode = PreviewMode::Surrounding;
                if let Some(selected_entry) = selected_entry {
                    self.dispatch_active_mode(selected_entry, buffer, search_tx);
                }
                PreviewModeCycle::Applied
            }
        }
    }

    pub fn open_field_selection(&mut self) {
        if self.field_selection_previous_mode.is_none() {
            self.field_selection_previous_mode = Some(self.mode.clone());
        }
    }

    pub fn cancel_field_selection(&mut self) {
        if let Some(previous_mode) = self.field_selection_previous_mode.take() {
            self.mode = previous_mode;
        }
    }

    /// Treat a cancelled field picker as cycling past field-matched mode.
    pub fn skip_field_selection_cycle(&mut self) {
        if let Some(previous_mode) = self.field_selection_previous_mode.take() {
            self.mode = previous_mode;
        }
    }

    pub fn apply_field_selection(
        &mut self,
        selected_entry: &SelectedEntry,
        selected_keys: &[String],
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        let predicates = selected_keys
            .iter()
            .filter_map(|key| {
                selected_entry
                    .entry
                    .fields
                    .get(key)
                    .cloned()
                    .map(|value| FieldPredicate {
                        key: key.clone(),
                        value,
                    })
            })
            .collect();

        self.field_selection_previous_mode = None;
        self.mode = PreviewMode::FieldMatched { predicates };
        self.dispatch_active_mode(selected_entry, buffer, search_tx);
    }

    fn dispatch_active_mode(
        &mut self,
        selected_entry: &SelectedEntry,
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        match &self.mode {
            PreviewMode::Surrounding => {
                self.request_surrounding(selected_entry, buffer, search_tx);
            }
            PreviewMode::FieldMatched { .. } => {
                self.request_field_matched(selected_entry, buffer, search_tx);
            }
        }
    }

    fn request_field_matched(
        &mut self,
        selected_entry: &SelectedEntry,
        buffer: u64,
        search_tx: &mpsc::Sender<SearchEvent>,
    ) {
        let anchor_seq = selected_entry.entry.seq;
        let predicates = match &self.mode {
            PreviewMode::FieldMatched { predicates } => predicates.clone(),
            PreviewMode::Surrounding => Vec::new(),
        };
        self.pending_anchor_seq = Some(anchor_seq);

        if let Err(err) = search_tx.try_send(SearchEvent::Search {
            target: SearchTarget::PreviewPane,
            query: Query::FieldMatched {
                anchor_seq_id: anchor_seq,
                buffer,
                predicates,
            },
            sources: Vec::new(),
        }) {
            error!("failed to dispatch preview field-matched search: {err}");
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
        self.start_active_mode(anchor_seq);
    }

    pub fn start_active_mode(&mut self, anchor_seq: u64) {
        self.status = PreviewStatus::Loading;
        self.anchor_seq = Some(anchor_seq);
        self.pending_anchor_seq = None;
        self.items.clear();
    }

    pub fn clear(&mut self) {
        self.status = PreviewStatus::NoSelection;
        self.anchor_seq = None;
        self.pending_anchor_seq = None;
        self.field_selection_previous_mode = None;
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

    pub fn apply_field_matched(
        &mut self,
        anchor_seq: u64,
        items: Vec<Arc<LogEntry>>,
        retained_bounds: (u64, u64),
    ) {
        if let Some(pending_anchor_seq) = self.pending_anchor_seq {
            if pending_anchor_seq != anchor_seq {
                return;
            }
        } else if self.anchor_seq != Some(anchor_seq) {
            return;
        }

        self.anchor_seq = Some(anchor_seq);
        self.pending_anchor_seq = None;
        self.items = items;

        let (retained_low, retained_high) = retained_bounds;
        let anchor_retained =
            retained_low != 0 && anchor_seq >= retained_low && anchor_seq <= retained_high;
        self.status = if !anchor_retained {
            PreviewStatus::AnchorEvicted
        } else if self.items.is_empty() {
            PreviewStatus::NoMatches
        } else {
            PreviewStatus::Ready
        };
    }

    pub fn items(&self) -> &[Arc<LogEntry>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::log::{LogEntry, LogLevel, Source};

    fn selected_entry(
        seq: u64,
        source_id: &str,
        fields: HashMap<String, serde_json::Value>,
    ) -> SelectedEntry {
        SelectedEntry {
            entry: Arc::new(LogEntry {
                seq,
                msg: format!("entry {seq}"),
                ts: Utc::now(),
                level: Some(LogLevel::Info),
                source: Source {
                    producer: "fake".to_string(),
                    id: source_id.to_string(),
                    display_name: source_id.to_string(),
                    group: None,
                },
                fields,
            }),
            matches: Vec::new(),
        }
    }

    fn recv_search_event(rx: &mut mpsc::Receiver<SearchEvent>) -> SearchEvent {
        rx.try_recv().expect("search event")
    }

    #[test]
    fn cycle_with_selection_opens_field_selection() {
        let (tx, mut rx) = mpsc::channel(4);
        let selected = selected_entry(7, "src-a", HashMap::new());
        let mut state = PreviewPaneState::new();

        let result = state.cycle_mode(Some(&selected), 5, &tx);

        assert_eq!(result, PreviewModeCycle::NeedsFieldSelection);
        assert_eq!(state.mode, PreviewMode::Surrounding);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn no_selection_cycle_reports_no_selection() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut state = PreviewPaneState::new();

        assert_eq!(
            state.cycle_mode(None, 5, &tx),
            PreviewModeCycle::NoSelection
        );
        assert_eq!(state.mode, PreviewMode::Surrounding);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn field_selection_can_be_cancelled_without_losing_previous_mode() {
        let (tx, mut rx) = mpsc::channel(4);
        let selected = selected_entry(
            7,
            "src-a",
            HashMap::from([("request_id".to_string(), json!("abc"))]),
        );
        let mut state = PreviewPaneState::new();

        let result = state.cycle_mode(Some(&selected), 5, &tx);
        state.cancel_field_selection();

        assert_eq!(result, PreviewModeCycle::NeedsFieldSelection);
        assert_eq!(state.mode, PreviewMode::Surrounding);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn applying_field_selection_remembers_exact_predicates() {
        let (tx, mut rx) = mpsc::channel(4);
        let selected = selected_entry(
            7,
            "src-a",
            HashMap::from([
                ("request_id".to_string(), json!("abc")),
                ("status".to_string(), json!(500)),
            ]),
        );
        let mut state = PreviewPaneState::new();
        let keys = vec!["status".to_string(), "missing".to_string()];

        state.apply_field_selection(&selected, &keys, 5, &tx);

        assert_eq!(
            state.mode,
            PreviewMode::FieldMatched {
                predicates: vec![FieldPredicate {
                    key: "status".to_string(),
                    value: json!(500),
                }],
            }
        );
        match recv_search_event(&mut rx) {
            SearchEvent::Search {
                target,
                query:
                    Query::FieldMatched {
                        anchor_seq_id,
                        buffer,
                        predicates,
                    },
                sources,
            } => {
                assert_eq!(target, SearchTarget::PreviewPane);
                assert_eq!(anchor_seq_id, 7);
                assert_eq!(buffer, 5);
                assert_eq!(
                    predicates,
                    vec![FieldPredicate {
                        key: "status".to_string(),
                        value: json!(500),
                    }]
                );
                assert!(sources.is_empty());
            }
            event => panic!("expected field-matched preview search, got {event:?}"),
        }
    }

    #[test]
    fn selected_entry_change_redispatches_field_matched_cross_source() {
        let (tx, mut rx) = mpsc::channel(4);
        let selected = selected_entry(
            9,
            "src-b",
            HashMap::from([("request_id".to_string(), json!("abc"))]),
        );
        let mut state = PreviewPaneState::new();
        state.mode = PreviewMode::FieldMatched {
            predicates: vec![FieldPredicate {
                key: "request_id".to_string(),
                value: json!("abc"),
            }],
        };

        state.selected_entry_changed(Some(&selected), 3, &tx);

        match recv_search_event(&mut rx) {
            SearchEvent::Search {
                target,
                query:
                    Query::FieldMatched {
                        anchor_seq_id,
                        buffer,
                        predicates,
                    },
                sources,
            } => {
                assert_eq!(target, SearchTarget::PreviewPane);
                assert_eq!(anchor_seq_id, 9);
                assert_eq!(buffer, 3);
                assert_eq!(
                    predicates,
                    vec![FieldPredicate {
                        key: "request_id".to_string(),
                        value: json!("abc"),
                    }]
                );
                assert!(sources.is_empty());
            }
            event => panic!("expected field-matched preview search, got {event:?}"),
        }
    }

    #[test]
    fn field_matched_result_reports_no_matches_or_anchor_eviction() {
        let selected = selected_entry(7, "src-a", HashMap::new());
        let mut state = PreviewPaneState::new();
        state.mode = PreviewMode::FieldMatched {
            predicates: vec![FieldPredicate {
                key: "request_id".to_string(),
                value: json!("abc"),
            }],
        };
        state.pending_anchor_seq = Some(7);

        state.apply_field_matched(7, Vec::new(), (1, 10));

        assert_eq!(state.status, PreviewStatus::NoMatches);

        state.pending_anchor_seq = Some(7);
        state.apply_field_matched(7, vec![selected.entry], (8, 10));

        assert_eq!(state.status, PreviewStatus::AnchorEvicted);
    }

    #[test]
    fn stale_surrounding_result_is_rejected_while_new_anchor_is_pending() {
        let (tx, _rx) = mpsc::channel(4);
        let selected = selected_entry(7, "src-a", HashMap::new());
        let mut state = PreviewPaneState::new();

        state.selected_entry_changed(Some(&selected), 5, &tx);
        state.apply_surrounding(6, vec![selected.entry.clone()]);

        assert_eq!(state.status, PreviewStatus::NoSelection);
        assert!(state.items().is_empty());
    }
}
