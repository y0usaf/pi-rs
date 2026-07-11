//! Pins `pi.diff.*` to the vendored jsdiff 8.0.4 that Pi's coding agent
//! uses (`edit-diff.ts` diffLines/createTwoFilesPatch, `components/diff.ts`
//! diffWords). The oracle in tests/jsdiff-parity/oracle.json is generated
//! from `ref/pi/node_modules/diff` by scripts/jsdiff-oracle; cases are
//! replayed through the public Lua surface, never the Rust module directly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

const RUNNER: &str = r#"
local pi = ...
pi.register_command("jsdiff-run", {
  handler = function(args)
    local cases = pi.json.decode(args)
    local out = { lines = {}, words = {}, patch = {} }
    for i, c in ipairs(cases.lines) do
      out.lines[i] = { name = c.name, changes = pi.diff.lines(c.old, c.new) }
    end
    for i, c in ipairs(cases.words) do
      out.words[i] = { name = c.name, changes = pi.diff.words(c.old, c.new) }
    end
    for i, c in ipairs(cases.patch) do
      out.patch[i] = {
        name = c.name,
        patch = pi.diff.unified_patch(c.oldName, c.newName, c.old, c.new,
          { context = c.context, headers = c.headers }),
      }
    end
    return out
  end,
})
"#;

/// An empty Lua table crosses the boundary as `{}`; jsdiff's empty change
/// list is `[]`. Normalize the encoding artifact before comparing.
fn normalize_empty(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) if map.is_empty() => {
            *value = serde_json::Value::Array(Vec::new());
        }
        serde_json::Value::Object(map) => {
            for child in map.values_mut() {
                normalize_empty(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                normalize_empty(child);
            }
        }
        _ => {}
    }
}

fn fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/jsdiff-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture parses")
}

#[test]
fn pi_diff_matches_vendored_jsdiff_oracle() {
    let cases = fixture("cases.json");
    let oracle = fixture("oracle.json");

    let host = Host::new(HostConfig::default()).expect("host");
    host.load("jsdiff-test", RUNNER).expect("runner loads");
    let mut result = host
        .call_command("jsdiff-run", &cases.to_string())
        .expect("command")
        .expect("result");
    normalize_empty(&mut result);

    for section in ["lines", "words", "patch"] {
        let got = result[section].as_array().expect(section);
        let want = oracle[section].as_array().expect(section);
        assert_eq!(got.len(), want.len(), "{section}: case count");
        for (got_case, want_case) in got.iter().zip(want) {
            assert_eq!(
                got_case, want_case,
                "{section} case {} diverges from jsdiff 8.0.4",
                want_case["name"]
            );
        }
    }
}

#[test]
fn diff_demo_example_exercises_the_public_surface() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/diff-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("diff-demo", "")
        .expect("command")
        .expect("result");

    // jsdiff dedupes repeated whitespace across change boundaries:
    // K:'foo ' D:'bar ' K:'baz' (word.js dedupeWhitespaceInChangeObjects).
    assert_eq!(result["words"][0]["value"], "foo ");
    assert_eq!(result["words"][1]["value"], "bar ");
    assert_eq!(result["words"][1]["removed"], true);
    assert_eq!(result["words"][2]["value"], "baz");

    assert_eq!(result["lines"][1]["removed"], true);
    assert_eq!(result["lines"][1]["value"], "two\n");

    let patch = result["patch"].as_str().expect("patch string");
    assert!(patch.starts_with("--- greeting.txt\n+++ greeting.txt\n"));
    assert!(patch.contains("\\ No newline at end of file"));
}
