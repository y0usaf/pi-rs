//! Port of the spec's uuid helpers.
//!
//! - [`uuidv7`] ← `packages/agent/src/harness/session/uuid.ts`, including
//!   the module-level monotonic `lastTimestamp`/`sequence` state (same-ms
//!   calls increment the sequence; sequence overflow bumps the timestamp).
//! - [`random_uuid`] ← Node's `crypto.randomUUID()` (v4), used by
//!   session-manager's short-id generator (`randomUUID().slice(0, 8)`).

use std::sync::Mutex;

struct State {
    /// Spec: `let lastTimestamp = -Infinity` — `None` until the first call.
    last_timestamp: Option<u64>,
    sequence: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    last_timestamp: None,
    sequence: 0,
});

/// Spec: `fillRandomBytes` — crypto randomness. The spec's `Math.random`
/// fallback is unreachable here (`getrandom` is the platform RNG); on the
/// impossible failure path we fall back to a time-derived xor pattern so
/// the function stays infallible like the spec's.
fn fill_random_bytes(bytes: &mut [u8]) {
    if getrandom::fill(bytes).is_ok() {
        return;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    let mut x = seed | 1;
    for b in bytes.iter_mut() {
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xff) as u8;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Spec: `uuidv7()` — 48-bit ms timestamp, version 7, monotonic sequence
/// in the rand_a/rand_b high bits, crypto randomness in the tail.
pub fn uuidv7() -> String {
    let mut random = [0u8; 16];
    fill_random_bytes(&mut random);
    let timestamp = now_ms();

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.last_timestamp.is_none_or(|last| timestamp > last) {
        state.sequence = u32::from(random[6]) * 0x0100_0000
            + u32::from(random[7]) * 0x0001_0000
            + u32::from(random[8]) * 0x0000_0100
            + u32::from(random[9]);
        state.last_timestamp = Some(timestamp);
    } else {
        state.sequence = state.sequence.wrapping_add(1);
        if state.sequence == 0 {
            state.last_timestamp = state.last_timestamp.map(|last| last + 1);
        }
    }
    let ts = state.last_timestamp.unwrap_or(timestamp);
    let seq = state.sequence;
    drop(state);

    let bytes: [u8; 16] = [
        ((ts >> 40) & 0xff) as u8,
        ((ts >> 32) & 0xff) as u8,
        ((ts >> 24) & 0xff) as u8,
        ((ts >> 16) & 0xff) as u8,
        ((ts >> 8) & 0xff) as u8,
        (ts & 0xff) as u8,
        0x70 | ((seq >> 28) & 0x0f) as u8,
        ((seq >> 20) & 0xff) as u8,
        0x80 | ((seq >> 14) & 0x3f) as u8,
        ((seq >> 6) & 0xff) as u8,
        (((seq & 0x3f) << 2) as u8) | (random[10] & 0x03),
        random[11],
        random[12],
        random[13],
        random[14],
        random[15],
    ];
    format_uuid(&bytes)
}

/// Node `crypto.randomUUID()` — a random v4 UUID.
pub fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    fill_random_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format_uuid(&bytes)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex[0..4].join(""),
        hex[4..6].join(""),
        hex[6..8].join(""),
        hex[8..10].join(""),
        hex[10..16].join("")
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn uuidv7_shape_and_monotonicity() {
        let re_check = |id: &str| {
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(parts.len(), 5);
            assert_eq!(
                parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                vec![8, 4, 4, 4, 12]
            );
            assert!(parts[2].starts_with('7'));
            assert!(matches!(
                parts[3].chars().next().unwrap(),
                '8' | '9' | 'a' | 'b'
            ));
        };
        let a = uuidv7();
        let b = uuidv7();
        re_check(&a);
        re_check(&b);
        // Time-ordered: lexicographic order is creation order.
        assert!(a < b, "{a} < {b}");
    }

    #[test]
    fn random_uuid_is_v4() {
        let id = random_uuid();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[2].starts_with('4'));
    }
}
