//! RFC 8628 device-code polling shared by Codex and GitHub Copilot.
//!
//! Spec: `utils/oauth/device-code.ts`. Polling happens immediately, then
//! sleeps between incomplete responses. `slow_down` permanently adds five
//! seconds to the interval.

use std::future::Future;
use std::time::Duration;

use crate::error::AuthError;
use crate::types::OAuthLoginCallbacks;

const CANCEL_MESSAGE: &str = "Login cancelled";
const TIMEOUT_MESSAGE: &str = "Device flow timed out";
const SLOW_DOWN_TIMEOUT_MESSAGE: &str = "Device flow timed out after one or more slow_down responses. This is often caused by clock drift in WSL or VM environments. Please sync or restart the VM clock and try again.";
const MINIMUM_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);

/// One result from a device-token endpoint.
#[derive(Clone, Debug, PartialEq)]
pub enum DeviceCodePoll<T> {
    Pending,
    SlowDown,
    Failed(String),
    Complete(T),
}

/// Spec: `pollOAuthDeviceCodeFlow`.
pub async fn poll_device_code<T, F, Fut>(
    interval_seconds: Option<f64>,
    expires_in_seconds: Option<f64>,
    callbacks: &dyn OAuthLoginCallbacks,
    mut poll: F,
) -> Result<T, AuthError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<DeviceCodePoll<T>, AuthError>>,
{
    let interval = interval_seconds
        .map(|seconds| Duration::from_millis(((seconds * 1000.0).floor().max(1000.0)) as u64))
        .unwrap_or(DEFAULT_INTERVAL)
        .max(MINIMUM_INTERVAL);
    let mut interval = interval;
    let deadline = expires_in_seconds.map(|seconds| {
        tokio::time::Instant::now() + Duration::from_millis(((seconds * 1000.0).max(0.0)) as u64)
    });
    let mut slow_down_responses = 0_u64;

    loop {
        if callbacks.is_cancelled() {
            return Err(AuthError::Message(CANCEL_MESSAGE.into()));
        }
        if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
            break;
        }

        match poll().await? {
            DeviceCodePoll::Complete(value) => return Ok(value),
            DeviceCodePoll::Failed(message) => return Err(AuthError::Message(message)),
            DeviceCodePoll::Pending => {}
            DeviceCodePoll::SlowDown => {
                slow_down_responses += 1;
                interval = interval
                    .saturating_add(SLOW_DOWN_INCREMENT)
                    .max(MINIMUM_INTERVAL);
            }
        }

        let sleep_for = deadline
            .map(|deadline| {
                deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .min(interval)
            })
            .unwrap_or(interval);
        if sleep_for.is_zero() {
            break;
        }
        tokio::select! {
            () = tokio::time::sleep(sleep_for) => {}
            result = callbacks.on_cancelled() => {
                result?;
                return Err(AuthError::Message(CANCEL_MESSAGE.into()));
            }
        }
    }

    Err(AuthError::Message(if slow_down_responses > 0 {
        SLOW_DOWN_TIMEOUT_MESSAGE.into()
    } else {
        TIMEOUT_MESSAGE.into()
    }))
}
