//! Retry policy — port of the spec's retry helpers
//! (`openai-codex-responses.ts`, "Retry Helpers" section).
//!
//! The spec's regexes are alternations of literals with at most one `.?`
//! gap; they are hand-rolled here ([`contains_loose`]) because building a
//! regex can fail and the code standard denies `expect` in library
//! crates. Behavior is pinned by tests, including the JS quirk that `.`
//! does not match `\n`.

use std::time::{Duration, SystemTime};

use reqwest::header::HeaderMap;

use super::TransportError;
use super::abort::AbortSignal;

/// Spec: `DEFAULT_MAX_RETRIES = 0`.
pub const DEFAULT_MAX_RETRIES: u32 = 0;
/// Spec: `BASE_DELAY_MS = 1000`.
pub const BASE_DELAY_MS: f64 = 1_000.0;
/// Spec: `DEFAULT_MAX_RETRY_DELAY_MS = 60_000`.
pub const DEFAULT_MAX_RETRY_DELAY_MS: f64 = 60_000.0;
/// Spec: `DEFAULT_SSE_HEADER_TIMEOUT_MS = 10_000`.
pub const DEFAULT_SSE_HEADER_TIMEOUT_MS: u64 = 10_000;

/// Spec: `isTerminalRateLimitError` — quota/billing failures that must
/// never be retried.
pub fn is_terminal_rate_limit_error(error_text: &str) -> bool {
    let text = error_text.to_lowercase();
    [
        "gousagelimiterror",
        "freeusagelimiterror",
        "monthly usage limit reached",
        "available balance",
        "insufficient_quota",
        "out of budget",
        "quota exceeded",
        "billing",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

/// Spec: `isRetryableError`.
pub fn is_retryable_error(status: u16, error_text: &str) -> bool {
    if status == 429 && is_terminal_rate_limit_error(error_text) {
        return false;
    }
    if matches!(status, 429 | 500 | 502 | 503 | 504) {
        return true;
    }
    let text = error_text.to_lowercase();
    contains_loose(&text, "rate", "limit")
        || text.contains("overloaded")
        || contains_loose(&text, "service", "unavailable")
        || contains_loose(&text, "upstream", "connect")
        || contains_loose(&text, "connection", "refused")
}

/// `a.?b` in the spec's regex: `a`, then zero or one character (JS `.`
/// does not match `\n`), then `b`. Case is the caller's (text is
/// pre-lowercased, the spec's `i` flag).
fn contains_loose(text: &str, a: &str, b: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = text[start..].find(a) {
        let after = start + pos + a.len();
        if text[after..].starts_with(b) {
            return true;
        }
        if let Some(gap) = text[after..].chars().next()
            && gap != '\n'
            && text[after + gap.len_utf8()..].starts_with(b)
        {
            return true;
        }
        // None of the spec's literals self-overlap; skip the whole match.
        start = after;
    }
    false
}

/// JS `parseFloat(header)`: longest numeric prefix (optional sign,
/// decimal, exponent); `None` for NaN.
fn js_parse_float(value: &str) -> Option<f64> {
    let text = value.trim_start();
    let bytes = text.as_bytes();
    let mut end = 0;
    let mut seen_digit = false;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        seen_digit = true;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            seen_digit = true;
        }
    }
    if !seen_digit {
        return None;
    }
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut exp_end = end + 1;
        if exp_end < bytes.len() && (bytes[exp_end] == b'+' || bytes[exp_end] == b'-') {
            exp_end += 1;
        }
        let digits_start = exp_end;
        while exp_end < bytes.len() && bytes[exp_end].is_ascii_digit() {
            exp_end += 1;
        }
        if exp_end > digits_start {
            end = exp_end;
        }
    }
    text[..end].parse::<f64>().ok()
}

