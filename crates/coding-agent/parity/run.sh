#!/usr/bin/env bash
# Tool parity: runs parity/tools.json through upstream's tools (TS, from $PI_TS) and through
# this crate's (examples/tools_parity), and compares the results.
#   PI_TS=/path/to/pi crates/coding-agent/parity/run.sh
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
: "${PI_TS:?set PI_TS to an upstream pi checkout with node_modules installed}"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
cargo build -q -p pi-coding-agent --example tools_parity
bin="$(cd "$here/../../.." && pwd)/target/debug/examples/tools_parity"
mkdir -p "$out/ts" "$out/rs"
PI_CODING_AGENT_DIR="$out/agent" PI_OFFLINE=1 node --import "$PI_TS/packages/coding-agent/src/experimental/source-resolver.ts" \
	"$here/tools-driver.ts" "$here/tools.json" "$out/ts" >"$out/ts.jsonl"
PI_CODING_AGENT_DIR="$out/agent" "$bin" "$here/tools.json" "$out/rs" >"$out/rs.jsonl"
node "$here/compare-tools.mjs" "$out/ts.jsonl" "$out/rs.jsonl" "$here/tools.json"

# Model resolution parity: parity/models.json through model-resolver.ts and examples/models_parity.
cargo build -q -p pi-coding-agent --example models_parity
PI_CODING_AGENT_DIR="$out/agent" PI_OFFLINE=1 node --import "$PI_TS/packages/coding-agent/src/experimental/source-resolver.ts" \
	"$here/models-driver.ts" "$here/models.json" >"$out/models-ts.jsonl"
"$(dirname "$bin")/models_parity" "$here/models.json" >"$out/models-rs.jsonl"
node "$here/compare-lines.mjs" models "$out/models-ts.jsonl" "$out/models-rs.jsonl"

# System prompt parity: parity/prompt.json (context files, SYSTEM.md, skills) through upstream's
# resource loader and buildSystemPrompt, and through examples/prompt_parity.
cargo build -q -p pi-coding-agent --example prompt_parity
mkdir -p "$out/prompt-ts" "$out/prompt-rs"
PI_CODING_AGENT_DIR="$out/agent" PI_OFFLINE=1 node --import "$PI_TS/packages/coding-agent/src/experimental/source-resolver.ts" \
	"$here/prompt-driver.ts" "$here/prompt.json" "$out/prompt-ts" >"$out/prompt-ts.jsonl"
"$(dirname "$bin")/prompt_parity" "$here/prompt.json" "$out/prompt-rs" >"$out/prompt-rs.jsonl"
node "$here/compare-lines.mjs" prompt "$out/prompt-ts.jsonl" "$out/prompt-rs.jsonl"
