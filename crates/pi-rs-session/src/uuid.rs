//! Generic UUID helpers shared by host mechanisms.

use std::sync::Mutex;

struct State {
    last_timestamp: Option<u64>,
    sequence: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    last_timestamp: None,
    sequence: 0,
});

fn fill_random_bytes(bytes: &mut [u8]) {
    if getrandom::fill(bytes).is_ok() {
        return;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64 ^ duration.as_secs())
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    let mut value = seed | 1;
    for byte in bytes.iter_mut() {
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        *byte = (value & 0xff) as u8;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Time-ordered UUIDv7 suitable for generic storage identifiers.
pub fn uuidv7() -> String {
    let mut random = [0u8; 16];
    fill_random_bytes(&mut random);
    let timestamp = now_ms();

    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
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
    let timestamp = state.last_timestamp.unwrap_or(timestamp);
    let sequence = state.sequence;
    drop(state);

    let bytes = [
        ((timestamp >> 40) & 0xff) as u8,
        ((timestamp >> 32) & 0xff) as u8,
        ((timestamp >> 24) & 0xff) as u8,
        ((timestamp >> 16) & 0xff) as u8,
        ((timestamp >> 8) & 0xff) as u8,
        (timestamp & 0xff) as u8,
        0x70 | ((sequence >> 28) & 0x0f) as u8,
        ((sequence >> 20) & 0xff) as u8,
        0x80 | ((sequence >> 14) & 0x3f) as u8,
        ((sequence >> 6) & 0xff) as u8,
        (((sequence & 0x3f) << 2) as u8) | (random[10] & 0x03),
        random[11],
        random[12],
        random[13],
        random[14],
        random[15],
    ];
    format_uuid(&bytes)
}

/// Random UUIDv4 suitable for source-neutral host resource IDs.
pub fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    fill_random_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format_uuid(&bytes)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    format!(
        "{0:02x}{1:02x}{2:02x}{3:02x}-{4:02x}{5:02x}-{6:02x}{7:02x}-{8:02x}{9:02x}-{10:02x}{11:02x}{12:02x}{13:02x}{14:02x}{15:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{random_uuid, uuidv7};

    #[test]
    fn generated_ids_have_the_requested_uuid_versions() {
        let ordered_a = uuidv7();
        let ordered_b = uuidv7();
        let random = random_uuid();
        assert!(ordered_a < ordered_b);
        assert_eq!(ordered_a.as_bytes()[14], b'7');
        assert_eq!(random.as_bytes()[14], b'4');
    }
}
