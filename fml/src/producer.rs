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

use std::path::PathBuf;

use tokio::sync::mpsc;
use tracing::debug;

use crate::{error::ProducerError, event::ProducerEvent, state::AppState};

/// Parsed form of a `--producer KIND[:ARG]` CLI value.
#[derive(Debug, Clone, PartialEq)]
pub enum ProducerSpec {
    Demo,
    File(PathBuf),
    Docker,
    Kubernetes(Option<String>),
}

impl ProducerSpec {
    /// Parse a `--producer` CLI string into a [`ProducerSpec`].
    ///
    /// Uses `splitn(2, ':')` so paths containing `:` are passed through
    /// whole as the file argument (e.g. `file:/var/log/2026-05-02:00:00.log`).
    pub fn parse(s: &str) -> Result<Self, ProducerError> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        let (kind, arg) = match parts.as_slice() {
            [k] => (*k, None),
            [k, a] => (*k, Some(*a)),
            _ => unreachable!(),
        };
        match (kind, arg) {
            ("demo", None) => Ok(ProducerSpec::Demo),
            ("demo", Some(_)) => Err(ProducerError::Cli(
                "`demo` takes no argument".to_string(),
            )),
            ("file", Some(path)) if !path.is_empty() => {
                Ok(ProducerSpec::File(PathBuf::from(path)))
            }
            ("file", _) => Err(ProducerError::Cli(
                "`file` requires a path argument: `file:<path>`".to_string(),
            )),
            ("docker", None) => Ok(ProducerSpec::Docker),
            ("docker", Some(_)) => Err(ProducerError::Cli(
                "`docker` takes no argument".to_string(),
            )),
            ("kubernetes", None) => Ok(ProducerSpec::Kubernetes(None)),
            ("kubernetes", Some("")) => Err(ProducerError::Cli(
                "`kubernetes` namespace cannot be empty; omit the colon to use the active context namespace".to_string(),
            )),
            ("kubernetes", Some(ns)) => Ok(ProducerSpec::Kubernetes(Some(ns.to_string()))),
            (kind, _) => Err(ProducerError::Cli(format!(
                "unknown producer kind `{kind}`; expected: demo, file:<path>, docker, kubernetes[:<namespace>]"
            ))),
        }
    }
}

pub mod docker;
pub mod fake;
pub mod file;
pub mod kubernetes;
pub mod normalizer;

