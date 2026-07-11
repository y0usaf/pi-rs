//! Pins the JSON ⇄ Lua boundary to `JSON.parse`/`JSON.stringify` semantics
//! (PLAN 2b.5). Pi renders model-emitted tool args with
//! `JSON.stringify(args, null, 2)` in JS [[OwnPropertyKeys]] order — array
//! indices ascending, then string keys in wire order — and prints integral
//! doubles without a fraction. Lua tables are unordered, so `pi.json.decode`
//! records the order in a metatable and `pi.json.encode` replays it. Oracle
//! strings below were generated with node 22 (`JSON.stringify`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

const RUNNER: &str = r#"
local pi = ...
pi.register_command("roundtrip", {
  handler = function(args)
    return { text = pi.json.encode(pi.json.decode(args), true) }
  end,
})
pi.register_command("roundtrip-compact", {
  handler = function(args)
    return { text = pi.json.encode(pi.json.decode(args)) }
  end,
})
pi.register_command("mutate", {
  handler = function(args)
    local value = pi.json.decode(args)
    value.added = "later"
    value.a = 3
    return { text = pi.json.encode(value) }
  end,
})
pi.register_command("lua-authored", {
  handler = function()
    return { text = pi.json.encode({ zeta = 1, alpha = { 0 / 0, 2.0, 2.5 } }) }
  end,
})
"#;

const WIRE: &str = concat!(
    r#"{"url":"https://example.com/api/items","method":"POST","10":"ten","#,
    r#""2":"two","retries":1.0,"headers":{"x-request-id":"abc-123","#,
    r#""accept":"application/json"},"verbose":true}"#
);

fn run(host: &Host, command: &str, args: &str) -> String {
    let result = host
        .call_command(command, args)
        .expect("command")
        .expect("result");
    result["text"].as_str().expect("text").to_owned()
}

#[test]
fn decode_encode_matches_json_stringify() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load("json-test", RUNNER).expect("runner loads");

    // JSON.stringify(JSON.parse(WIRE), null, 2): indices "2","10" ascend
    // first, the rest keep wire order, and 1.0 collapses to 1.
    let pretty = run(&host, "roundtrip", WIRE);
    assert_eq!(
        pretty,
        "{\n  \"2\": \"two\",\n  \"10\": \"ten\",\n  \"url\": \"https://example.com/api/items\",\n  \"method\": \"POST\",\n  \"retries\": 1,\n  \"headers\": {\n    \"x-request-id\": \"abc-123\",\n    \"accept\": \"application/json\"\n  },\n  \"verbose\": true\n}"
    );

    let compact = run(&host, "roundtrip-compact", WIRE);
    assert_eq!(
        compact,
        r#"{"2":"two","10":"ten","url":"https://example.com/api/items","method":"POST","retries":1,"headers":{"x-request-id":"abc-123","accept":"application/json"},"verbose":true}"#
    );
}

#[test]
fn lua_added_keys_follow_recorded_order() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load("json-test", RUNNER).expect("runner loads");

    // Overwriting a decoded key keeps its wire position; a Lua-added key
    // lands after every recorded key (sorted remainder).
    let text = run(&host, "mutate", r#"{"b":1,"a":2}"#);
    assert_eq!(text, r#"{"b":1,"a":3,"added":"later"}"#);
}

#[test]
fn lua_authored_tables_encode_deterministically_with_js_numbers() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load("json-test", RUNNER).expect("runner loads");

    // No boundary metadata → sorted keys; NaN → null and 2.0 → 2, like
    // JSON.stringify.
    let text = run(&host, "lua-authored", "");
    assert_eq!(text, r#"{"alpha":[null,2,2.5],"zeta":1}"#);
}
