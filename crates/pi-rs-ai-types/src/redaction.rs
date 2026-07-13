//! Shared secret redaction for provider transport and authentication errors.
//!
//! Redaction is intentionally independent of any provider. Known secret values
//! are removed verbatim, while JSON response bodies are scrubbed by field name.

use serde_json::Value;

pub const REDACTED: &str = "[REDACTED]";

/// Redact known secret values and credential-shaped JSON fields.
///
/// Inputs without a match are returned byte-for-byte. This preserves provider
/// error compatibility while ensuring echoed credentials do not escape through
/// errors, diagnostics, or debug snapshots.
#[must_use]
pub fn redact_sensitive(input: &str, secrets: &[&str]) -> String {
    let mut output = input.to_owned();
    let mut changed = false;
    for secret in secrets {
        if secret.len() >= 4 && output.contains(secret) {
            output = output.replace(secret, REDACTED);
            changed = true;
        }
    }

    changed |= redact_embedded_json_strings(&mut output);
    if let Ok(mut value) = serde_json::from_str::<Value>(&output)
        && redact_json_value(&mut value)
    {
        return serde_json::to_string(&value).unwrap_or_else(|_| REDACTED.to_owned());
    }

    if changed { output } else { input.to_owned() }
}

fn redact_embedded_json_strings(input: &mut String) -> bool {
    const KEYS: &[&str] = &[
        "access_token",
        "refresh_token",
        "id_token",
        "api_key",
        "api-key",
        "authorization",
        "client_secret",
        "secret_access_key",
        "session_token",
        "code_verifier",
        "device_code",
        "token",
    ];
    let mut changed = false;
    for key in KEYS {
        let needle = format!("\"{key}\"");
        let mut search_from = 0;
        while let Some(relative) = input[search_from..].find(&needle) {
            let key_end = search_from + relative + needle.len();
            let Some(colon_relative) = input[key_end..].find(':') else {
                break;
            };
            let value_start = key_end + colon_relative + 1;
            let whitespace = input[value_start..]
                .bytes()
                .take_while(u8::is_ascii_whitespace)
                .count();
            let quote = value_start + whitespace;
            if input.as_bytes().get(quote) != Some(&b'"') {
                search_from = key_end;
                continue;
            }
            let content_start = quote + 1;
            let Some(content_end) = closing_json_quote(input, content_start) else {
                break;
            };
            input.replace_range(content_start..content_end, REDACTED);
            changed = true;
            search_from = content_start + REDACTED.len() + 1;
        }
    }
    changed
}

fn closing_json_quote(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut index = start;
    let mut escaped = false;
    while let Some(byte) = bytes.get(index) {
        if *byte == b'"' && !escaped {
            return Some(index);
        }
        escaped = *byte == b'\\' && !escaped;
        if *byte != b'\\' {
            escaped = false;
        }
        index += 1;
    }
    None
}

fn redact_json_value(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            for (key, value) in map {
                if is_sensitive_key(key) {
                    if !value.is_null() {
                        *value = Value::String(REDACTED.to_owned());
                        changed = true;
                    }
                } else {
                    changed |= redact_json_value(value);
                }
            }
            changed
        }
        Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= redact_json_value(value);
            }
            changed
        }
        _ => false,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "apikey"
            | "authorization"
            | "clientsecret"
            | "secretaccesskey"
            | "sessiontoken"
            | "codeverifier"
            | "devicecode"
    ) || normalized == "token"
}

#[cfg(test)]
mod tests {
    use super::{REDACTED, redact_sensitive};

    #[test]
    fn preserves_safe_text_and_scrubs_known_and_json_secrets() {
        assert_eq!(redact_sensitive("safe error", &[]), "safe error");
        assert_eq!(
            redact_sensitive("echo super-secret-value", &["super-secret-value"]),
            format!("echo {REDACTED}")
        );
        assert_eq!(
            redact_sensitive(
                r#"{"error":"bad","access_token":"token-value","nested":{"api-key":"key-value"}}"#,
                &[]
            ),
            r#"{"error":"bad","access_token":"[REDACTED]","nested":{"api-key":"[REDACTED]"}}"#
        );
        assert_eq!(
            redact_sensitive(
                r#"provider failed: {"refresh_token": "embedded-secret"}"#,
                &[]
            ),
            r#"provider failed: {"refresh_token": "[REDACTED]"}"#
        );
    }
}
