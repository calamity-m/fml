#![cfg(feature = "integration")]

mod common;

use std::time::{Duration, Instant};

use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent},
    log::Source,
};

use common::make_entry;

/// Default per-source backfill cap from `IngestConfig`.
const LINES_PER_SOURCE: usize = 5000;
const SOURCES: usize = 20;

/// Documented startup-burst threshold: 20 sources × the 5,000-line default
/// cap (100k entries) must announce, ingest, and drain through the real event
/// loop within this budget on a debug build. The bound is deliberately loose
/// for CI; the point is catching order-of-magnitude regressions in the
/// per-event producer path, not micro-benchmarking.
const BURST_BUDGET: Duration = Duration::from_secs(10);

/// Emulate the worst accepted startup case: every source emits its full
/// capped backfill at once, racing the other sources for the producer
/// channel. Per-source order must survive, the store must hold every entry
/// (capacity exceeds the burst), and the loop must drain the backlog and
/// observe the quit within the documented budget.
#[tokio::test(flavor = "multi_thread")]
async fn capped_multi_source_backfill_burst_drains_within_budget() {
    let app = App::with_test_backend(Config::default(), 80, 24).expect("app construction");
    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    let started = Instant::now();

    let mut senders = Vec::new();
    for s in 0..SOURCES {
        let tx = producer_tx.clone();
        senders.push(tokio::spawn(async move {
            let source_id = format!("src-{s}");
            let source = Source {
                producer: "fake".to_string(),
                id: source_id.clone(),
                display_name: source_id.clone(),
                group: None,
            };
            tx.send(ProducerEvent::SourceFound(source))
                .await
                .expect("send source found");
            for i in 0..LINES_PER_SOURCE {
                tx.send(ProducerEvent::StoreEvent(make_entry(
                    &format!("backfill {i}"),
                    &source_id,
                )))
                .await
                .expect("send store event");
            }
        }));
    }

    // Quit only after every sender has delivered its burst into the channel.
    // The event loop's biased select drains queued producer events before it
    // observes the quit, so reaching the assertions means the whole backlog
    // was processed.
    tokio::spawn(async move {
        for sender in senders {
            sender.await.expect("sender task");
        }
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let elapsed = started.elapsed();

    assert_eq!(
        app.state.store.bounds(),
        (1, (SOURCES * LINES_PER_SOURCE) as u64)
    );
    assert_eq!(app.state.producer.sources.len(), SOURCES);
    assert_eq!(app.state.event_bus.producer_event_rx.len(), 0);
    assert!(
        elapsed < BURST_BUDGET,
        "startup burst took {elapsed:?}, budget is {BURST_BUDGET:?}"
    );
}
