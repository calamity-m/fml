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
    config::IngestConfig,
    event::ProducerEvent,
    log::Source,
    producer::{LogProducer, normalizer::Normalizer},
};

const MAX_LINE_BYTES: usize = 64 * 1024;
const TRUNCATED_MARKER: &str = "... [truncated]";
/// Chunk size for the backwards startup-backfill scan.
const BACKFILL_SCAN_CHUNK: usize = 64 * 1024;

/// Tails one log file and emits normalized entries for appended lines.
///
/// When startup backfill is enabled and the file exists at startup, up to
/// `backfill_max_lines_per_source` complete lines before the captured EOF are
/// emitted oldest-to-newest before live following begins. The configured time
/// window is not applied to raw files: per-line timestamps are not
/// trustworthy before normalization, so only the line cap bounds file
/// backfill.
pub struct FileProducer {
    path: PathBuf,
    normalizer: Normalizer,
    cancel: CancellationToken,
    ingest: IngestConfig,
}

impl FileProducer {
    pub fn new(path: PathBuf, ingest: IngestConfig) -> Self {
        Self {
            path,
            normalizer: Normalizer::new(),
            cancel: CancellationToken::new(),
            ingest,
        }
    }
}

impl LogProducer for FileProducer {
    fn start(&self, tx: mpsc::Sender<ProducerEvent>) {
        let path = self.path.clone();
        let normalizer = self.normalizer;
        let cancel = self.cancel.clone();
        let ingest = self.ingest;

        tokio::spawn(async move {
            if let Err(err) = run_file_producer(path, normalizer, tx, cancel, ingest).await {
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
    ingest: IngestConfig,
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

    // The watcher must be in place before the backfill offset is captured so
    // appends racing the backfill still raise a notify event for the follow
    // reader (which starts at the captured offset, before those appends).
    let (notify_tx, mut notify_rx) = mpsc::channel(64);
    let mut watcher = make_watcher(notify_tx).map_err(std::io::Error::other)?;
    watcher
        .watch(parent, RecursiveMode::NonRecursive)
        .map_err(std::io::Error::other)?;

    let mut reader = if ingest.backfill_enabled() {
        match startup_backfill(&watch_path, ingest.backfill_max_lines_per_source) {
            // Missing file: keep current behavior and start from the
            // beginning when it is later created.
            Ok(None) => None,
            Ok(Some(backfill)) => {
                for line in backfill.lines {
                    let entry = normalizer.normalize(&line, source.clone());
                    if tx.send(ProducerEvent::StoreEvent(entry)).await.is_err() {
                        debug!("file producer {} aborting: event channel closed", source.id);
                        return Ok(());
                    }
                }
                FileReader::open_at(&watch_path, backfill.follow_pos).ok()
            }
            Err(err) => {
                // Backfill failure is non-fatal: log it and fall back to
                // live-only tailing from the current EOF.
                warn!("file backfill failed for {}: {err}", source.id);
                FileReader::open_at_end(&watch_path).ok()
            }
        }
    } else {
        FileReader::open_at_end(&watch_path).ok()
    };

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

/// Startup backfill for one existing file: the bounded set of complete lines
/// to emit, plus the offset live following must start from.
struct FileBackfill {
    lines: Vec<String>,
    /// Offset just after the newline ending the newest complete line. Bytes
    /// at or beyond this offset (a trailing partial line, plus any appends
    /// racing the backfill) belong to the follow reader, so the handoff
    /// neither loses nor duplicates lines.
    follow_pos: u64,
}

/// Capture the file length and tail up to `max_lines` complete lines before
/// it. Returns `Ok(None)` when the file is missing at startup.
fn startup_backfill(path: &Path, max_lines: usize) -> std::io::Result<Option<FileBackfill>> {
    let eof = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    tail_complete_lines_before(path, eof, max_lines).map(Some)
}

/// Read up to the last `max_lines` complete lines whose terminating newline
/// is before `eof`, oldest-to-newest, without loading unbounded files into
/// memory.
///
/// The newline scan walks backwards in fixed-size chunks and gives up after
/// `(max_lines + 1) * MAX_LINE_BYTES` bytes: lines longer than
/// `MAX_LINE_BYTES` are truncated on emission anyway, so scanning deeper can
/// only find bytes that would be discarded. When the scan stops early, the
/// oldest partially-scanned line is dropped entirely so only complete lines
/// are ever emitted.
fn tail_complete_lines_before(
    path: &Path,
    eof: u64,
    max_lines: usize,
) -> std::io::Result<FileBackfill> {
    use std::io::{Read as _, Seek as _};

    let mut file = std::fs::File::open(path)?;
    let scan_floor = eof.saturating_sub((max_lines as u64 + 1) * MAX_LINE_BYTES as u64);

    // Backwards scan: newline offsets, newest first, at most `max_lines + 1`.
    // The extra newline (when present) marks the end of the line *before* the
    // oldest emitted line, i.e. the read start boundary.
    let mut newlines: Vec<u64> = Vec::new();
    let mut chunk = vec![0u8; BACKFILL_SCAN_CHUNK];
    let mut pos = eof;
    'scan: while pos > scan_floor {
        let chunk_start = pos
            .saturating_sub(BACKFILL_SCAN_CHUNK as u64)
            .max(scan_floor);
        let len = (pos - chunk_start) as usize;
        file.seek(SeekFrom::Start(chunk_start))?;
        file.read_exact(&mut chunk[..len])?;
        for offset in (0..len).rev() {
            if chunk[offset] == b'\n' {
                newlines.push(chunk_start + offset as u64);
                if newlines.len() > max_lines {
                    break 'scan;
                }
            }
        }
        pos = chunk_start;
    }

    let Some(&last_newline) = newlines.first() else {
        // No complete line before `eof`. Follow from the start when the whole
        // (partial) file was scanned; otherwise skip the oversized partial
        // line entirely rather than re-reading it unbounded.
        let follow_pos = if scan_floor == 0 { 0 } else { eof };
        return Ok(FileBackfill {
            lines: Vec::new(),
            follow_pos,
        });
    };
    let follow_pos = last_newline + 1;

    // Start after the oldest scanned newline unless the scan reached the
    // beginning of the file with lines to spare.
    let hit_cap = newlines.len() > max_lines;
    let oldest_newline = *newlines.last().expect("newlines is non-empty");
    let start = if !hit_cap && scan_floor == 0 {
        0
    } else {
        oldest_newline + 1
    };
    if start == follow_pos {
        // Only one newline was found and its line start is unknown: nothing
        // complete to emit.
        return Ok(FileBackfill {
            lines: Vec::new(),
            follow_pos,
        });
    }

    // Forward read of [start, follow_pos): every byte run ending in '\n'
    // here is a complete line, so LineBuffer flushes them all.
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = follow_pos - start;
    let mut buffer = LineBuffer::default();
    let mut lines = Vec::new();
    while remaining > 0 {
        let len = remaining.min(BACKFILL_SCAN_CHUNK as u64) as usize;
        file.read_exact(&mut chunk[..len])?;
        remaining -= len as u64;
        lines.extend(buffer.push(&chunk[..len]));
    }

    Ok(FileBackfill { lines, follow_pos })
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
        Self::open_at(path, 0)
    }

    fn open_at(path: &Path, position: u64) -> std::io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let position = file.seek(SeekFrom::Start(position))?;
        Ok(Self {
            file,
            position,
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
        let producer = FileProducer::new(path, IngestConfig::default());
        let (tx, _rx) = mpsc::channel(8);

        let start = std::time::Instant::now();
        producer.start(tx);

        assert!(start.elapsed() < Duration::from_millis(50));
        producer.stop();
    }

    fn write_file(dir: &tempfile::TempDir, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).expect("write file");
        path
    }

    #[test]
    fn startup_backfill_missing_file_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");

        let backfill = startup_backfill(&dir.path().join("missing.log"), 100).expect("backfill");

        assert!(backfill.is_none());
    }

    #[test]
    fn tail_returns_all_lines_when_fewer_than_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "app.log", b"one\ntwo\nthree\n");
        let eof = std::fs::metadata(&path).unwrap().len();

        let backfill = tail_complete_lines_before(&path, eof, 100).expect("tail");

        assert_eq!(backfill.lines, ["one", "two", "three"]);
        assert_eq!(backfill.follow_pos, eof);
    }

