#![cfg(feature = "integration")]

use std::{fs::OpenOptions, io::Write as _, time::Duration};

use fml::{
    event::ProducerEvent,
    producer::{LogProducer, file::FileProducer},
};
use tokio::{sync::mpsc, time::timeout};

async fn next_event(rx: &mut mpsc::Receiver<ProducerEvent>) -> ProducerEvent {
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for producer event")
        .expect("producer event channel closed")
}

async fn wait_for_source_found(rx: &mut mpsc::Receiver<ProducerEvent>) {
    loop {
        if matches!(next_event(rx).await, ProducerEvent::SourceFound(_)) {
            return;
        }
    }
}

async fn collect_store_messages(
    rx: &mut mpsc::Receiver<ProducerEvent>,
    count: usize,
) -> Vec<String> {
    let mut messages = Vec::new();
    while messages.len() < count {
        if let ProducerEvent::StoreEvent(entry) = next_event(rx).await {
            messages.push(entry.msg);
        }
    }
    messages
}

fn append_lines(path: &std::path::Path, lines: &[&str]) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open file for append");
    for line in lines {
        writeln!(file, "{line}").expect("write line");
    }
}

#[tokio::test]
async fn file_producer_emits_appended_lines_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    std::fs::File::create(&path).expect("create log file");
    let producer = FileProducer::new(path.clone());
    let (tx, mut rx) = mpsc::channel(32);

    producer.start(tx);
    wait_for_source_found(&mut rx).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    append_lines(&path, &["one", "two", "three"]);

    assert_eq!(
        collect_store_messages(&mut rx, 3).await,
        ["one", "two", "three"]
    );

    producer.stop();
}

#[tokio::test]
async fn file_producer_survives_rename_recreate_rotation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    let rotated_path = dir.path().join("app.log.1");
    std::fs::File::create(&path).expect("create log file");
    let producer = FileProducer::new(path.clone());
    let (tx, mut rx) = mpsc::channel(64);

    producer.start(tx);
    wait_for_source_found(&mut rx).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    append_lines(&path, &["old-1", "old-2", "old-3", "old-4", "old-5"]);
    let mut messages = collect_store_messages(&mut rx, 5).await;

    std::fs::rename(&path, &rotated_path).expect("rotate file");
    std::fs::File::create(&path).expect("recreate log file");
    append_lines(&path, &["new-1", "new-2", "new-3", "new-4", "new-5"]);

    messages.extend(collect_store_messages(&mut rx, 5).await);

    assert_eq!(
        messages,
        [
            "old-1", "old-2", "old-3", "old-4", "old-5", "new-1", "new-2", "new-3", "new-4",
            "new-5"
        ]
    );

    producer.stop();
}
