# Acceptance suite ownership

Each permanent suite has one acceptance owner; overlap is diagnostic only.

| Suite | Unique contract owner |
|---|---|
| `tests/experience/**` + `crates/pi-rs-app/tests/experience_contract.rs` | Bounded canonical terminal grids/input journeys selected by PLAN 0.2. |
| `tests/performance/**` + `crates/pi-rs-app/tests/performance_contract.rs` | Versioned release benchmark schema, parameters, and budgets selected by PLAN 0.2. |
| `tests/{anthropic,azure-openai-responses,bedrock-converse-stream,google-generative-ai,google-vertex,mistral-conversations,openai-codex-responses,openai-codex-websocket,openai-completions,openai-responses}-parity/**` + matching `crates/pi-rs-ai/tests/*_parity.rs` | Pinned provider request/stream/error/cancellation wire compatibility for the named protocol family. These checked fixtures do not imply product parity. |
| `tests/model-catalog-update/**`, `scripts/{update-model-catalog.ts,test-model-catalog-update,model-catalog-overrides.json}`, and `.github/workflows/model-catalog-update.yml` | Fail-closed model-catalog normalization, provenance, and reviewed update workflow. |
| `crates/pi-rs-ai/tests/{event_stream,http,json_parse,openai_completions,registry,retry,sse,transform_messages}.rs` + `crates/pi-rs-ai/tests/fixtures/**` | Provider transport, registry, conversion, and replay mechanisms independent of product workflow. |
| `crates/pi-rs-ai-types/tests/**` | Typed provider/model/message wire-schema round trips and validation. |
| `crates/pi-rs-ai-auth/tests/**` | Credential, PKCE, callback/device-flow, registry, and subscription-auth engines. |
| `crates/pi-rs-host/tests/**` | Generic host/Lua mechanism invariants: bounded dispatch, source neutrality, registries, effects, JSON, and terminal primitives. |
| `crates/pi-rs-agent/tests/**` | Public Lua agent primitive state, lifecycle, streaming, and tool-settlement mechanics; not default-product behavior. |
| `crates/pi-rs-app/tests/assembly.rs` | Zero-pack boot plus package/root suppression and file-backed replacement. |
| `crates/pi-rs-app/tests/agent_tool_roundtrip.rs` | Ordinary file-backed Lua consumer spanning agent and tool capabilities. |
| `crates/pi-rs-app/tests/ai_auth_catalog.rs` | Every advertised model API dispatches and every supported subscription auth family is registered. |
| `flake.nix` checks `workspace-test`, `workspace-clippy`, and `launcher-smoke` | Aggregate retained suites, shipped-target lint safety, and installed-binary/catalog startup respectively. |

Whole-product snapshots, Pi extension compatibility, external-extension dogfood,
and legacy session/tool/frontend policy have no permanent suite here.