    #[test]
    fn tail_caps_at_max_lines_keeping_newest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "app.log", b"one\ntwo\nthree\nfour\n");
        let eof = std::fs::metadata(&path).unwrap().len();

        let backfill = tail_complete_lines_before(&path, eof, 2).expect("tail");

        assert_eq!(backfill.lines, ["three", "four"]);
        assert_eq!(backfill.follow_pos, eof);
    }

    #[test]
    fn tail_excludes_trailing_partial_line_and_follows_from_its_start() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "app.log", b"one\ntwo\npart");
        let eof = std::fs::metadata(&path).unwrap().len();

        let backfill = tail_complete_lines_before(&path, eof, 100).expect("tail");

        assert_eq!(backfill.lines, ["one", "two"]);
        // Follow position is the start of "part" so the partial line is
        // emitted whole once its newline arrives.
        assert_eq!(backfill.follow_pos, 8);
    }

    #[test]
    fn tail_with_no_newlines_emits_nothing_and_follows_from_start() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "app.log", b"partial without newline");
        let eof = std::fs::metadata(&path).unwrap().len();

        let backfill = tail_complete_lines_before(&path, eof, 100).expect("tail");

        assert!(backfill.lines.is_empty());
        assert_eq!(backfill.follow_pos, 0);
    }

    #[test]
    fn tail_strips_carriage_returns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "app.log", b"one\r\ntwo\r\n");
        let eof = std::fs::metadata(&path).unwrap().len();

        let backfill = tail_complete_lines_before(&path, eof, 100).expect("tail");

        assert_eq!(backfill.lines, ["one", "two"]);
    }

    #[test]
    fn tail_bounded_read_spans_many_scan_chunks() {
        // ~1.3MB file (20 chunks of the 64KiB scan size) exercises the
        // chunked backwards scan and the chunked forward read.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut contents = Vec::new();
        for i in 0..20_000 {
            contents.extend_from_slice(format!("line-{i:05} {}\n", "x".repeat(50)).as_bytes());
        }
        let path = write_file(&dir, "app.log", &contents);
        let eof = std::fs::metadata(&path).unwrap().len();

        let backfill = tail_complete_lines_before(&path, eof, 5000).expect("tail");

        assert_eq!(backfill.lines.len(), 5000);
        assert!(backfill.lines[0].starts_with("line-15000 "));
        assert!(backfill.lines[4999].starts_with("line-19999 "));
        assert_eq!(backfill.follow_pos, eof);
    }

    #[test]
    fn tail_ignores_appends_after_captured_eof_and_follow_reader_picks_them_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "app.log", b"old-1\nold-2\npart");
        let eof = std::fs::metadata(&path).unwrap().len();

        // Append racing the backfill: arrives after the EOF capture.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open append");
        file.write_all(b"ial\nnew-1\n").expect("append");
        drop(file);

        let backfill = tail_complete_lines_before(&path, eof, 100).expect("tail");
        assert_eq!(backfill.lines, ["old-1", "old-2"]);

        // The follow reader opens at the captured handoff offset and emits
        // exactly the completed partial line plus the raced append.
        let mut reader = FileReader::open_at(&path, backfill.follow_pos).expect("reader");
        assert_eq!(reader.read_available_lines().unwrap(), ["partial", "new-1"]);
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
