//! pi.crypto + pi.buffer — reviewed hashes/crypto and binary-data
//! mechanisms (PLAN 9.9 system_needs group 2: crypto.*, Buffer.*).
//!
//! Hashes use the reviewed Rust crates sha2 (SHA-256) and twox-hash
//! (XXH32, Bun.hash.xxHash32 with seed 0). random_uuid is a standard
//! v4 UUID from getrandom(2) — no PRNG state on the Lua surface.
//!
//! Lua surface (pi.crypto):
//! - sha256(data) -> lowercase hex string
//! - random_uuid() -> v4 UUID string
//! - xxhash32(data) -> uint32 (number)
//! - create_hash("sha256") -> { update(data), digest(encoding?) }
//!   encoding: "hex" (default) | "base64" | "raw" (binary string)
//!
//! Lua surface (pi.buffer):
//! - alloc(n) -> n zero bytes as a Lua string
//! - from(value) -> bytes as a Lua string (string | array of bytes)
//! - byte_length(s) -> #s
//! - concat(list) -> concatenated bytes
//! - from_hex(s) -> binary string from lowercase hex


use mlua::{Lua, Table};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// RFC 4122 v4 UUID from getrandom bytes.
pub(crate) fn random_uuid() -> mlua::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(mlua::Error::external)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10xx
    let mut out = String::with_capacity(36);
    for (index, b) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push_str(&format!("{b:02x}"));
    }
    Ok(out)
}

/// Streaming hash handle: update(data), then digest(encoding) consumes.
struct LuaHash {
    inner: Option<Sha256>,
}

impl mlua::UserData for LuaHash {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("update", |_, this, data: mlua::String| {
            let Some(hasher) = this.inner.as_mut() else {
                return Err(mlua::Error::runtime("hash already digested"));
            };
            hasher.update(data.as_bytes());
            Ok(())
        });
        methods.add_method_mut("digest", |lua, this, encoding: Option<String>| {
            let hasher = this.inner.take().ok_or_else(|| {
                mlua::Error::runtime("hash already digested")
            })?;
            let bytes = hasher.finalize();
            match encoding.as_deref().unwrap_or("hex") {
                "hex" => Ok(mlua::Value::String(lua.create_string(hex(&bytes))?)),
                "base64" => Ok(mlua::Value::String(lua.create_string(base64(&bytes))?)),
                "raw" => Ok(mlua::Value::String(lua.create_string(bytes)?)),
                other => Err(mlua::Error::runtime(format!(
                    "create_hash digest: unknown encoding {other}"
                ))),
            }
        });
    }
}

fn install_crypto(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let crypto = lua.create_table()?;
    crypto.set(
        "sha256",
        lua.create_function(|lua, data: mlua::String| {
            let digest = Sha256::digest(data.as_bytes());
            lua.create_string(hex(&digest))
        })?,
    )?;
    crypto.set(
        "random_uuid",
        lua.create_function(|_, ()| random_uuid())?,
    )?;
    crypto.set(
        "xxhash32",
        lua.create_function(|_, data: mlua::String| {
            use std::hash::Hasher as _;
            let mut hasher = twox_hash::XxHash32::with_seed(0);
            hasher.write(data.as_bytes().as_ref());
            Ok(hasher.finish() as u32)
        })?,
    )?;
    crypto.set(
        "create_hash",
        lua.create_function(|lua, algorithm: String| {
            match algorithm.as_str() {
                "sha256" => Ok(mlua::Value::UserData(
                    lua.create_userdata(LuaHash { inner: Some(Sha256::new()) })?,
                )),
                other => Err(mlua::Error::runtime(format!(
                    "create_hash: unsupported algorithm {other}"
                ))),
            }
        })?,
    )?;
    pi.set("crypto", crypto)?;
    Ok(())
}

fn install_buffer(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let buffer = lua.create_table()?;
    buffer.set(
        "alloc",
        lua.create_function(|lua, size: usize| {
            let bytes = vec![0u8; size];
            lua.create_string(&bytes)
        })?,
    )?;
    buffer.set(
        "from",
        lua.create_function(|lua, value: mlua::Value| {
            let bytes: Vec<u8> = match value {
                mlua::Value::String(s) => s.as_bytes().to_vec(),
                mlua::Value::Table(t) => {
                    let mut out = Vec::new();
                    for byte in t.sequence_values::<u8>() {
                        out.push(byte?);
                    }
                    out
                }
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "Buffer.from: unsupported value {other:?}"
                    )));
                }
            };
            lua.create_string(&bytes)
        })?,
    )?;
    buffer.set(
        "byte_length",
        lua.create_function(|_, value: mlua::String| Ok(value.as_bytes().len()))?,
    )?;
    buffer.set(
        "concat",
        lua.create_function(|lua, parts: mlua::Table| {
            let mut out = Vec::new();
            for part in parts.sequence_values::<mlua::String>() {
                let part = part?;
                out.extend_from_slice(part.as_bytes().as_ref());
            }
            lua.create_string(&out)
        })?,
    )?;
    buffer.set(
        "from_hex",
        lua.create_function(|lua, s: String| {
            if !s.len().is_multiple_of(2) {
                return Err(mlua::Error::runtime("from_hex: odd hex length"));
            }
            let mut out = Vec::with_capacity(s.len() / 2);
            let bytes = s.as_bytes();
            for pair in bytes.chunks(2) {
                let hi = (pair[0] as char).to_digit(16).ok_or_else(|| {
                    mlua::Error::runtime("from_hex: invalid hex")
                })?;
                let lo = (pair[1] as char).to_digit(16).ok_or_else(|| {
                    mlua::Error::runtime("from_hex: invalid hex")
                })?;
                out.push(((hi << 4) | lo) as u8);
            }
            lua.create_string(&out)
        })?,
    )?;
    pi.set("buffer", buffer)?;
    Ok(())
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    install_crypto(lua, pi)?;
    install_buffer(lua, pi)?;
    Ok(())
}
