use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::time::sleep;

pub(crate) const MAX_LOG_LINE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_HELPER_OUTPUT_LINES: usize = 1024;

/// Reads one line from `reader`, capping allocation at `max_bytes` so a
/// newline-free stream cannot grow the heap unboundedly.
pub(crate) async fn read_bounded_line<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max_bytes: usize,
) -> std::io::Result<Option<String>>
where
    R: AsyncBufReadExt + Unpin,
{
    buf.clear();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(Some(String::from_utf8_lossy(buf).into_owned()));
        }

        let remaining = max_bytes.saturating_sub(buf.len());
        if let Some(nl) = chunk.iter().take(remaining).position(|&b| b == b'\n') {
            buf.extend_from_slice(&chunk[..nl]);
            reader.consume(nl + 1);
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(Some(String::from_utf8_lossy(buf).into_owned()));
        }

        let take = chunk.len().min(remaining);
        buf.extend_from_slice(&chunk[..take]);
        reader.consume(take);

        if buf.len() >= max_bytes {
            return Ok(Some(String::from_utf8_lossy(buf).into_owned()));
        }
    }
}

/// Spawns a child process, retrying on `ETXTBSY` which can occur on overlayfs
/// (e.g. Docker containers) when a binary is written and immediately executed.
///
/// `build` is invoked once per attempt so a fresh `Command` (including any
/// `pre_exec` hooks) is constructed each time.
pub(crate) async fn spawn_with_etxtbsy_retry<F>(mut build: F) -> std::io::Result<Child>
where
    F: FnMut() -> Command,
{
    const MAX_RETRIES: u32 = 5;
    for attempt in 0..MAX_RETRIES {
        match build().spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                if attempt == MAX_RETRIES - 1 {
                    return Err(e);
                }
                sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn read_bounded_line_caps_newline_free_stream() {
        let payload = vec![b'a'; MAX_LOG_LINE_BYTES * 2 + 123];
        let mut reader = BufReader::new(std::io::Cursor::new(payload));
        let mut buf = Vec::new();
        let mut chunks = Vec::new();

        while let Some(line) = read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES)
            .await
            .unwrap()
        {
            assert!(line.len() <= MAX_LOG_LINE_BYTES);
            chunks.push(line.len());
        }

        assert_eq!(chunks, vec![MAX_LOG_LINE_BYTES, MAX_LOG_LINE_BYTES, 123]);
    }

    #[tokio::test]
    async fn read_bounded_line_strips_crlf() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"one\r\ntwo\nthree".as_slice()));
        let mut buf = Vec::new();
        assert_eq!(
            read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES)
                .await
                .unwrap()
                .as_deref(),
            Some("one")
        );
        assert_eq!(
            read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES)
                .await
                .unwrap()
                .as_deref(),
            Some("two")
        );
        assert_eq!(
            read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES)
                .await
                .unwrap()
                .as_deref(),
            Some("three")
        );
        assert_eq!(
            read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES)
                .await
                .unwrap(),
            None
        );
    }
}
