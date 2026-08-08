# Test-suite classification (PLAN A.2)

Every retained suite under `tests/` owns exactly one distinct failure, classified
by the layer that must detect it. A suite is retained only when the contract it
pins is unobservable at a cheaper layer: a unit test of a single function cannot
pin cross-stream ordering, an oracle cannot pin a mechanism invariant, and a
frame comparison cannot pin queued extension actions. Scenario count is not the
metric; unique observable contracts and mutation-resistant failure signals are.

Categories: (a) Rust mechanism invariant, (b) Pi differential (oracle-based),
(c) public Lua exerciser, (d) construction/ablation proof, (e) external dogfood
contract.

## (b) Pi differentials — one per pinned protocol/policy, consumed by crate tests

Each `tests/*-parity/` directory holds `cases.json` (inputs) + `oracle.json`
(real Pi output, generated from `ref/pi` via `scripts/*-oracle` over
`gen-oracle.ts`) and is replayed by a `*_parity.rs` test through the public
command surface. The oracle is the only artifact that can fail when pi-rs drifts
from the pinned Pi release; a Rust unit test asserting expected values would
duplicate the oracle's content and rot independently of Pi.

| Suite | Unique contract | Why a cheaper layer cannot own it |
|---|---|---|
| `tests/agent-parity/` | Lua agent loop (`pi.agent.new`) full subscriber event order, per-stream request snapshots, phase outcomes, final state vs Pi's `Agent/agent-loop.ts` | Ordering across streams/turns emerges only end-to-end; per-function unit tests cannot pin queue-drain and abort transitions |
| `tests/anthropic-parity/` | Anthropic protocol request bodies, SSE event sequence, final messages vs vendored SDK | Wire-format drift is only observable through the real stream path |
| `tests/azure-openai-responses-parity/` | Azure OpenAI Responses protocol | same |
| `tests/bedrock-converse-stream-parity/` | Bedrock Converse stream framing | same |
| `tests/google-generative-ai-parity/` | Gemini protocol | same |
| `tests/google-vertex-parity/` | Vertex protocol incl. certificate external-account ADC (mTLS subject token) + 3 PEM/key fixtures | mTLS identity and STS body only verifiable through the real credential path; fixtures are the only contract |
| `tests/mistral-conversations-parity/` | Mistral Conversations protocol | same |
| `tests/openai-codex-responses-parity/` | Codex Responses SSE/fallback | same |
| `tests/openai-codex-websocket-parity/` | Codex WebSocket framing | same |
| `tests/openai-completions-parity/` | OpenAI Completions protocol; `rich-stream` subsumes the deleted hand-built fixtures | replaced the old hand-derived expectation fixtures (commit 78ed0b4) |
| `tests/openai-responses-parity/` | OpenAI Responses protocol | same |
| `tests/compaction-parity/` | Lua compaction policy cut points, summarization requests, summaries, overflow vs Pi compaction.ts | policy lives in Lua; only the full pipeline shows the cut/request coupling |
| `tests/retry-parity/` | retry classification, attempt/event order, context removal, exhaustion, cancellation matrix | matrix behavior is cross-component (session + provider + policy) |
| `tests/session-parity/` | session persistence/resume/tree semantics vs Pi | same |
| `tests/system-prompt-parity/` | Lua system-prompt port (buildSystemPrompt + project context files) vs Pi | pure Lua policy; oracle pins exact prompt text incl. TZ/epoch |
| `tests/tool-parity/` | tool call/result conversion, argument handling, renderers vs Pi | conversion matrix is broad; oracle is the compact authoritative form |
| `tests/hljs-parity/` | `pi.hljs.*` highlighting vs vendored highlight.js 10.7.3 | exact token output only reproducible from the library |
| `tests/jsdiff-parity/` | jsdiff algorithm port vs Pi's jsdiff | diff output only reproducible from the reference implementation |
| `tests/image-parity/` | image protocol handling vs Pi | same |
| `tests/export-html-parity/` | standalone HTML export shape (DOCTYPE, CSS, render helpers, session-data) | export contract is the generated page itself |
| `tests/extension-context-parity/` (9.2) | ExtensionContext/CommandContext snapshots + queued lifecycle actions vs Pi v0.79.0 | action semantics (abort/compact/shutdown/wait) only observable through live snapshots |
| `tests/extension-event-parity/` (9.3) | event pipeline folds + ordering; `01-first.lua`/`02-second.lua` are the public Lua subscribers (exercisers) | fold order/replacement semantics emerge only from the real event run |
| `tests/extension-runtime-parity/` (9.1) | extension loader/runtime behavior vs Pi | same |
| `tests/extension-ui-parity/` (9.1) | queued extension UI actions extracted from the `extension-ui-turn` ui-parity scenario | frame comparison pins rendering, not the queued-action list; this oracle pins the actions |

