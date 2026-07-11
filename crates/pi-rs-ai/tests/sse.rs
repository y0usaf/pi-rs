//! Behavioral parity tests for `transport::sse` against the spec's
//! `parseSSE` (`openai-codex-responses.ts`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use futures_util::stream;
use pi_rs_ai::transport::{AbortSignal, SseDecoder, SseEvent, SseReader, TransportError};

fn data_events(events: &[SseEvent]) -> Vec<&str> {
    events.iter().map(|e| e.data.as_str()).collect::<Vec<_>>()
}

#[test]
fn decodes_blocks_delimited_by_blank_lines() {
    let mut decoder = SseDecoder::new();
    let events = decoder.push(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
    assert_eq!(data_events(&events), vec!["{\"a\":1}", "{\"b\":2}"]);
}

#[test]
fn buffers_partial_blocks_across_chunks() {
    let mut decoder = SseDecoder::new();
    assert!(decoder.push(b"data: {\"a\"").is_empty());
    assert!(decoder.push(b":1}\n").is_empty());
    let events = decoder.push(b"\n");
    assert_eq!(data_events(&events), vec!["{\"a\":1}"]);
}

#[test]
fn survives_multibyte_utf8_split_across_chunks() {
    let mut decoder = SseDecoder::new();
    let payload = "data: héllo\n\n".as_bytes();
    // Split inside the two-byte 'é'.
    let split = payload.iter().position(|&b| b == 0xc3).unwrap() + 1;
    assert!(decoder.push(&payload[..split]).is_empty());
    let events = decoder.push(&payload[split..]);
    assert_eq!(data_events(&events), vec!["héllo"]);
}

#[test]
fn joins_multiple_data_lines_with_newline() {
    // Spec: dataLines.map(trim).join("\n").
    let mut decoder = SseDecoder::new();
    let events = decoder.push(b"data: line1\ndata: line2\n\n");
    assert_eq!(data_events(&events), vec!["line1\nline2"]);
}

#[test]
fn captures_event_field_and_trims_crlf() {
    let mut decoder = SseDecoder::new();
    let events = decoder.push(b"event: message_start\r\ndata: {\"x\":1}\r\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.as_deref(), Some("message_start"));
    assert_eq!(events[0].data, "{\"x\":1}");
}

#[test]
fn blocks_without_data_lines_are_dropped() {
    // Spec: only blocks with data lines are yielded (comments, retry:,
    // event:-only blocks fall through).
    let mut decoder = SseDecoder::new();
    let events = decoder.push(b": comment\n\nretry: 100\n\nevent: ping\n\ndata: kept\n\n");
    assert_eq!(data_events(&events), vec!["kept"]);
}

#[test]
fn done_sentinel_is_yielded_not_filtered() {
    // Filtering [DONE] / empty data is the protocol layer's, as in the
    // spec's caller.
    let mut decoder = SseDecoder::new();
    let events = decoder.push(b"data: [DONE]\n\n");
    assert_eq!(data_events(&events), vec!["[DONE]"]);
}

#[tokio::test]
async fn reader_yields_events_and_drops_trailing_partial() {
    let chunks: Vec<Result<&[u8], std::convert::Infallible>> = vec![
        Ok(b"data: one\n\nda"),
        Ok(b"ta: two\n\ndata: trailing-partial"),
    ];
    let mut reader = SseReader::new(stream::iter(chunks), None);
    assert_eq!(reader.next().await.unwrap().unwrap().data, "one");
    assert_eq!(reader.next().await.unwrap().unwrap().data, "two");
    // Spec: buffer remainder without a closing \n\n is dropped.
    assert!(reader.next().await.unwrap().is_none());
    assert!(reader.next().await.unwrap().is_none());
}

#[tokio::test]
async fn reader_aborts_between_chunks() {
    let signal = AbortSignal::new();
    let chunks: Vec<Result<&[u8], std::convert::Infallible>> =
        vec![Ok(b"data: one\n\n"), Ok(b"data: two\n\n")];
    let mut reader = SseReader::new(stream::iter(chunks), Some(signal.clone()));
    assert_eq!(reader.next().await.unwrap().unwrap().data, "one");
    signal.abort();
    let error = reader.next().await.unwrap_err();
    assert!(matches!(error, TransportError::Aborted));
    assert_eq!(error.to_string(), "Request was aborted");
}
