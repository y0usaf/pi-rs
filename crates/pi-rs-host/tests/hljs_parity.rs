//! Pins `pi.hljs.*` to the vendored highlight.js 10.7.3 that Pi's coding
//! agent uses (`utils/syntax-highlight.ts`). The oracle in
//! tests/hljs-parity/oracle.json is generated from
//! `ref/pi/node_modules/highlight.js` by scripts/hljs-oracle; cases are
//! replayed through the public Lua surface, never the Rust module directly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

const RUNNER: &str = r#"
local pi = ...
pi.register_command("hljs-run", {
  handler = function(args)
    local cases = pi.json.decode(args)
    local out = {}
    for i, c in ipairs(cases) do
      local result = pi.hljs.highlight(c.code, {
        language = c.language,
        ignore_illegals = true,
      })
      out[i] = {
        name = c.name,
        value = result.value,
        relevance = result.relevance,
        illegal = result.illegal,
        detectedLanguage = result.language,
      }
    end
    return out
  end,
})
"#;

fn fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/hljs-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture parses")
}

/// Every grammar in the scoped catalog loads, translates, and parses.
#[test]
fn every_grammar_compiles_and_runs() {
    let names = pi_rs_host::hljs::list_languages().expect("catalog loads");
    assert!(
        names.len() >= 40,
        "expected the scoped grammar set, got {names:?}"
    );
    for name in &names {
        assert!(pi_rs_host::hljs::supports_language(name), "{name} missing");
        let result = pi_rs_host::hljs::highlight("x = 1", name, true).expect("highlight runs");
        assert!(!result.value.is_empty(), "{name} produced no output");
    }
}

#[test]
fn pi_hljs_matches_vendored_highlight_js_oracle() {
    let cases = fixture("cases.json");
    let oracle = fixture("oracle.json");

    let host = Host::new(HostConfig::default()).expect("host");
    host.load("hljs-test", RUNNER).expect("runner loads");
    let result = host
        .call_command("hljs-run", &cases.to_string())
        .expect("command")
        .expect("result");

    let mut got = result.as_array().expect("results").clone();
    let mut want = oracle.as_array().expect("oracle").clone();
    assert_eq!(got.len(), want.len(), "case count");
    // Lua numbers cross the boundary as floats; the oracle serializes
    // integral relevance as integers. Same value, different JSON spelling.
    for case in got.iter_mut().chain(want.iter_mut()) {
        if let Some(relevance) = case.get("relevance").and_then(|v| v.as_f64()) {
            case["relevance"] = serde_json::json!(relevance);
        }
    }
    let mut failures = Vec::new();
    for (got_case, want_case) in got.iter().zip(&want) {
        if got_case != want_case {
            failures.push(format!(
                "case {}:\n  pi : {}\n  pi-rs: {}",
                want_case["name"], want_case, got_case
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} oracle cases diverge from highlight.js 10.7.3:\n{}",
        failures.len(),
        want.len(),
        failures.join("\n")
    );
    assert!(
        want.len() >= 50,
        "oracle unexpectedly small: {}",
        want.len()
    );
}

#[test]
fn highlight_demo_example_exercises_the_public_surface() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/highlight-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("highlight-demo", "")
        .expect("command")
        .expect("result");
    let ts_value = result["ts_value"].as_str().expect("ts value");
    assert!(
        ts_value.contains(r#"<span class="hljs-keyword">const</span>"#),
        "{ts_value}"
    );
    assert!(
        ts_value.contains(r#"<span class="hljs-comment">// note</span>"#),
        "{ts_value}"
    );
    assert_eq!(result["aliases"]["html"], serde_json::json!(true));
    assert_eq!(result["aliases"]["toml"], serde_json::json!(true));
    assert_eq!(result["aliases"]["quux"], serde_json::json!(false));
    assert_eq!(result["detected"], serde_json::json!("json"));
    assert!(result["languages"].as_i64().unwrap_or(0) >= 40);
}

/// The Lua surface mirrors the library API, including the
/// unknown-language error `highlightCode`'s try/catch depends on.
#[test]
fn lua_surface_exposes_the_library() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load(
        "hljs-surface",
        r#"
        local pi = ...
        pi.register_command("hljs-check", {
          handler = function()
            local ts = pi.hljs.highlight("const x = 1;", { language = "typescript", ignore_illegals = true })
            local ok, err = pcall(function()
              pi.hljs.highlight("x", { language = "not-a-language" })
            end)
            return {
              value = ts.value,
              supports_ts = pi.hljs.supports_language("typescript"),
              supports_alias = pi.hljs.supports_language("HTML"),
              supports_missing = pi.hljs.supports_language("brainfuck"),
              unknown_error = (not ok) and tostring(err) or nil,
            }
          end,
        })
        "#,
    )
    .expect("extension loads");
    let result = host
        .call_command("hljs-check", "")
        .expect("command")
        .expect("result");
    let value = result["value"].as_str().expect("value");
    assert!(
        value.contains(r#"<span class="hljs-keyword">const</span>"#),
        "unexpected html: {value}"
    );
    assert_eq!(result["supports_ts"], serde_json::json!(true));
    assert_eq!(result["supports_alias"], serde_json::json!(true));
    assert_eq!(result["supports_missing"], serde_json::json!(false));
    let unknown = result["unknown_error"].as_str().expect("error string");
    assert!(unknown.contains("Unknown language"), "{unknown}");
}