## (c) Public Lua exercisers

- `examples/extensions/*.lua` (61 files) — run through the public extension
  surface by the `pi-rs-host` tests (acceptance, agent_mechanisms, auth_bindings,
  providers, registries, …). Each demonstrates one public API member end-to-end;
  a Rust-side mock of the API would test the mock, not the surface.
- `tests/extension-event-parity/01-first.lua`, `02-second.lua` — subscribers
  exercising the public `pi.events` surface inside the (b) oracle suite.

## (d) Construction/ablation proofs

- `tests/construction-inventory/` — fail-closed first-party assembly manifest;
  `test_checker.py` pins rejection behavior. Owns the *inventory* failure (a
  unit/mechanism test cannot prove every construction row stays classified).
  Manifest is parent-tracked, not edited here.
- `tests/extension-inventory/` — extension surface + translation matrix
  manifest, `pinned-surface.json`. Same rationale.
- `tests/final-parity-audit/` — closed offline source/public-surface audit vs
  the pinned Pi extraction. Migration-era audit; per PLAN A.2 accept it should
  retire or collapse once open 9.x rows close — currently still wired into
  `nix flake check` and parent-tracked, so retained untouched.
- `tests/model-catalog-update/` — fixtures (`*.generated.ts`, `overrides.json`)
  for the reviewed model-catalog update path: idempotency + rejection of unknown
  fields / unsupported wire protocols. Owns the update-path gate that
  `scripts/test-model-catalog-update` runs; a mechanism unit test cannot pin the
  reviewed-tool contract.

## (e) External dogfood contracts

- `tests/dogfood-suite/` — pinned pi-flake fixture contract (`contract.json` +
  `test_contract.py`); provenance/source-identity is deliberately duplicated
  outside the JSON so a checked fixture cannot silently rewrite pinned identity.
  Parent-tracked.
- `tests/external-extension-inventory/` — capability inventory + provenance of
  the 15 pinned external extensions; `fixtures/` are checked pinned oracle
  inputs (A.3 moves them to a hash-locked Nix oracle input). Parent-tracked.

## (a) Rust mechanism invariants (crate test files)

Component/unit tests are retained only where they localize a distinct failure
the oracles cannot: `pi-rs-ai` protocol internals without an oracle owner
(`http`, `sse`, `json_parse`, `retry`, `transform_messages`, `event_stream`,
`registry`, `openai_completions` mechanism tests), `pi-rs-app` product-path pins
through public commands (`interactive_*`, `tools`, `extension_loading`,
`assembly`, `ai_auth_catalog`, `startup_network`, `anthropic_replay`,
`block_images_replay`, `agent_tool_roundtrip`), `pi-rs-host` binding/TUI tests
(`*_bindings`, `tui_*`, `seam`, `parallel`, `config_pipeline`, `discovery_trust`,
`json_boundary`, `public_surface`, `embedded`), `pi-rs-agent` loop-contract
tests (`lifecycle`, `loop_policy`, `state`, `streamed_turn`, `tool_roundtrip`),
`pi-rs-session` persistence tests, `pi-rs-ai-auth` OAuth/PKCE tests,
`pi-rs-ai-types` model/fixture tests.

## Shared harness (one implementation)

- `crates/pi-rs-ai/tests/common/mod.rs` — scripted loopback server +
  normalization + request reader for the 12 ai parity/mechanism tests.
- `crates/pi-rs-app/tests/common/mod.rs` — product host/fixture helpers + the
  factored `spawn_stub`/`StubResponse` (7 local copies), `text_sse` (3 local
  copies), `normalize_empty_object` (3 local copies). Scenario-specific SSE
  bodies (`hang_sse`, `SUMMARY_SSE`, `DONE_SSE`, `success_sse`), per-test
  `run_sequence` envelopes, and session writers stay local.
- `crates/pi-rs-session/tests/common/mod.rs` — session harness.
- Cross-crate duplicate of `normalize_empty_object` in `pi-rs-host/jsdiff_parity`
  is left local: a shared test-support crate would be over-abstraction for one
  12-line helper, and crate tests cannot share code without one.
