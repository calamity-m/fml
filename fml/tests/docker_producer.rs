#![cfg(feature = "integration")]
//! Docker producer integration tests.
//!
//! These tests require a running Docker daemon. They fail loudly when none is
//! available; use `cargo test --features integration` only on machines with
//! Docker.

use std::time::Duration;

use fml::{
    event::ProducerEvent,
    producer::{LogProducer, SourceBlock, docker::DockerProducer},
};
use testcontainers::{GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner};
use tokio::sync::mpsc;

#[tokio::test]
async fn docker_producer_streams_container_logs_and_source_lifecycle() {
    let container = GenericImage::new("busybox", "latest")
        .with_wait_for(WaitFor::seconds(1))
        .with_cmd([
            "sh",
            "-c",
            "i=0; while true; do echo line-$i; i=$((i+1)); sleep 0.1; done",
        ])
        .start()
        .await
        .expect("busybox container should start");
    let container_id = container.id().to_string();

    let producer =
        DockerProducer::new(SourceBlock::none()).expect("docker producer should connect");
    let (tx, mut rx) = mpsc::channel(128);
    producer.start(tx);

    let mut saw_source_found = false;
    let mut store_events = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && (!saw_source_found || store_events < 3) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Some(event) = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timed out waiting for docker producer events")
        else {
            panic!("producer event channel closed");
        };

        match event {
            ProducerEvent::SourceFound(source) if source.id == container_id => {
                saw_source_found = true;
            }
            ProducerEvent::StoreEvent(entry) if entry.source.id == container_id => {
                assert!(
                    entry.msg.starts_with("line-"),
                    "unexpected busybox log line: {}",
                    entry.msg
                );
                store_events += 1;
            }
            _ => {}
        }
    }

    assert!(saw_source_found, "expected SourceFound for test container");
    assert!(
        store_events >= 3,
        "expected at least 3 StoreEvents for test container, got {store_events}"
    );

    container.stop().await.expect("container should stop");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_source_lost = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Some(event) = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timed out waiting for docker SourceLost")
        else {
            panic!("producer event channel closed");
        };

        if matches!(event, ProducerEvent::SourceLost(source_id) if source_id == container_id) {
            saw_source_lost = true;
            break;
        }
    }

    producer.stop();
    assert!(saw_source_lost, "expected SourceLost for test container");
}
