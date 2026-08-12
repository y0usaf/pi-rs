//! Pins `pi.register_provider` validation and custom-API-stream registration
//! to Pi's real `ModelRegistry.registerProvider` (coding-agent
//! model-registry.ts) and `api-registry.ts`, driven by the Pi-generated
//! oracle in tests/provider-registry-parity/oracle.json. The oracle is
//! replayed through the public Lua surface, never the Rust module directly.
//!
//! For every oracle case we replay the exact provider config through
//! `pi.register_provider` and compare the thrown error (the spec's
//! `validateProviderConfig` throw) byte-for-byte with Pi's recorded message.
//! Cases Pi accepts with a custom-API `streamSimple` are also asserted to
//! register a dispatching handler through `pi.ai.stream_simple`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/provider-registry-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture parses")
}

/// One command runs one oracle case:
///  1. Replay the exact config via `pi.register_provider`, capturing the
///     thrown error message (or null) byte-for-byte.
///  2. When accepted and a truthy `api` is present, probe that a custom
///     stream handler dispatches through `pi.ai.stream_simple`.
const RUNNER: &str = r#"
local pi = ...
local function placeholder_stream(model, context, options, on_event)
  if options ~= nil and type(options) == "table" and options.signal then end
  on_event({ type = "start", partial = {} })
  on_event({ type = "done", reason = "stop", message = {
    role = "assistant", content = {}, api = model.api, provider = "case",
    model = model.id, stopReason = "stop", timestamp = 0,
  } })
  return { role = "assistant", content = {}, provider = "case", stopReason = "stop" }
end
local function with_fn(cfg)
  if type(cfg) ~= "table" then return cfg end
  local out = {}
  for k, v in pairs(cfg) do
    if v == "<function>" and k == "streamSimple" then
      out[k] = placeholder_stream
    elseif type(v) == "table" then
      out[k] = with_fn(v)
    else
      out[k] = v
    end
  end
  return out
end
pi.register_command("provider-case", {
  handler = function(args)
    local case = pi.json.decode(args)
    local threw = nil
    local ok, err = pcall(function()
      pi.register_provider(case.name, with_fn(case.config))
    end)
    if not ok then
      -- mlua prefixes the error with "runtime error: " plus a traceback;
      -- Pi's throw surfaces only the raw validateProviderConfig message, so
      -- normalize to the first message line (drop prefix and traceback).
      -- ASSUMPTION: this couples to mlua's error-string format ("runtime
      -- error: " prefix + "\nstack traceback: ..."). It's a host-side test
      -- harness detail, not part of the Pi differential surface, so it is
      -- intentionally not pinned against an oracle; if mlua's wrapping ever
      -- changes, normalize here rather than asserting the raw string.
      local full = tostring(err)
      full = full:gsub("^runtime error: ", "", 1)
      local nl = full:find("\n")
      if nl then full = full:sub(1, nl - 1) end
      threw = full
    end
    local customDispatches = false
    local api = case.config.api
    if threw == nil and api ~= nil and #api > 0 then
      local model = {
        id = "case-1", name = "Case", api = api, provider = case.name,
        baseUrl = "", reasoning = false, input = { "text" },
        cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
        contextWindow = 100000, maxTokens = 8000,
      }
      local ok2, final_or_err = pcall(function()
        local events = {}
        local final = pi.ai.stream_simple(model, { messages = {} }, nil,
          function(ev) events[#events + 1] = ev.type end)
        return final
      end)
      if ok2 and final_or_err ~= nil and final_or_err.stopReason == "stop" then
        customDispatches = true
      end
    end
    return { threw = threw, customDispatches = customDispatches }
  end,
})
"#;

fn replay(host: &Host, case: &serde_json::Value) -> serde_json::Value {
    host.call_command("provider-case", &case.to_string())
        .unwrap()
        .unwrap()
}

#[test]
fn provider_validation_matches_pi_oracle() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load("<runner>", RUNNER).unwrap();

    let oracle = fixture("oracle.json");
    let cases = oracle["cases"].as_array().expect("cases").clone();

    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let expected_threw = case["threw"].as_str();
        let got = replay(&host, case);
        let got_threw = got["threw"].as_str();
        assert_eq!(
            got_threw, expected_threw,
            "case {name}: thrown error differs from Pi oracle",
        );

        // For streamSimple/accept cases, Pi registers a custom API handler.
        let has_custom_stream = case["config"]["streamSimple"].as_str() == Some("<function>");
        let api_present = case["config"]["api"].as_str().map(|s| !s.is_empty()).unwrap_or(false);
        if has_custom_stream && got_threw.is_none() && api_present {
            assert!(
                got["customDispatches"].as_bool().unwrap_or(false),
                "case {name}: accepted provider with a truthy api must dispatch a custom stream handler"
            );
        }
    }
}
