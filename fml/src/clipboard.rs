use std::io::Write;

use base64::Engine as _;

use crate::error::FmlError;

/// Maximum base64-encoded payload size before warning the user. xterm and
/// older VTE cap OSC52 payloads at ~8 KB; exceeding this silently drops the
/// yank in those terminals.
pub const OSC52_WARN_BYTES: usize = 8 * 1024;

/// Write an OSC 52 clipboard sequence to `out` and return the base64-encoded
/// byte length of the payload.
///
/// OSC 52 is a best-effort sequence: there is no in-band reply confirming that
/// the terminal actually wrote the clipboard. Delivery depends on terminal
/// support and multiplexer configuration (see README for details).
///
/// The 8 KB threshold mentioned in the plan is the responsibility of callers —
/// this function encodes and writes unconditionally.
pub fn yank_osc52<W: Write>(out: &mut W, payload: &str) -> Result<usize, FmlError> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
    let len = encoded.len();
    write!(out, "\x1b]52;c;{encoded}\x1b\\")?;
    out.flush()?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn encodes_hello_as_exact_osc52_sequence() {
        let mut buf = Cursor::new(Vec::new());
        let n = yank_osc52(&mut buf, "hello").unwrap();
        let result = String::from_utf8(buf.into_inner()).unwrap();
        assert_eq!(result, "\x1b]52;c;aGVsbG8=\x1b\\");
        assert_eq!(n, 8);
    }

    #[test]
    fn empty_string_writes_osc52_with_empty_payload() {
        let mut buf = Cursor::new(Vec::new());
        let n = yank_osc52(&mut buf, "").unwrap();
        let result = String::from_utf8(buf.into_inner()).unwrap();
        assert_eq!(result, "\x1b]52;c;\x1b\\");
        assert_eq!(n, 0);
    }

    #[test]
    fn writer_error_returns_fml_error() {
        struct FailWriter;
        impl Write for FailWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let result = yank_osc52(&mut FailWriter, "hello");
        assert!(matches!(result, Err(FmlError::Io(_))));
    }
}
