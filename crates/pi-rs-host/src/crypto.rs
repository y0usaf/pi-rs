//! Reviewed hashes and crypto mechanisms for Lua-authored policy.
//!
//! Node translations (`node:crypto#createHash`, `node:crypto#randomUUID`,
//! `Bun.hash.xxHash32`, `Buffer` base64) become explicit `pi.crypto` and
//! `pi.buffer` bindings. Which data to hash, which algorithm, and how the
//! result is presented stay in Lua; this module only performs the primitive.
//!
//! All hashes are output as lowercase hex strings (Node `createHash
//! .update(d).digest("hex")`). `xxhash32` matches `Bun.hash.xxHash32`:
//! a `u32` (returned as a Lua integer). `random_uuid` matches
//! `crypto.randomUUID()`: a version-4 UUID string.

use md5::{Digest as _, Md5};
use sha1::Sha1;
use sha2::{Digest as _, Sha256};

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub(crate) fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

pub(crate) fn md5_hex(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

/// `Bun.hash.xxHash32` — a u32 over the data with the given seed.
pub(crate) fn xxhash32(data: &[u8], seed: u32) -> u32 {
    twox_hash::XxHash32::oneshot(seed, data)
}

/// An xxHash-based 64-bit filesystem/stream fingerprint (used by the
/// Hashline dogfood for change detection; xxHash matches Bun's murmur/xor
/// family semantics for a fast content digest).
pub(crate) fn xxhash64(data: &[u8], seed: u64) -> u64 {
    twox_hash::XxHash64::oneshot(seed, data)
}

/// Version-4 random UUID string (`crypto.randomUUID()`).
pub(crate) fn random_uuid() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Fall back to a time-seeded degenerate value so the binding never
        // panics; callers needing cryptographic uniqueness run on platforms
        // where getrandom succeeds.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        bytes[..8].copy_from_slice(&now.to_le_bytes());
    }
    // Set version 4 and the RFC 4122 variant bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Install `pi.crypto` and `pi.buffer` on the API table.
pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let crypto = lua.create_table()?;
    let hash = |alg: &'static str| {
        lua.create_function(move |_, data: mlua::String| -> mlua::Result<String> {
            let bytes = data.as_bytes();
            let out = match alg {
                "sha1" => sha1_hex(&bytes),
                "sha256" => sha256_hex(&bytes),
                "md5" => md5_hex(&bytes),
                _ => unreachable!(),
            };
            Ok(out)
        })
    };
    crypto.set("sha1", hash("sha1")?)?;
    crypto.set("sha256", hash("sha256")?)?;
    crypto.set("md5", hash("md5")?)?;
    crypto.set(
        "xxhash32",
        lua.create_function(|_, (data, seed): (mlua::String, Option<u32>)| {
            let bytes = data.as_bytes();
            Ok(xxhash32(&bytes, seed.unwrap_or(0)))
        })?,
    )?;
    crypto.set(
        "xxhash64",
        lua.create_function(|_, (data, seed): (mlua::String, Option<u64>)| {
            let bytes = data.as_bytes();
            Ok(xxhash64(&bytes, seed.unwrap_or(0)))
        })?,
    )?;
    crypto.set(
        "random_uuid",
        lua.create_function(|_, ()| Ok(random_uuid()))?,
    )?;
    pi.set("crypto", crypto)?;

    // Buffer translations: base64 encode/decode and byte length. `Buffer
    // .concat`/`Buffer.from(array)` are expressed array-to-string/string
    // joins in Lua; these two are the irreducible encodings.
    let buffer = lua.create_table()?;
    buffer.set(
        "base64_encode",
        lua.create_function(|_, data: mlua::String| {
            use base64::Engine as _;
            Ok(base64::engine::general_purpose::STANDARD.encode(data.as_bytes()))
        })?,
    )?;
    buffer.set(
        "base64_decode",
        lua.create_function(|lua, data: String| {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| mlua::Error::runtime(format!("invalid base64: {e}")))?;
            lua.create_string(&bytes)
        })?,
    )?;
    buffer.set(
        "byte_length",
        lua.create_function(|_, data: mlua::String| Ok(data.as_bytes().len()))?,
    )?;
    pi.set("buffer", buffer)?;
    Ok(())
}