/// SDK spec (`@anthropic-ai/sdk` 0.91.1 `client.shouldRetry`): an
/// explicit `x-should-retry` header wins; otherwise request timeouts
/// (408), lock timeouts (409), rate limits (429), and internal errors
/// (>=500) retry.
pub fn sdk_should_retry(status: u16, headers: &HeaderMap) -> bool {
    match headers
        .get("x-should-retry")
        .and_then(|value| value.to_str().ok())
    {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    matches!(status, 408 | 409 | 429) || status >= 500
}

/// SDK spec (`client.retryRequest` + `calculateDefaultRetryTimeoutMillis`):
/// `retry-after-ms` (millis), then `retry-after` (seconds, else an HTTP
/// date, else a zero sleep), then exponential backoff
/// `min(0.5 * 2^numRetries, 8)` seconds with up-to-25% jitter.
pub fn sdk_retry_delay_ms(
    headers: Option<&HeaderMap>,
    retries_remaining: u32,
    max_retries: u32,
) -> f64 {
    let mut timeout_ms: Option<f64> = None;
    if let Some(headers) = headers {
        if let Some(value) = headers.get("retry-after-ms").and_then(|v| v.to_str().ok())
            && let Some(millis) = js_parse_float(value)
        {
            timeout_ms = Some(millis);
        }
        // JS `!timeoutMillis`: unset or zero falls through.
        if timeout_ms.unwrap_or(0.0) == 0.0
            && let Some(value) = headers.get("retry-after").and_then(|v| v.to_str().ok())
        {
            if let Some(seconds) = js_parse_float(value) {
                timeout_ms = Some(seconds * 1000.0);
            } else {
                // `Date.parse(retryAfter) - Date.now()`; a NaN result is
                // not `undefined`, so it still skips the default backoff
                // (and sleeps zero).
                timeout_ms = Some(match httpdate::parse_http_date(value.trim()) {
                    Ok(date) => date
                        .duration_since(SystemTime::now())
                        .map(|d| d.as_secs_f64() * 1000.0)
                        .unwrap_or(0.0),
                    Err(_) => 0.0,
                });
            }
        }
    }
    if let Some(delay) = timeout_ms {
        return delay.max(0.0);
    }
    let num_retries = max_retries.saturating_sub(retries_remaining);
    let sleep_seconds = (0.5 * f64::powi(2.0, num_retries as i32)).min(8.0);
    let jitter = 1.0 - sdk_jitter_random() * 0.25;
    sleep_seconds * jitter * 1000.0
}

/// `Math.random()` stand-in for the SDK's retry jitter; timing-only, so
/// a subsecond-clock source is sufficient.
fn sdk_jitter_random() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    f64::from(nanos % 1_000_000) / 1_000_000.0
}

/// JS `Number(header)` approximation for the values that occur in
/// `retry-after` headers: trimmed-empty is `0`, decimal/exponent floats
/// parse, everything else is NaN (`None`).
fn js_number(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    trimmed.parse::<f64>().ok()
}

/// Spec: `getRetryAfterDelayMs` — `retry-after-ms` (millis), then
/// `retry-after` as seconds, then as an HTTP date.
pub fn retry_after_delay_ms(headers: &HeaderMap) -> Option<f64> {
    if let Some(value) = headers.get("retry-after-ms").and_then(|v| v.to_str().ok())
        && let Some(millis) = js_number(value)
        && millis.is_finite()
    {
        return Some(millis.max(0.0));
    }

    let retry_after = headers.get("retry-after")?.to_str().ok()?;
    if let Some(seconds) = js_number(retry_after)
        && seconds.is_finite()
    {
        return Some((seconds * 1000.0).max(0.0));
    }

    // Spec: `Date.parse(retryAfter)` — HTTP-date forms.
    if let Ok(date) = httpdate::parse_http_date(retry_after.trim()) {
        let delta = date
            .duration_since(SystemTime::now())
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        return Some(delta);
    }

    None
}

/// Spec: `capRetryDelayMs` — cap server-requested delays; `0` disables
/// the cap.
pub fn cap_retry_delay_ms(delay_ms: f64, max_retry_delay_ms: Option<f64>) -> f64 {
    let max = max_retry_delay_ms.unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS);
    if max > 0.0 {
        delay_ms.min(max)
    } else {
        delay_ms
    }
}

/// Spec: `BASE_DELAY_MS * 2 ** attempt`.
pub fn backoff_delay_ms(attempt: u32) -> f64 {
    BASE_DELAY_MS * f64::powi(2.0, attempt as i32)
}

/// Spec: abortable `sleep(ms, signal)` — rejects with the abort error if
/// the signal fires first (or already fired).
pub async fn sleep_ms(ms: f64, signal: Option<&AbortSignal>) -> Result<(), TransportError> {
    let duration = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
    match signal {
        Some(signal) => {
            if signal.is_aborted() {
                return Err(TransportError::Aborted);
            }
            tokio::select! {
                _ = signal.aborted() => Err(TransportError::Aborted),
                _ = tokio::time::sleep(duration) => Ok(()),
            }
        }
        None => {
            tokio::time::sleep(duration).await;
            Ok(())
        }
    }
}
