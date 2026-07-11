//! Pins `pi.image.*` to the vendored `@silvia-odwyer/photon-node` 0.3.4
//! WASM build Pi's coding agent uses (`utils/image-resize-core.ts`
//! `resizeImageInProcess`, `utils/image-convert.ts` `convertToPng`). The
//! oracle in tests/image-parity/oracle.json is generated from the
//! vendored library by scripts/image-oracle (which also synthesizes and
//! records the case inputs); cases are replayed through the public Lua
//! surface, never the Rust module directly, and encoded bytes must match
//! byte-for-byte.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

const RUNNER: &str = r#"
local pi = ...

local B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local REV = {}
for i = 1, #B64 do REV[B64:sub(i, i)] = i - 1 end

local function base64_decode(data)
  local out = {}
  local bits, count = 0, 0
  for i = 1, #data do
    local c = data:sub(i, i)
    local v = REV[c]
    if v ~= nil then
      bits = (bits << 6) | v
      count = count + 6
      if count >= 8 then
        count = count - 8
        out[#out + 1] = string.char((bits >> count) & 0xFF)
      end
    end
  end
  return table.concat(out)
end

pi.register_command("image-parity-run", {
  handler = function(args)
    local cases = pi.json.decode(args)
    local out = {}
    for i, c in ipairs(cases) do
      local result
      if c.kind == "resize" then
        result = pi.image.resize(base64_decode(c.input), c.mimeType, c.options)
      else
        result = pi.image.convert_to_png(c.input, c.mimeType)
      end
      if result == nil then
        out[i] = { name = c.name, isNull = true }
      else
        out[i] = { name = c.name, expected = result }
      end
    end
    return out
  end,
})
"#;

fn fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/image-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture parses")
}

#[test]
fn pi_image_matches_vendored_photon_oracle() {
    let oracle = fixture("oracle.json");
    let cases = oracle.as_array().expect("oracle is an array");
    assert!(cases.len() >= 20, "oracle should carry the full case set");

    let host = Host::new(HostConfig::default()).expect("host");
    host.load("image-test", RUNNER).expect("runner loads");
    let result = host
        .call_command("image-parity-run", &oracle.to_string())
        .expect("command")
        .expect("result");
    let results = result.as_array().expect("results array");
    assert_eq!(results.len(), cases.len());

    for (case, got) in cases.iter().zip(results) {
        let name = case["name"].as_str().expect("case name");
        assert_eq!(got["name"].as_str(), Some(name), "case order for {name}");
        let want = &case["expected"];
        if want.is_null() {
            assert_eq!(
                got["isNull"].as_bool(),
                Some(true),
                "{name}: expected null result"
            );
            continue;
        }
        let got = &got["expected"];
        for key in ["mimeType", "data"] {
            assert_eq!(got[key], want[key], "{name}: {key} mismatch");
        }
        if case["kind"].as_str() == Some("resize") {
            for key in [
                "originalWidth",
                "originalHeight",
                "width",
                "height",
                "wasResized",
            ] {
                assert_eq!(got[key], want[key], "{name}: {key} mismatch");
            }
        }
    }
}

#[test]
fn image_demo_example_exercises_the_public_surface() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/image-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("image-demo", "")
        .expect("command")
        .expect("result");

    assert_eq!(result["passthrough"]["wasResized"], false);
    assert_eq!(result["passthrough"]["originalWidth"], 4);
    assert_eq!(result["passthrough"]["originalHeight"], 2);

    assert_eq!(result["resized"]["wasResized"], true);
    assert_eq!(result["resized"]["width"], 2);
    assert_eq!(result["resized"]["height"], 1);

    assert_eq!(result["impossible_was_nil"], true);
    assert_eq!(result["converted"]["mimeType"], "image/png");
}
