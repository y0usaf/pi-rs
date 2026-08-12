#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

const KNOWN: &[(&str, &str, &str, &str)] = &[
    // (input, sha1, sha256, md5)
    (
        "hello",
        "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        "5d41402abc4b2a76b9719d911017c592",
    ),
];

#[test]
fn crypto_hashes_match_reviewed_primitives() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "crypto-demo.lua",
        include_str!("../../../examples/extensions/crypto-demo.lua"),
    )
    .unwrap();
    let out = host.call_command("crypto-demo", "").unwrap().unwrap();

    let (_, sha1, sha256, md5) = KNOWN[0];
    assert_eq!(out["sha1"], sha1);
    assert_eq!(out["sha256"], sha256);
    assert_eq!(out["md5"], md5);
    assert!(out["xxhash32"].as_u64().unwrap() > 0);

    assert_eq!(out["uuid_shape"], true);
    assert_eq!(out["unique"], true);
    assert_eq!(out["uuid_shape"], true);

    assert_eq!(out["base64"], "aGVsbG8=");
    assert_eq!(out["decoded"], "hello");
    assert_eq!(out["byte_len"], 5);
}

#[test]
fn crypto_uuid_is_version4_and_rfc4122() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "test://uuid",
        r#"
        local pi = ...
        pi.register_command("uuid", {
            handler = function()
                local u = pi.crypto.random_uuid()
                local ok = u:find("^%x%x%x%x%x%x%x%x%-%x%x%x%x%-4%x%x%x%-[89ab]%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$")
                return { valid = ok ~= nil, uuid = u }
            end,
        })
        "#,
    )
    .unwrap();
    let out = host.call_command("uuid", "").unwrap().unwrap();
    assert_eq!(out["valid"], true);
    assert_eq!(out["uuid"].as_str().unwrap().len(), 36);
}

#[test]
fn base64_roundtrip_through_binary_safe_string() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "test://b64",
        r#"
        local pi = ...
        pi.register_command("b64", {
            handler = function()
                local enc = pi.buffer.base64_encode("a\0b")
                local dec = pi.buffer.base64_decode(enc)
                return { enc = enc, dec = dec, len = pi.buffer.byte_length("a\0b") }
            end,
        })
        "#,
    )
    .unwrap();
    let out = host.call_command("b64", "").unwrap().unwrap();
    // NUL preserved through binary-safe Lua strings.
    assert_eq!(out["enc"], "YQBi");
    assert_eq!(out["dec"], "a\0b");
    assert_eq!(out["len"], 3);
}
