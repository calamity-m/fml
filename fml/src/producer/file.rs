//! File-backed log producer.
//!
//! `FileProducer` tails one path from EOF when the file already exists and
//! watches the parent directory so common rotation strategies (rename +
//! recreate, delete + recreate) are visible. Copy-truncate rotation is handled
//! by reopening when the observed file length moves behind the current read
//! cursor, but historical bytes removed by truncation are not replayed.

use std::{
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    event::ProducerEvent,
    log::Source,
    producer::{LogProducer, normalizer::Normalizer},
};

const MAX_LINE_BYTES: usize = 64 * 1024;
const TRUNCATED_MARKER: &str = "... [truncated]";

/// Tails one log file and emits normalized entries for appended lines.
pub struct FileProducer {
    path: PathBuf,
    normalizer: Normalizer,
    cancel: CancellationToken,
}

impl FileProducer {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            normalizer: Normalizer::new(),
            cancel: CancellationToken::new(),
        }
    }
}

impl LogProducer for FileProducer {
    fn start(&self, tx: mpsc::Sender<ProducerEvent>) {
        let path = self.path.clone();
        let normalizer = self.normalizer.clone();
        let cancel = self.cancel.clone();

        tokio::spawn(async move {
            if let Err(err) = run_file_producer(path, normalizer, tx, cancel).await {
                warn!("file producer exited with error: {err}");
            }
        });
    }

    fn stop(&self) {
        self.cancel.cancel();
    }
}

async fn run_file_producer(
    path: PathBuf,
    normalizer: Normalizer,
    tx: mpsc::Sender<ProducerEvent>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let source = source_for_path(&path)?;
    let watch_path = PathBuf::from(&source.id);
    let parent = watch_path.parent().unwrap_or_else(|| Path::new("/"));

    if tx
        .send(ProducerEvent::SourceFound(source.clone()))
        .await
        .is_err()
    {
        debug!("file producer {} aborting: event channel closed", source.id);
        return Ok(());
    }

    let (notify_tx, mut notify_rx) = mpsc::channel(64);
    let mut watcher = make_watcher(notify_tx).map_err(std::io::Error::other)?;
    watcher
        .watch(parent, RecursiveMode::NonRecursive)
        .map_err(std::io::Error::other)?;

    let mut reader = FileReader::open_at_end(&watch_path).ok();

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            maybe_event = notify_rx.recv() => {
                let Some(event) = maybe_event else { break };
                match event {
                    Ok(event) => {
                        if !event_touches_path(&event, &watch_path) {
                            continue;
                        }

                        if is_remove_or_rename(&event.kind) {
                            reader = None;
                            continue;
                        }

                        if is_create_or_modify(&event.kind) {
                            if reader.as_mut().is_some_and(|reader| reader.was_truncated()) {
                                reader = None;
                            }
                            if reader.is_none() {
                                reader = FileReader::open_at_start(&watch_path).ok();
                            }
                            if let Some(reader) = &mut reader {
                                for line in reader.read_available_lines()? {
                                    let entry = normalizer.normalize(&line, source.clone());
                                    if tx.send(ProducerEvent::StoreEvent(entry)).await.is_err() {
                                        debug!("file producer {} aborting: event channel closed", source.id);
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                    Err(err) => warn!("file watcher error for {}: {err}", source.id),
                }
            }
        }
    }

    let _ = tx.send(ProducerEvent::SourceLost(source.id)).await;
    Ok(())
}

fn make_watcher(tx: mpsc::Sender<notify::Result<Event>>) -> notify::Result<RecommendedWatcher> {
    notify::recommended_watcher(move |event| {
        if tx.try_send(event).is_err() {
            warn!("file watcher event channel is full; dropping filesystem event");
        }
    })
}

fn is_create_or_modify(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Create(_) | EventKind::Modify(_))
}

fn is_remove_or_rename(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Remove(_))
        || matches!(kind, EventKind::Modify(notify::event::ModifyKind::Name(_)))
}

fn event_touches_path(event: &Event, target: &Path) -> bool {
    event.paths.iter().any(|path| path == target)
}

fn source_for_path(path: &Path) -> std::io::Result<Source> {
    let absolute = stable_absolute_path(path)?;
    let display_name = absolute
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| absolute.display().to_string());
    let group = absolute
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());

    Ok(Source {
        producer: "file".to_string(),
        id: absolute.display().to_string(),
        display_name,
        group,
    })
}

