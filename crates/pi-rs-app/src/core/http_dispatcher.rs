//! Port of `core/http-dispatcher.ts` — the HTTP idle-timeout setting
//! vocabulary.
//!
//! WS3.1 subset (recorded): the pure parts only. The spec's
//! `configureHttpDispatcher` installs a process-global undici dispatcher;
//! pi-rs's transport (`pi-rs-ai`) owns request timeouts, and wiring this
//! setting into it lands with its consumer (WS4 session wiring).

// The parse vocabulary lives with the settings port in pi-rs-host (the
// `pi.settings` consumer); re-exported here to keep the spec shape.
pub use pi_rs_host::settings_manager::{DEFAULT_HTTP_IDLE_TIMEOUT_MS, parse_http_idle_timeout_ms};

/// Spec: `HTTP_IDLE_TIMEOUT_CHOICES` — `(label, timeoutMs)`.
pub const HTTP_IDLE_TIMEOUT_CHOICES: &[(&str, u64)] = &[
    ("30 sec", 30_000),
    ("1 min", 60_000),
    ("2 min", 120_000),
    ("5 min", 300_000),
    ("disabled", 0),
];

/// Spec: `formatHttpIdleTimeoutMs(timeoutMs)`.
pub fn format_http_idle_timeout_ms(timeout_ms: u64) -> String {
    if let Some((label, _)) = HTTP_IDLE_TIMEOUT_CHOICES
        .iter()
        .find(|(_, ms)| *ms == timeout_ms)
    {
        return (*label).to_owned();
    }
    // Spec: `${timeoutMs / 1000} sec` — JS number formatting.
    let seconds = timeout_ms as f64 / 1000.0;
    if seconds.fract() == 0.0 {
        format!("{} sec", seconds as u64)
    } else {
        format!("{seconds} sec")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_spec_shapes() {
        assert_eq!(parse_http_idle_timeout_ms(&json!("disabled")), Some(0));
        assert_eq!(parse_http_idle_timeout_ms(&json!(" DISABLED ")), Some(0));
        assert_eq!(parse_http_idle_timeout_ms(&json!("")), None);
        assert_eq!(parse_http_idle_timeout_ms(&json!("2000")), Some(2000));
        assert_eq!(parse_http_idle_timeout_ms(&json!("1e3")), Some(1000));
        assert_eq!(parse_http_idle_timeout_ms(&json!(1500.9)), Some(1500));
        assert_eq!(parse_http_idle_timeout_ms(&json!(0)), Some(0));
        assert_eq!(parse_http_idle_timeout_ms(&json!(-1)), None);
        assert_eq!(parse_http_idle_timeout_ms(&json!("nope")), None);
        assert_eq!(parse_http_idle_timeout_ms(&json!(true)), None);
        assert_eq!(parse_http_idle_timeout_ms(&json!(null)), None);
    }

    #[test]
    fn formats_choices_and_fallback() {
        assert_eq!(format_http_idle_timeout_ms(60_000), "1 min");
        assert_eq!(format_http_idle_timeout_ms(0), "disabled");
        assert_eq!(format_http_idle_timeout_ms(45_000), "45 sec");
        assert_eq!(format_http_idle_timeout_ms(45_500), "45.5 sec");
    }
}