/// A log source ingester.
///
/// A producer may back one or many [`Source`]s — e.g. a docker or kubernetes
/// producer discovers sources at runtime as containers/pods come and go.
/// Sources are announced by emitting [`ProducerEvent::SourceFound`] before
/// any [`ProducerEvent::StoreEvent`] referencing them, and retired with
/// [`ProducerEvent::SourceLost`]. The `source.id` and `source.producer`
/// fields on every emitted entry must be stable across the producer's
/// lifetime so multi-source filters in tail/history/fuzzy stay meaningful.
///
/// Implementations are required to be `Send + Sync` because `start` and
/// `stop` are invoked from the main async event loop while the producer's
/// spawned task may run on any executor thread.
///
/// See the [module docs](self) for the cancellation contract.
///
/// [`Source`]: crate::log::Source
pub trait LogProducer: Send + Sync {
    /// Begin producing events on `tx`. Implementations should emit a
    /// [`ProducerEvent::SourceFound`] for each source before any
    /// [`ProducerEvent::StoreEvent`] referencing it.
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
                // Keep newly discovered sources visible by default even while
                // the popup is open. The popup's open_sources snapshot is left
                // untouched, so the new row appears only after reopening.
                state.tui.enable_source_id(source.id.clone());
                state.producer.sources.push(source);
            }
        }
        ProducerEvent::SourceLost(source_id) => {
            debug!("received source lost event - {}", source_id);
            state.tui.remove_source_id(&source_id);
            state.producer.sources.retain(|s| s.id != source_id);
        }
        ProducerEvent::StoreEvent(entry) => {
            debug!("received new entry store event - {:?}", entry);
            state.store.insert(entry);
            state.tui.log_pane.set_store_stats(state.store.stats());
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use serial_test::serial;

    use super::*;
    use crate::{
        config::Config,
        event::ProducerEvent,
        log::{LogLevel, NewLogEntry, Source},
        state::AppState,
    };

    fn source(id: &str) -> Source {
        Source {
            producer: "fake".to_string(),
            id: id.to_string(),
            display_name: format!("Source {id}"),
            group: None,
        }
    }

    fn entry(msg: &str, source_id: &str) -> NewLogEntry {
        NewLogEntry {
            msg: msg.to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: source(source_id),
            fields: HashMap::new(),
        }
    }

    fn test_state() -> AppState {
        AppState::new(Config::default()).expect("test app state should construct")
    }

    #[test]
    #[serial]
    fn source_found_is_idempotent() {
        let state = test_state();

        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);
        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);

        assert_eq!(state.producer.sources.len(), 1);
        assert_eq!(state.producer.sources[0].id, "src-a");
    }

    #[test]
    #[serial]
    fn source_lost_removes_matching_source() {
        let state = test_state();
        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);
        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-b")), state);

        let state = handle_producer_event(ProducerEvent::SourceLost("src-a".to_string()), state);

        assert_eq!(state.producer.sources.len(), 1);
        assert_eq!(state.producer.sources[0].id, "src-b");
    }

    #[test]
    #[serial]
    fn source_found_enables_new_source_id() {
        let state = test_state();

        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);

        assert!(
            state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-a")
        );
    }

    #[test]
    #[serial]
    fn duplicate_source_found_does_not_reenable_disabled_source() {
        let state = test_state();
        let mut state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);
        state.tui.source_selector.enabled_source_ids.remove("src-a");

        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);

        assert!(
            !state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-a"),
            "duplicate SourceFound should not undo an explicit user disable"
        );
    }

    #[test]
    #[serial]
    fn source_lost_removes_enabled_source_id() {
        let state = test_state();
        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);

        let state = handle_producer_event(ProducerEvent::SourceLost("src-a".to_string()), state);

        assert!(
            !state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-a")
        );
    }

    #[test]
    #[serial]
    fn source_lost_while_popup_open_keeps_snapshot_but_removes_enabled_id() {
        let state = test_state();
        let state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);
        let mut state = handle_producer_event(ProducerEvent::SourceFound(source("src-b")), state);
        state.tui.open_source_selector(&state.producer.sources);

        let state = handle_producer_event(ProducerEvent::SourceLost("src-a".to_string()), state);

        assert!(
            state
                .producer
                .sources
                .iter()
                .all(|source| source.id != "src-a")
        );
        assert!(
            state
                .tui
                .source_selector
                .open_sources
                .iter()
                .any(|source| source.id == "src-a"),
            "the open popup renders from its open-time snapshot"
        );
        assert!(
            !state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-a")
        );
    }

    #[test]
    #[serial]
    fn source_found_while_popup_open_enables_but_does_not_change_snapshot_until_reopen() {
        let state = test_state();
        let mut state = handle_producer_event(ProducerEvent::SourceFound(source("src-a")), state);
        state.tui.open_source_selector(&state.producer.sources);

        let mut state = handle_producer_event(ProducerEvent::SourceFound(source("src-b")), state);

        assert!(
            state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-b")
        );
        assert!(
            state
                .tui
                .source_selector
                .open_sources
                .iter()
                .all(|source| source.id != "src-b"),
            "the current popup snapshot is intentionally stable"
        );

        state.tui.close_source_selector();
        state.tui.open_source_selector(&state.producer.sources);

        assert!(
            state
                .tui
                .source_selector
                .open_sources
                .iter()
                .any(|source| source.id == "src-b")
        );
    }

    #[test]
    #[serial]
    fn store_event_inserts_into_store_and_advances_bounds() {
        let state = test_state();

        let state = handle_producer_event(
            ProducerEvent::StoreEvent(entry("hello from producer", "src-a")),
            state,
        );

        assert_eq!(state.store.bounds(), (1, 1));
        assert_eq!(state.tui.log_pane.store_stats.retained, 1);
        assert_eq!(state.tui.log_pane.store_stats.bounds, (1, 1));

        let mut entries = Vec::new();
        state
            .store
            .fetch_requested(&[1], &mut entries)
            .expect("fetch should succeed");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].msg, "hello from producer");
        assert_eq!(entries[0].source.id, "src-a");
    }
}

#[cfg(test)]
mod spec_tests {
    use std::path::PathBuf;

    use super::ProducerSpec;

    #[test]
    fn parse_demo() {
        assert_eq!(ProducerSpec::parse("demo").unwrap(), ProducerSpec::Demo);
    }

    #[test]
    fn parse_file() {
        assert_eq!(
            ProducerSpec::parse("file:/var/log/app.log").unwrap(),
            ProducerSpec::File(PathBuf::from("/var/log/app.log")),
        );
    }

    #[test]
    fn parse_file_path_with_colons() {
        assert_eq!(
            ProducerSpec::parse("file:/var/log/2026-05-02:12:00.log").unwrap(),
            ProducerSpec::File(PathBuf::from("/var/log/2026-05-02:12:00.log")),
        );
    }

    #[test]
    fn parse_docker() {
        assert_eq!(ProducerSpec::parse("docker").unwrap(), ProducerSpec::Docker);
    }

    #[test]
    fn parse_kubernetes_bare() {
        assert_eq!(
            ProducerSpec::parse("kubernetes").unwrap(),
            ProducerSpec::Kubernetes(None),
        );
    }

    #[test]
    fn parse_kubernetes_with_namespace() {
        assert_eq!(
            ProducerSpec::parse("kubernetes:my-ns").unwrap(),
            ProducerSpec::Kubernetes(Some("my-ns".to_string())),
        );
    }

    #[test]
    fn parse_demo_with_arg_is_error() {
        assert!(ProducerSpec::parse("demo:foo").is_err());
    }

    #[test]
    fn parse_file_without_path_is_error() {
        assert!(ProducerSpec::parse("file").is_err());
    }

    #[test]
    fn parse_docker_with_arg_is_error() {
        assert!(ProducerSpec::parse("docker:foo").is_err());
    }

    #[test]
    fn parse_kubernetes_empty_namespace_is_error() {
        assert!(ProducerSpec::parse("kubernetes:").is_err());
    }

    #[test]
    fn parse_unknown_kind_is_error() {
        assert!(ProducerSpec::parse("syslog").is_err());
    }
}
