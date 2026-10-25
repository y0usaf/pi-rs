//! ai/src/auth/oauth/device-code.ts: the RFC 8628 polling loop the device-code flows share.

use std::time::Duration;

use pi_rt::cancel::CancelToken;

use crate::oauth::now_ms;

pub const CANCELLED: &str = "Login cancelled";
pub const TIMED_OUT: &str = "Device flow timed out";
pub const SLOW_DOWN_TIMED_OUT: &str = "Device flow timed out after one or more slow_down responses. This is often caused by clock drift in WSL or VM environments. Please sync or restart the VM clock and try again.";
const MINIMUM_INTERVAL_MS: i64 = 1000;
const DEFAULT_POLL_INTERVAL_SECONDS: f64 = 5.0;
const SLOW_DOWN_INTERVAL_INCREMENT_MS: i64 = 5000;

/// One poll's answer. SlowDown carries the interval the server sent, when it sent one.
pub enum Poll<T> {
    Pending,
    SlowDown(Option<f64>),
    Failed(String),
    Complete(T),
}

/// What the flow knows before it polls.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub interval_seconds: Option<f64>,
    pub expires_in_seconds: Option<i64>,
    pub wait_before_first_poll: bool,
}

impl Options {
    pub fn new(interval_seconds: Option<f64>, expires_in_seconds: Option<i64>) -> Self {
        Self {
            interval_seconds,
            expires_in_seconds,
            wait_before_first_poll: false,
        }
    }

    pub fn waiting_before_first_poll(mut self) -> Self {
        self.wait_before_first_poll = true;
        self
    }
}

/// pollOAuthDeviceCodeFlow: polls until the flow completes, the user cancels, the flow expires or
/// the server refuses. slow_down raises the interval the way RFC 8628 section 3.5 says, or to the
/// interval the server sent.
pub fn poll<T>(
    options: &Options,
    cancel: &CancelToken,
    mut step: impl FnMut() -> Result<Poll<T>, String>,
) -> Result<T, String> {
    let deadline = options
        .expires_in_seconds
        .map(|seconds| now_ms() + seconds * 1000);
    let mut interval = interval_ms(
        options
            .interval_seconds
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS),
    );
    let mut slow_downs = 0;
    if options.wait_before_first_poll {
        let remaining = deadline.map_or(i64::MAX, |deadline| deadline - now_ms());
        if remaining > 0 && sleep(interval.min(remaining), cancel) {
            return Err(CANCELLED.to_owned());
        }
    }
    while deadline.is_none_or(|deadline| now_ms() < deadline) {
        if cancel.is_cancelled() {
            return Err(CANCELLED.to_owned());
        }
        match step()? {
            Poll::Complete(value) => return Ok(value),
            Poll::Failed(message) => return Err(message),
            Poll::SlowDown(server) => {
                slow_downs += 1;
                interval = match server {
                    Some(seconds) if seconds.is_finite() && seconds > 0.0 => interval_ms(seconds),
                    _ => (interval + SLOW_DOWN_INTERVAL_INCREMENT_MS).max(MINIMUM_INTERVAL_MS),
                };
            }
            Poll::Pending => {}
        }
        let remaining = deadline.map(|deadline| deadline - now_ms());
        if remaining.is_some_and(|remaining| remaining <= 0) {
            break;
        }
        let wait = remaining.map_or(interval, |remaining| interval.min(remaining));
        if sleep(wait, cancel) {
            return Err(CANCELLED.to_owned());
        }
    }
    Err(if slow_downs > 0 {
        SLOW_DOWN_TIMED_OUT
    } else {
        TIMED_OUT
    }
    .to_owned())
}

/// Math.max(MINIMUM_INTERVAL_MS, Math.floor(seconds * 1000)).
fn interval_ms(seconds: f64) -> i64 {
    ((seconds * 1000.0).floor() as i64).max(MINIMUM_INTERVAL_MS)
}

/// Waits, and answers whether the user cancelled meanwhile.
fn sleep(millis: i64, cancel: &CancelToken) -> bool {
    cancel.wait_timeout(Duration::from_millis(millis.max(0) as u64))
}
