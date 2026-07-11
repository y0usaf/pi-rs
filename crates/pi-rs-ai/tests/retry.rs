//! Behavioral parity tests for `transport::retry` against the spec's
//! retry helpers (`openai-codex-responses.ts`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::transport::retry::{
    backoff_delay_ms, cap_retry_delay_ms, is_retryable_error, is_terminal_rate_limit_error,
    retry_after_delay_ms, sleep_ms,
};
use pi_rs_ai::transport::{AbortSignal, TransportError};
use reqwest::header::{HeaderMap, HeaderValue};

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(*name, HeaderValue::from_str(value).unwrap());
    }
    map
}

#[test]
fn terminal_rate_limit_patterns() {
    for text in [
        "GoUsageLimitError",
        "FreeUsageLimitError",
        "Monthly usage limit reached",
        "your available balance is too low",
        "insufficient_quota",
        "out of budget",
        "Quota exceeded for this project",
        "BILLING hard limit",
    ] {
        assert!(is_terminal_rate_limit_error(text), "{text}");
    }
    assert!(!is_terminal_rate_limit_error("rate limit exceeded"));
}

#[test]
fn retryable_statuses_and_terminal_429() {
    for status in [429, 500, 502, 503, 504] {
        assert!(is_retryable_error(status, ""), "{status}");
    }
    // 429 with a terminal quota message is not retryable.
    assert!(!is_retryable_error(429, "Monthly usage limit reached"));
    // …but the same text on a 5xx still is (spec order: terminal check is
    // 429-only).
    assert!(is_retryable_error(500, "Monthly usage limit reached"));
    assert!(!is_retryable_error(400, "bad request"));
}

#[test]
fn retryable_text_patterns_with_loose_gap() {
    // Spec regex: /rate.?limit|overloaded|service.?unavailable|…/i —
    // `.?` is zero or one char, `.` does not match \n.
    for text in [
        "rate limit",
        "ratelimit",
        "Rate-Limit reached",
        "OVERLOADED",
        "service unavailable",
        "ServiceUnavailable",
        "upstream connect error",
        "connection refused",
        "Connection_Refused",
    ] {
        assert!(is_retryable_error(400, text), "{text}");
    }
    // Two-char gap does not match; newline gap does not match (JS `.`).
    assert!(!is_retryable_error(400, "rate  limit"));
    assert!(!is_retryable_error(400, "rate\nlimit"));
}

#[test]
fn retry_after_ms_header_wins() {
    let map = headers(&[("retry-after-ms", "1500"), ("retry-after", "60")]);
    assert_eq!(retry_after_delay_ms(&map), Some(1500.0));
    // Negative clamps to zero (spec: Math.max(0, millis)).
    let map = headers(&[("retry-after-ms", "-5")]);
    assert_eq!(retry_after_delay_ms(&map), Some(0.0));
    // Non-numeric retry-after-ms falls through to retry-after.
    let map = headers(&[("retry-after-ms", "soon"), ("retry-after", "2")]);
    assert_eq!(retry_after_delay_ms(&map), Some(2000.0));
}

#[test]
fn retry_after_seconds_and_http_date() {
    let map = headers(&[("retry-after", "1.5")]);
    assert_eq!(retry_after_delay_ms(&map), Some(1500.0));

    // HTTP-date in the past clamps to 0 (spec: Math.max(0, date - now)).
    let map = headers(&[("retry-after", "Wed, 21 Oct 2015 07:28:00 GMT")]);
    assert_eq!(retry_after_delay_ms(&map), Some(0.0));

    // Future HTTP-date yields a positive delay.
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(30);
    let map = headers(&[("retry-after", &httpdate::fmt_http_date(future))]);
    let delay = retry_after_delay_ms(&map).unwrap();
    assert!(delay > 25_000.0 && delay <= 30_000.0, "{delay}");

    // Unparseable value → None.
    let map = headers(&[("retry-after", "soon")]);
    assert_eq!(retry_after_delay_ms(&map), None);

    // No headers at all → None.
    assert_eq!(retry_after_delay_ms(&HeaderMap::new()), None);
}

#[test]
fn cap_and_backoff() {
    // Default cap is 60s.
    assert_eq!(cap_retry_delay_ms(90_000.0, None), 60_000.0);
    assert_eq!(cap_retry_delay_ms(5_000.0, None), 5_000.0);
    // Explicit cap.
    assert_eq!(cap_retry_delay_ms(5_000.0, Some(1_000.0)), 1_000.0);
    // Zero disables the cap (spec: maxRetryDelayMs > 0 ? min : delay).
    assert_eq!(cap_retry_delay_ms(90_000.0, Some(0.0)), 90_000.0);

    assert_eq!(backoff_delay_ms(0), 1_000.0);
    assert_eq!(backoff_delay_ms(1), 2_000.0);
    assert_eq!(backoff_delay_ms(3), 8_000.0);
}

#[tokio::test]
async fn sleep_rejects_on_abort() {
    // Pre-aborted signal rejects immediately.
    let signal = AbortSignal::new();
    signal.abort();
    let error = sleep_ms(10_000.0, Some(&signal)).await.unwrap_err();
    assert!(matches!(error, TransportError::Aborted));

    // Abort mid-sleep wakes the sleeper.
    let signal = AbortSignal::new();
    let sleeper = {
        let signal = signal.clone();
        tokio::spawn(async move { sleep_ms(60_000.0, Some(&signal)).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    signal.abort();
    assert!(matches!(
        sleeper.await.unwrap(),
        Err(TransportError::Aborted)
    ));
}