fn stable_absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if path.is_absolute() {
                Ok(path.to_path_buf())
            } else {
                Ok(std::env::current_dir()?.join(path))
            }
        }
        Err(err) => Err(err),
    }
}

pub(super) fn decode_line(bytes: &[u8]) -> String {
    let (bytes, truncated) = if bytes.len() > MAX_LINE_BYTES {
        (&bytes[..MAX_LINE_BYTES], true)
    } else {
        (bytes, false)
    };

    let mut line = String::from_utf8_lossy(bytes).into_owned();
    if truncated {
        line.push_str(TRUNCATED_MARKER);
    }
    line
}

#[derive(Default)]
pub(super) struct LineBuffer {
    bytes: Vec<u8>,
}

impl LineBuffer {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.bytes.extend_from_slice(bytes);
        let mut lines = Vec::new();

        while let Some(pos) = self.bytes.iter().position(|byte| *byte == b'\n') {
            let mut line = self.bytes.drain(..=pos).collect::<Vec<_>>();
            if line.ends_with(b"\n") {
                line.pop();
            }
            if line.ends_with(b"\r") {
                line.pop();
            }
            lines.push(decode_line(&line));
        }

        lines
    }
}

struct FileReader {
    file: std::fs::File,
    position: u64,
    buffer: LineBuffer,
}

impl FileReader {
    fn open_at_end(path: &Path) -> std::io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let position = file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            position,
            buffer: LineBuffer::default(),
        })
    }

    fn open_at_start(path: &Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        Ok(Self {
            file,
            position: 0,
            buffer: LineBuffer::default(),
        })
    }

    fn read_available_lines(&mut self) -> std::io::Result<Vec<String>> {
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        self.position += bytes.len() as u64;
        Ok(self.buffer.push(&bytes))
    }

    fn was_truncated(&self) -> bool {
        self.file
            .metadata()
            .map(|metadata| metadata.len() < self.position)
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, time::Duration};

    use tokio::sync::mpsc;

    use super::*;

    #[test]
    fn decode_line_uses_lossy_utf8() {
        assert_eq!(decode_line(b"hello \xF0\x90\x80world"), "hello �world");
    }

    #[test]
    fn decode_line_truncates_long_lines() {
        let line = decode_line(&vec![b'a'; 100 * 1024]);

        assert_eq!(line.len(), MAX_LINE_BYTES + TRUNCATED_MARKER.len());
        assert!(line.ends_with(TRUNCATED_MARKER));
    }

    #[test]
    fn line_buffer_flushes_complete_lines_and_keeps_partial() {
        let mut buffer = LineBuffer::default();

        assert!(buffer.push(b"first pa").is_empty());
        assert_eq!(
            buffer.push(b"rt\nsecond\r\nthird"),
            ["first part", "second"]
        );
        assert_eq!(buffer.push(b" line\n"), ["third line"]);
    }

    #[test]
    fn source_for_existing_path_uses_canonical_absolute_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        std::fs::File::create(&path).expect("create file");

        let source = source_for_path(&path).expect("source");

        assert_eq!(source.producer, "file");
        assert_eq!(
            source.id,
            path.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(source.display_name, "app.log");
        assert_eq!(
            source.group,
            dir.path()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn source_for_missing_relative_path_absolutizes_without_canonicalizing() {
        let path = PathBuf::from("missing-file-producer-test.log");

        let source = source_for_path(&path).expect("source");

        assert_eq!(
            source.id,
            std::env::current_dir()
                .unwrap()
                .join(path)
                .display()
                .to_string()
        );
        assert_eq!(source.display_name, "missing-file-producer-test.log");
    }

    #[tokio::test]
    async fn start_returns_promptly_for_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.log");
        let producer = FileProducer::new(path);
        let (tx, _rx) = mpsc::channel(8);

        let start = std::time::Instant::now();
        producer.start(tx);

        assert!(start.elapsed() < Duration::from_millis(50));
        producer.stop();
    }

    #[test]
    fn file_reader_reads_only_complete_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        let mut file = std::fs::File::create(&path).expect("create file");
        file.write_all(b"one\ntw").expect("write initial");
        drop(file);

        let mut reader = FileReader::open_at_start(&path).expect("reader");

        assert_eq!(reader.read_available_lines().unwrap(), ["one"]);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open append");
        file.write_all(b"o\n").expect("write rest");
        drop(file);

        assert_eq!(reader.read_available_lines().unwrap(), ["two"]);
    }
}
