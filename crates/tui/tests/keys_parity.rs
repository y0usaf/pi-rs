//! Golden parity with upstream `keys.ts`: fixtures come from running the TypeScript
//! (`parity/gen-keys-fixtures.mjs`), in legacy and Kitty mode.

use pi_tui::keys::*;
use serde_json::Value;

fn opt(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

#[test]
fn keys_match_upstream() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Lone surrogates (from `String.fromCodePoint(0xd800)`) are not valid JSON for serde_json;
    // Rust strings cannot hold them either, so the fixture writes them as escapes we map to U+FFFD.
    let raw = include_str!("fixtures/keys.json").replace("\\ud800", "\\ufffd");
    let fx: Value = serde_json::from_str(&raw).unwrap();
    let ids: Vec<&str> = fx["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let mut failures = Vec::new();
    for (mode, kitty) in [("legacy", false), ("kitty", true)] {
        set_kitty_protocol_active(kitty);
        for row in fx["modes"][mode].as_array().unwrap() {
            let d = row["d"].as_str().unwrap();
            let mut check = |what: &str, got: String, want: String| {
                if got != want {
                    failures.push(format!(
                        "[{mode}] {what}({d:?})\n  got:  {got}\n  want: {want}"
                    ));
                }
            };
            check(
                "parse_key",
                format!("{:?}", parse_key(d)),
                format!("{:?}", opt(&row["parse"])),
            );
            check(
                "decode_kitty_printable",
                format!("{:?}", decode_kitty_printable(d)),
                format!("{:?}", opt(&row["kp"])),
            );
            check(
                "decode_printable_key",
                format!("{:?}", decode_printable_key(d)),
                format!("{:?}", opt(&row["pk"])),
            );
            check(
                "is_key_release",
                is_key_release(d).to_string(),
                row["rel"].to_string(),
            );
            check(
                "is_key_repeat",
                is_key_repeat(d).to_string(),
                row["rep"].to_string(),
            );
            let want: Vec<&str> = row["m"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| ids[i.as_u64().unwrap() as usize])
                .collect();
            let got: Vec<&str> = ids
                .iter()
                .copied()
                .filter(|id| matches_key(d, id))
                .collect();
            check("matches_key", format!("{got:?}"), format!("{want:?}"));
        }
    }
    set_kitty_protocol_active(false);
    let total = failures.len();
    failures.truncate(30);
    assert!(
        total == 0,
        "{total} mismatches; first ones:\n{}",
        failures.join("\n")
    );
}

/// Replays every call upstream's `test/keys.test.ts` makes (recorded by
/// `parity/record-keys-test.mjs`) with the same Kitty state and environment.
#[test]
fn upstream_keys_test_calls() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/keys-upstream-test.json")).unwrap();
    let _guard = ENV_LOCK.lock().unwrap();
    let mut failures = Vec::new();
    for c in &calls {
        set_kitty_protocol_active(c["kitty"].as_bool().unwrap());
        for (name, v) in c["env"].as_object().unwrap() {
            // SAFETY: tests touching the environment hold ENV_LOCK.
            unsafe {
                match v.as_str() {
                    Some(s) => std::env::set_var(name, s),
                    None => std::env::remove_var(name),
                }
            }
        }
        let args = c["args"].as_array().unwrap();
        let d = args[0].as_str().unwrap();
        let got = match c["fn"].as_str().unwrap() {
            "matchesKey" => Value::Bool(matches_key(d, args[1].as_str().unwrap())),
            "parseKey" => parse_key(d).map_or(Value::Null, Value::String),
            "decodeKittyPrintable" => decode_kitty_printable(d).map_or(Value::Null, Value::String),
            "decodePrintableKey" => decode_printable_key(d).map_or(Value::Null, Value::String),
            "isKeyRelease" => Value::Bool(is_key_release(d)),
            "isKeyRepeat" => Value::Bool(is_key_repeat(d)),
            f => panic!("unknown fn {f}"),
        };
        if got != c["r"] {
            failures.push(format!(
                "{} {:?} kitty={} env={}: got {got}, want {}",
                c["fn"], args, c["kitty"], c["env"], c["r"]
            ));
        }
    }
    for n in ["WT_SESSION", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
        unsafe { std::env::remove_var(n) };
    }
    set_kitty_protocol_active(false);
    assert!(
        failures.is_empty(),
        "{} of {} calls differ:\n{}",
        failures.len(),
        calls.len(),
        failures.join("\n")
    );
}

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
