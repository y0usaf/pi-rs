# Acceptance suite ownership

Each permanent suite has one acceptance owner; overlap is diagnostic only.

| Suite | Unique contract owner |
|---|---|
| `tests/experience/**` + `crates/pi-rs-app/tests/experience_contract.rs` | Bounded canonical terminal grids/input journeys selected by PLAN 0.2. |
| `tests/performance/**` + `crates/pi-rs-app/tests/performance_contract.rs` | Versioned release benchmark schema, parameters, and budgets selected by PLAN 0.2. |
| `tests/{anthropic,azure-openai-responses,bedrock-converse-stream,google-generative-ai,google-vertex,mistral-conversations,openai-codex-responses,openai-codex-websocket,openai-completions,openai-responses}-parity/**` + matching `crates/pi-rs-ai/tests/*_parity.rs` | Pinned provider request/stream/error/cancellation wire compatibility for the named protocol family. These checked fixtures do not imply product parity. |
| `tests/model-catalog-update/**` and `scripts/{update-model-catalog.ts,test-model-catalog-update,model-catalog-overrides.json}` | Fail-closed model-catalog normalization, provenance, and reviewed update workflow. |
| `crates/pi-rs-ai/tests/{event_stream,http,json_parse,openai_completions,registry,retry,sse,transform_messages}.rs` + `crates/pi-rs-ai/tests/fixtures/**` | Provider transport, registry, conversion, and replay mechanisms independent of product workflow. |
| `crates/pi-rs-ai-types/tests/**` | Typed provider/model/message wire-schema round trips and validation. |
| `crates/pi-rs-ai-auth/tests/**` | Credential, PKCE, callback/device-flow, registry, and subscription-auth engines. |
| `crates/pi-rs-host/tests/**` | Generic kernel transactions, bounded effects/cleanup, exact compact Lua surface, source neutrality, terminal input/display, and the minimum file-backed coding spine. |
| `crates/pi-rs-app/tests/{launcher,launcher_coding_spine,launcher_surface,source_audit}.rs` | Raw zero-policy guidance, generic manifest/package loading, explicit file-backed coding capability, and absence of linked product policy. |
| `crates/pi-rs-app/tests/startup_paths.rs` | XDG root selection and read-only per-resource legacy fallback. |
| `crates/pi-rs-app/tests/ai_auth_catalog.rs` | Every advertised model API dispatches and every supported subscription auth family is registered. |
| `flake.nix` checks `workspace-test`, `workspace-clippy`, `core-no-package`, `core-file-application`, and `model-catalog-update` | Aggregate retained suites, shipped-target lint safety, raw-core ablation/capability, and fail-closed catalog updates. |

Whole-product snapshots, Pi extension compatibility, external-extension dogfood,
and legacy session/tool/frontend policy have no permanent suite here.
