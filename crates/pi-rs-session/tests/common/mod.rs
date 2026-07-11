//! Shared helpers, ported from pi's `test/utilities.ts` (`userMsg`,
//! `assistantMsg`) plus the uuid/filename shape checks its tests inline.
#![allow(dead_code, clippy::unwrap_used)]

use serde_json::{Value, json};

pub fn user_msg(text: &str) -> Value {
    json!({ "role": "user", "content": text, "timestamp": pi_rs_session::time::now_ms() })
}

pub fn assistant_msg(text: &str) -> Value {
    json!({
        "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "api": "anthropic-messages",
        "provider": "anthropic",
        "model": "test",
        "usage": {
            "input": 1,
            "output": 1,
            "cacheRead": 0,
            "cacheWrite": 0,
            "totalTokens": 2,
            "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 },
        },
        "stopReason": "stop",
        "timestamp": pi_rs_session::time::now_ms(),
    })
}

/// pi's `UUID_V7_RE`: `^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`.
pub fn is_uuid_v7(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups.iter().map(|g| g.len()).collect::<Vec<_>>() == vec![8, 4, 4, 4, 12]
        && groups.iter().all(|g| {
            g.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        && groups[2].starts_with('7')
        && matches!(groups[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b')
}

/// pi's session filename check:
/// `^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z_{id}\.jsonl$`.
pub fn is_session_file_name(name: &str, id: &str) -> bool {
    let Some(rest) = name.strip_suffix(".jsonl") else {
        return false;
    };
    let Some(timestamp) = rest.strip_suffix(id).and_then(|t| t.strip_suffix('_')) else {
        return false;
    };
    let bytes = timestamp.as_bytes();
    bytes.len() == 24
        && bytes[24 - 1] == b'Z'
        && bytes[10] == b'T'
        && [4, 7, 13, 16, 19].iter().all(|&i| bytes[i] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19 | 23) || b.is_ascii_digit())
}

pub fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
