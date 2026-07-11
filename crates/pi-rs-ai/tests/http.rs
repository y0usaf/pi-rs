//! Behavioral parity tests for `transport::http` against the spec's
//! fetch-with-retry loop (`openai-codex-responses.ts`), run against a
//! local raw-TCP HTTP server (sandbox-safe: loopback only).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;

use pi_rs_ai::transport::{
    AbortSignal, RetryOptions, TransportError, post_with_retry, response_sse_reader,
};
use reqwest::header::HeaderMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn http_response(status: u16, reason: &str, extra_headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} {reason}\r\nConnection: close\r\n{extra_headers}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Serve one canned response per connection, then keep the listener alive.
/// `hang_after` leaves subsequent connections open without responding.
fn serve(responses: Vec<String>, hang_after: bool) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    tokio::spawn(async move {
        let mut responses = responses.into_iter();
        let mut held = Vec::new();
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => return,
            };
            read_request(&mut sock).await;
            match responses.next() {
                Some(response) => {
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.shutdown().await;
                }
                None if hang_after => held.push(sock), // never respond
                None => return,
            }
        }
    });
    addr
}

/// Read one HTTP request: headers plus a content-length body.
async fn read_request(sock: &mut tokio::net::TcpStream) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let content_length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while buf.len() - (pos + 4) < content_length {
                let n = match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
            }
            return;
        }
    }
}

fn url(addr: SocketAddr) -> String {
    format!("http://{addr}/v1/responses")
}

#[tokio::test]
async fn success_streams_sse_end_to_end() {
    let body = "data: {\"type\":\"ping\"}\n\nevent: done\ndata: [DONE]\n\n";
    let addr = serve(
        vec![http_response(
            200,
            "OK",
            "Content-Type: text/event-stream\r\n",
            body,
        )],
        false,
    );
    let client = reqwest::Client::new();
    let response = post_with_retry(
        &client,
        &url(addr),
        &HeaderMap::new(),
        "{}",
        &RetryOptions::default(),
        None,
    )
    .await
    .unwrap();

    let mut reader = response_sse_reader(response, None);
    let first = reader.next().await.unwrap().unwrap();
    assert_eq!(first.data, "{\"type\":\"ping\"}");
    let second = reader.next().await.unwrap().unwrap();
    assert_eq!(second.event.as_deref(), Some("done"));
    assert_eq!(second.data, "[DONE]");
    assert!(reader.next().await.unwrap().is_none());
}

#[tokio::test]
async fn retries_429_honoring_retry_after_ms() {
    let addr = serve(
        vec![
            http_response(
                429,
                "Too Many Requests",
                "retry-after-ms: 5\r\n",
                "slow down",
            ),
            http_response(200, "OK", "", "ok"),
        ],
        false,
    );
    let client = reqwest::Client::new();
    let retry = RetryOptions {
        max_retries: 1,
        ..RetryOptions::default()
    };
    let response = post_with_retry(&client, &url(addr), &HeaderMap::new(), "{}", &retry, None)
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test]
async fn default_zero_retries_surfaces_status_error() {
    // Spec: DEFAULT_MAX_RETRIES = 0 — a retryable 500 still fails
    // immediately by default.
    let addr = serve(
        vec![http_response(500, "Internal Server Error", "", "boom")],
        false,
    );
    let client = reqwest::Client::new();
    let error = post_with_retry(
        &client,
        &url(addr),
        &HeaderMap::new(),
        "{}",
        &RetryOptions::default(),
        None,
    )
    .await
    .unwrap_err();
    match error {
        TransportError::Status { status, body, .. } => {
            assert_eq!(status, 500);
            assert_eq!(body, "boom");
        }
        other => panic!("expected Status error, got {other}"),
    }
}

#[tokio::test]
async fn non_retryable_status_still_retries_via_generic_catch() {
    // Spec nuance: a 400 throws a friendly error *inside the try*, which
    // the catch retries on backoff while attempts remain.
    let addr = serve(
        vec![
            http_response(400, "Bad Request", "", "bad payload"),
            http_response(200, "OK", "", "ok"),
        ],
        false,
    );
    let client = reqwest::Client::new();
    let retry = RetryOptions {
        max_retries: 1,
        ..RetryOptions::default()
    };
    let response = post_with_retry(&client, &url(addr), &HeaderMap::new(), "{}", &retry, None)
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test]
async fn usage_limit_errors_are_never_retried() {
    let addr = serve(
        vec![http_response(
            429,
            "Too Many Requests",
            "",
            "Monthly usage limit reached",
        )],
        false,
    );
    let client = reqwest::Client::new();
    let retry = RetryOptions {
        max_retries: 3,
        ..RetryOptions::default()
    };
    let started = std::time::Instant::now();
    let error = post_with_retry(&client, &url(addr), &HeaderMap::new(), "{}", &retry, None)
        .await
        .unwrap_err();
    assert!(matches!(error, TransportError::Status { status: 429, .. }));
    // No backoff sleeps happened.
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
}

#[tokio::test]
async fn pre_aborted_signal_short_circuits() {
    let signal = AbortSignal::new();
    signal.abort();
    let client = reqwest::Client::new();
    let error = post_with_retry(
        &client,
        "http://127.0.0.1:9/unreachable",
        &HeaderMap::new(),
        "{}",
        &RetryOptions::default(),
        Some(&signal),
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Request was aborted");
}

#[tokio::test]
async fn header_timeout_fires_when_server_stalls() {
    let addr = serve(vec![], true); // accept, read, never respond
    let client = reqwest::Client::new();
    let retry = RetryOptions {
        header_timeout_ms: 50,
        ..RetryOptions::default()
    };
    let error = post_with_retry(&client, &url(addr), &HeaderMap::new(), "{}", &retry, None)
        .await
        .unwrap_err();
    match error {
        TransportError::HeaderTimeout(ms) => {
            assert_eq!(ms, 50);
        }
        other => panic!("expected HeaderTimeout, got {other}"),
    }
    assert_eq!(
        TransportError::HeaderTimeout(50).to_string(),
        "SSE response headers timed out after 50ms"
    );
}
