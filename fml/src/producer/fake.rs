use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use fake::{
    Fake,
    faker::{lorem::en::Sentence, name::en::Name},
};
use rand::{Rng, seq::IndexedRandom};
use tokio::sync::mpsc;
use tracing::debug;

use crate::{
    event::ProducerEvent,
    log::{LogLevel, NewLogEntry, Source, SourceId},
    producer::LogProducer,
};

const DEFAULT_TICK: Duration = Duration::from_millis(500);

/// A synthetic [`LogProducer`] used by the `--demo` flag.
///
/// Emits a single [`ProducerEvent::SourceFound`] for its [`Source`] then
/// ticks out randomised [`NewLogEntry`] values until [`stop`](Self::stop)
/// is called.
pub struct FakeProducer {
    source: Source,
    tick: Duration,
    cancel: Arc<AtomicBool>,
}

impl FakeProducer {
    pub fn new(source: Source) -> Self {
        Self::with_tick(source, DEFAULT_TICK)
    }

    pub fn with_tick(source: Source, tick: Duration) -> Self {
        Self {
            source,
            tick,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl LogProducer for FakeProducer {
    fn source_id(&self) -> SourceId {
        self.source.id.clone()
    }

    fn start(&self, tx: mpsc::Sender<ProducerEvent>) {
        let source = self.source.clone();
        let cancel = self.cancel.clone();
        let tick = self.tick;

        tokio::spawn(async move {
            if tx
                .send(ProducerEvent::SourceFound(source.clone()))
                .await
                .is_err()
            {
                debug!(
                    "fake producer {} aborting: event channel closed before first tick",
                    source.id
                );
                return;
            }

            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(tick).await;
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let entry = synthetic_entry(&source);
                if tx.send(ProducerEvent::StoreEvent(entry)).await.is_err() {
                    debug!(
                        "fake producer {} aborting: event channel closed",
                        source.id
                    );
                    break;
                }
            }
        });
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn synthetic_entry(source: &Source) -> NewLogEntry {
    const LEVELS: &[LogLevel] = &[
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Fatal,
    ];

    let mut rng = rand::rng();

    let level = LEVELS.choose(&mut rng).copied();
    let msg: String = Sentence(4..12).fake();
    let host: String = Name().fake();

    let mut fields: HashMap<String, serde_json::Value> = HashMap::new();
    fields.insert("host".to_string(), serde_json::Value::String(host));
    fields.insert(
        "latency_ms".to_string(),
        serde_json::Value::from(rng.random_range(1u32..1000)),
    );
    if rng.random_bool(0.3) {
        fields.insert(
            "request_id".to_string(),
            serde_json::Value::from(rng.random_range(1_000u32..9_999)),
        );
    }

    NewLogEntry {
        msg,
        ts: Utc::now(),
        level,
        source: source.clone(),
        fields,
    }
}
