//! SSE decoding — port of the spec's `parseSSE`
//! (`openai-codex-responses.ts`), the only first-party SSE parser in pi.
//!
//! Semantics preserved: events are blocks delimited by `\n\n`; `data:`
//! lines are stripped of the prefix, trimmed, and joined with `\n`;
//! blocks without data lines are dropped; a trailing partial block at
//! stream end is dropped. Generalization (still protocol-free): the
//! SSE-standard `event:` field is captured, since anthropic-messages
//! streams are `event:`-tagged — the codex parser ignores it, protocols
//! that don't need it ignore it here too. `[DONE]` sentinels and empty
//! data are *yielded*, not filtered: that filtering is the protocol
//! layer's, exactly where the spec does it.

use std::collections::VecDeque;

use futures_util::{Stream, StreamExt};

use super::TransportError;
use super::abort::AbortSignal;

/// One decoded SSE event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SseEvent {
    /// SSE `event:` field of the block, if present.
    pub event: Option<String>,
    /// `data:` lines, each trimmed, joined with `\n`, then trimmed
    /// (spec: `dataLines.join("\n").trim()`).
    pub data: String,
}

/// Incremental push decoder. Byte-buffered so multi-byte UTF-8 sequences
/// split across chunks survive (the spec's `TextDecoder` with
/// `stream: true`); the `\n\n` delimiter search is byte-safe.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk, get every event completed by it.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(idx) = self.buffer.windows(2).position(|window| window == b"\n\n") {
            let rest = self.buffer.split_off(idx + 2);
            let block = std::mem::replace(&mut self.buffer, rest);
            let text = String::from_utf8_lossy(&block[..idx]);
            if let Some(event) = parse_block(&text) {
                events.push(event);
            }
        }
        events
    }
}

/// Spec: the per-chunk block parse — `data:` lines sliced and trimmed.
/// Returns `None` when the block has no data lines (comments,
/// `event:`-only blocks), which the spec skips.
fn parse_block(text: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim());
        } else if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    let data = data_lines.join("\n").trim().to_string();
    Some(SseEvent { event, data })
}

/// Pull-style reader over a byte stream — the async half of `parseSSE`:
/// abort is checked before and after every read, and aborting while
/// blocked on the transport wakes immediately.
pub struct SseReader<S> {
    stream: S,
    decoder: SseDecoder,
    pending: VecDeque<SseEvent>,
    signal: Option<AbortSignal>,
    done: bool,
}

impl<S, B, E> SseReader<S>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    pub fn new(stream: S, signal: Option<AbortSignal>) -> Self {
        Self {
            stream,
            decoder: SseDecoder::new(),
            pending: VecDeque::new(),
            signal,
            done: false,
        }
    }

    /// Next event, `Ok(None)` at end of stream.
    pub async fn next(&mut self) -> Result<Option<SseEvent>, TransportError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            if self.done {
                return Ok(None);
            }
            let signal = self.signal.clone();
            if let Some(signal) = &signal
                && signal.is_aborted()
            {
                return Err(TransportError::Aborted);
            }
            let chunk = match &signal {
                // Abort mid-read: pi's pending `reader.read()` rejects
                // with undici's abort `DOMException`, not the loop-top
                // "Request was aborted" check.
                Some(signal) => tokio::select! {
                    _ = signal.aborted() => return Err(TransportError::BodyAborted),
                    chunk = self.stream.next() => chunk,
                },
                None => self.stream.next().await,
            };
            if let Some(signal) = &signal
                && signal.is_aborted()
            {
                return Err(TransportError::Aborted);
            }
            match chunk {
                // Trailing partial block dropped, per spec.
                None => self.done = true,
                Some(Ok(bytes)) => self.pending.extend(self.decoder.push(bytes.as_ref())),
                Some(Err(error)) => {
                    self.done = true;
                    return Err(TransportError::Network(error.to_string()));
                }
            }
        }
    }
}
