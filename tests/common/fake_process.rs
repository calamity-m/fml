//! FakeProcessSpawner — returns an [`AsyncRead`] backed by a channel.
//!
//! Useful for simulating `kubectl logs -f` and `docker logs -f` output without
//! spawning real processes. Works with `tokio::time::pause()` for deterministic
//! timing tests.

use bytes::Bytes;
use futures::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

/// A handle for pushing log lines into a [`FakeProcess`] stream.
pub struct FakeProcessWriter {
    tx: mpsc::UnboundedSender<Bytes>,
}

impl FakeProcessWriter {
    /// Send a log line. Adds a trailing newline if not already present.
    pub fn send_line(&self, line: impl Into<String>) {
        let mut s = line.into();
        if !s.ends_with('\n') {
            s.push('\n');
        }
        let _ = self.tx.send(Bytes::from(s));
    }

    /// Send multiple lines at once (simulates a burst).
    pub fn send_burst(&self, lines: &[&str]) {
        for line in lines {
            self.send_line(*line);
        }
    }

    /// Close the stream, causing the consumer to see EOF.
    pub fn close(self) {
        // tx is dropped, causing the channel to close.
    }
}

/// A fake process output stream. Implements [`Stream<Item = Bytes>`] so it can
/// be used wherever an async byte stream is expected.
pub struct FakeProcess {
    rx: mpsc::UnboundedReceiver<Bytes>,
}

impl Stream for FakeProcess {
    type Item = Bytes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Create a linked writer/stream pair.
///
/// ```rust
/// let (writer, stream) = fake_process();
/// writer.send_line(r#"{"level":"INFO","msg":"hello"}"#);
/// writer.close();
/// ```
pub fn fake_process() -> (FakeProcessWriter, FakeProcess) {
    let (tx, rx) = mpsc::unbounded_channel();
    (FakeProcessWriter { tx }, FakeProcess { rx })
}

/// A rate-limited fake process that drips bytes at a configurable rate.
/// Use with `tokio::time::pause()` and `tokio::time::advance()` for
/// deterministic timing tests.
pub struct ThrottledFakeProcess {
    rx: mpsc::UnboundedReceiver<(Bytes, std::time::Duration)>,
}

/// Writer for a [`ThrottledFakeProcess`]. Each line is sent with an explicit
/// delay before it becomes readable.
pub struct ThrottledFakeProcessWriter {
    tx: mpsc::UnboundedSender<(Bytes, std::time::Duration)>,
}

impl ThrottledFakeProcessWriter {
    /// Queue a line to be readable after `delay`.
    pub fn send_after(&self, line: impl Into<String>, delay: std::time::Duration) {
        let mut s = line.into();
        if !s.ends_with('\n') {
            s.push('\n');
        }
        let _ = self.tx.send((Bytes::from(s), delay));
    }

    pub fn close(self) {}
}

impl Stream for ThrottledFakeProcess {
    type Item = Bytes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some((bytes, delay))) => {
                if !delay.is_zero() {
                    // Spawn a one-shot task to wake after the delay.
                    let waker = cx.waker().clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        waker.wake();
                    });
                    Poll::Pending
                } else {
                    Poll::Ready(Some(bytes))
                }
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub fn throttled_fake_process() -> (ThrottledFakeProcessWriter, ThrottledFakeProcess) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        ThrottledFakeProcessWriter { tx },
        ThrottledFakeProcess { rx },
    )
}
