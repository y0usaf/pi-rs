# pi-rs — Pi's coding agent in Rust

pi-rs is a faithful Rust port of [Pi](https://github.com/badlogic/pi-mono)'s
coding agent. The vendored Pi v0.79.0 snapshot at `ref/pi` (commit `c5582102`)
is the product specification for everything the coding agent does. Where this
document and the spec disagree, the spec wins unless the divergence is listed
below or has a locked-decision row.

Two commitments:

1. **Parity.** Given the same terminal, configuration, credentials, model,
   input, and provider responses, a user must not be able to distinguish
   pi-rs from Pi's coding agent: rendering, input behavior, commands, errors,
   requests, persistence, and session behavior all match.
2. **Compatibility maintenance.** pi-rs remains the faithful port. Custom
   products and behavior start as downstream forks after the parity gate rather
   than turning this repository into a dialect.

Rust is the implementation language. A different implementation is not a
license to invent a different product.

## The three divergences

Everything not listed here must look and feel **1:1 with Pi's coding agent**.

1. **Lua, not TypeScript.** Extensions and configuration are Lua (mlua,
   vendored 5.4). Pi's extension examples arrive as translations, not
   copy-paste — and each translation is a conformance test of the bridge. A Pi
   coding-agent example that *cannot* translate is a bridge bug, not an example
   we skip.
2. **Everything first-party is Lua on the public extension surface — no Rust
   shortcutting.** This is a hard rule. Rust is mechanism only. If a piece has
   product behavior a user could plausibly want to change — including the
   interactive frontend that carries visual parity — it ships as embedded
   `.lua` registered through the same API user extensions use. The Rust
   substrate provides what a Lua table cannot express: the runtime, OS and
   terminal bindings, provider transport, and persistence. Placement calls are
   recorded in the locked-decisions table; the default is Lua.
3. **The mechanism surface may exceed Pi's implementation API.** Rust bindings
   may expose the mechanisms needed to author the product in Lua, but shipped
   product behavior must remain Pi-compatible. Product experiments belong in a
   downstream fork.

## Product boundary

pi-rs ports only what is needed to reproduce Pi's coding-agent product:

- **AI and auth:** model types, provider protocols, streaming, credentials,
  OAuth, model discovery, and the provider behavior the coding agent exposes;
- **agent runtime:** the message/tool loop, steering, follow-ups, cancellation,
  usage, and state required by the coding agent;
- **coding agent:** CLI modes, interactive terminal UI, tools, sessions,
  settings, prompts, themes, commands, extensions, and other user-visible
  behavior;
- **terminal UI:** the rendering and input machinery required to make the
  coding agent visually and interactively identical to Pi.

Explicitly out of scope:

- Pi products other than the coding agent — the Discord bot and any unrelated
  application, integration, or demo;
- a general-purpose agent framework whose abstractions delay parity;
- pi-rs-specific UI, branding, chrome, commands, defaults, or workflows;
- downstream product behavior;
- module-for-module source translation where behavioral equivalence is better
  expressed idiomatically in Rust.

Out-of-scope code need not be ported, and existing code or docs outside this
boundary may be deleted when doing so simplifies the port. Git is the attic;
the pre-port trees are preserved on `main` (`fd373e0`) and `rebuild`
(`e8cb418`).

## Parity contract

`ref/pi/packages/coding-agent` is the product-level specification; its required
implementation dependencies are `ref/pi/packages/ai`, `agent`, and `tui`. When
documentation, tests, or pi-rs's current behavior disagree with the reference,
the reference wins unless a divergence is explicitly locked.

Parity has four inseparable dimensions:

1. **Visual:** identical terminal cells for stable frames — text, whitespace,
   borders, color, attributes, cursor, wrapping, clipping, and ordering.
2. **Interactive:** identical key handling, editor behavior, focus, dialogs,
   command flow, cancellation, and resize behavior.
3. **Behavioral:** identical requests, tool semantics, streaming transitions,
   errors, settings, sessions, and observable CLI behavior.
4. **Compatibility:** the same relevant configuration, credentials, themes,
   model/provider choices, session data, and extension use cases.

"Usable," "similar," and "inspired by Pi" are not acceptance criteria.
Temporary labels or placeholder components — such as `you:` and `assistant:`
transcript prefixes that Pi does not render — are bugs, even if a pi-rs-only test
expects them. Tests are corrected to the reference, never the reverse.

## Doctrine conformance

| Doctrine | Status | Notes |
|---|---|---|
| 01 extension-first core | follows (strengthened) | Divergence 2 is The Rule with the escape hatch removed: first-party product behavior may not shortcut through Rust. |
| 02 snapshot in, actions out | follows | Events in as tables, results out as tables; per-dispatch watchdog bounding *continuous* Lua execution (every host await resets the window — long-lived loops accumulate unbounded total time, busy loops still die). Async seam: handlers are coroutines that may await host futures (locked below), and `pi.spawn` starts background coroutines scoped to their dispatch — still no live `&mut` host references. |
| 03 daemon + thin client | deferred | Applies to downstream products, not the compatibility port. |
| 04 declarative front, idempotent executor | n/a | No system-config surface. |
| 05 one declaration mechanism | follows | Every unit of a kind is declared via `register_*` / `on(event)`, per Pi's vocabulary. |
| 06 bare core must boot | follows | Bare = substrate with zero packs: `pi --login`, `pi --list-models`, `pi "prompt"` streaming a raw completion. CI `bare-boot` check. |
| 07 nix source of truth | follows | crane flake; `cargo fmt`/`clippy` sanctioned exceptions. |

## Locked decisions

| Decision | Choice | Rationale |
|---|---|---|
| Spec | Pi v0.79.0, `ref/pi` @ `c5582102`, frozen | Port a snapshot, then deliberately adopt upstream changes with recorded parity evidence. |
| Runtime identity | Binary `pi`; app name `pi`; global config `~/.pi/agent`; project config `.pi/`; `PI_CODING_AGENT_*` overrides | pi-rs is a drop-in implementation and reads existing Pi configuration. Repository and Rust crate names remain `pi-rs*` so implementation identity does not leak into product behavior. |
| Extension language | Lua (divergence 1) | JS runtime considered for verbatim example reuse; rejected — embed weight, and translation-as-conformance is judged worth the cost. Final. |
| Mechanism/policy line | Rust substrate = Lua runtime (registries, event bus, watchdog), provider transport (HTTP/SSE/OAuth/streaming), terminal mechanism (raw mode, differential cell renderer, input decoding, components) exposed as bindings, OS bindings, JSONL persistence. Lua = tools, agent loop orchestration, slash commands, frontend/chrome, compaction policy, skills, prompt templates, themes, session naming — everything with product behavior. | Divergence 2 made mechanical. A placement that puts behavior in Rust needs a row here. |
| Async seam | Lua handlers run as coroutines and may await host futures (LLM streams, subprocesses, timers) | Required for the loop and frontend to be Lua-authored. Settled early because retrofitting it is the expensive mistake. |
| Workspace layout | `pi-rs-ai{,-types,-auth}` ← `packages/ai`; `pi-rs-agent` ← `packages/agent`; `pi-rs-tui` ← `packages/tui`; `pi-rs-app` ← `packages/coding-agent`; `pi-rs-host` ← `core/extensions`; `pi-rs-session` ← `core/session-manager` | Crate granularity may differ from package granularity; parity is judged by behavior (fixtures and frames), not module diffs. |
| `pi-rs-ai` structure | Layered compression, not Pi-mirrored: `types → auth → transport (written once) → protocols (wire mapping only) → registry`. ~5 wire protocols as trait impls; everything else is a catalog data row, zero Rust. Model catalog is data (`data/models.json`), never hand code. OAuth = PKCE engine + device-code engine + flows-as-data; irreducibly weird flows stay code sharing the machinery. | Pi's per-provider file sprawl is the thing being compressed. Parity contract: catalog diff vs Pi's registry + recorded-fixture replay. |
| Provider/auth scope | Catalog and provider surface match Pi's coding agent. Interactive `/login` OAuth: Claude Code (Anthropic) and Codex flows for now; API keys everywhere else. Remaining flows land with the auth-compatibility milestone. | The two flows cover daily driving; the engines already generalize. |
| Diff algorithm | jsdiff 8.0.4 ported 1:1 as a Rust mechanism binding (`pi.diff.lines/words/unified_patch`), pinned by an oracle generated from the vendored library | Pi consumes jsdiff as a third-party library; the algorithm is cross-cutting mechanism (written once), while what to diff and how to present it stays Lua (`edit-diff.lua`, diff rendering). |
| Image processing | photon 0.3.4 (pi's `@silvia-odwyer/photon-node` WASM build) ported as a Rust mechanism binding on the jsdiff split: the slice pi uses — `resizeImageInProcess`, `convertToPng`, EXIF orientation — over the exact dependency stack the WASM was compiled with (`image` =0.24.9, `png` 0.17.14, `flate2` 1.0.34, `miniz_oxide` 0.8.0, jpeg-decoder 0.3.1 `platform_independent`), pinned byte-for-byte by a vendored-library oracle (`tests/image-parity`). What to read/note/attach stays Lua (`read.lua`, `messages.lua`, `interactive.lua`). | Same split as jsdiff/hljs: the library is third-party mechanism; presentation and wiring are Lua. Encoded image bytes reach provider requests, so "equivalent requests" requires encoder-level parity. |
| Syntax highlighting | highlight.js 10.7.3 ported as a Rust mechanism binding (`pi.hljs.*`): the parse engine interprets grammar *data* compiled by the vendored library itself (`scripts/hljs-grammars` → `crates/pi-rs-host/data/hljs-grammars.json`), pinned by a vendored-library oracle. Pi's own layer — `renderHighlightedHtml`, theme mapping, `highlightCode` policy — stays Lua. Grammar scope: the `getLanguageFromPath` targets + Pi-pinned fence tags, closed over sublanguages (41 languages); other fence tags take the unvalidated-language fallback where Pi (191 grammars) would highlight — boundary recorded in PLAN 2b.3, widened by regenerating the catalog. | Same split as jsdiff: the library is third-party mechanism, grammars are catalog data (never hand code), presentation is Lua. |
| Code standard | Typed errors per layer (`thiserror`); no `unwrap`/`expect`/`panic!` in library crates (clippy `deny`); cross-cutting mechanisms (retry, SSE decode, truncation, cancellation, rendering) written exactly once; behavior pinned by golden fixtures before refactors touch it. | "Clean" must be checkable in CI or it erodes. |
| Exerciser rule | Softened: every new public extension hook *should* land with an example in `examples/`, which doubles as the conformance suite and docs. Skip only when the cost clearly outweighs the coverage, and say so in the commit. | Keeps the surface honest without taxing every commit. |
| Shipped defaults | Embedded `.lua` via `include_str!`, loaded through the public API | Divergence 2. The flake source filter must include every embedded file type. |
| Sandboxing / permissions | Pi's stance: none in core, full user permissions; trust gates extension loading only | Written down so plain-Lua `os`/`io` access is a choice, not an accident. |
| Extension distribution | Pi installs from npm; pi-rs adapts to Lua distribution | Mechanism undecided — decide when the port reaches `package-manager.ts`, not before. |
| Ablation | Code, docs, tests, and fixtures outside the product boundary or superseded by a port may be deleted freely | Git is the attic. Carrying dead weight costs more than resurrection. |

Any new product-visible divergence requires a row here, a concrete necessity,
and a test. Convenience, incomplete implementation, or taste is not sufficient.

## Architecture

```text
crates/
  pi-rs-ai-types   shared model/message/content/usage/stream-event types  (mechanism)
  pi-rs-ai-auth    credential storage, PKCE + device-code OAuth engines   (mechanism)
  pi-rs-ai         transport, wire protocols, streaming, model registry   (mechanism)
  pi-rs-agent      agent-loop primitives and event/state vocabulary       (mechanism)
  pi-rs-session    JSONL session persistence and reconstruction           (mechanism)
  pi-rs-tui        cells, differential renderer, input decoding, editor,
                 components — exposed as Lua bindings                   (mechanism)
  pi-rs-host       Lua runtime, registries, event bus, watchdog, trust,
                 OS bindings                                            (mechanism)
  pi-rs-app        thin binary: cli args, config, mode selection          (mechanism)
    src/builtins/*.lua   tools, agent loop, interactive frontend, slash
                         commands, themes, compaction, skills           (policy)
examples/        public-surface exercisers — the conformance suite
```

Dependencies point toward mechanisms: AI and agent crates must not depend on
the application or terminal frontend. Shared transport, cancellation, retry,
SSE decoding, and rendering mechanisms each have exactly one implementation.

## Acceptance

The standing product test is Pi and pi-rs in equivalent adjacent terminals,
driven with the same inputs and deterministic provider/tool fixtures. Stable
frames must match cell-for-cell, and the sequence of observable states and
accepted inputs must match.

Automated acceptance includes:

- differential terminal-frame snapshots derived from Pi, including cursor
  state (`scripts/ui-diff`);
- replay of the same provider streams and tool outcomes through Pi and pi-rs;
- key/input scripts covering editing, dialogs, commands, streaming, abort, and
  resize;
- compatibility fixtures for auth, model selection, settings, and sessions;
- focused protocol fixtures plus `cargo test --workspace` and the Nix checks.

A work item is complete only when its exposed behavior is compared with Pi. A
pi-rs-only unit test can protect implementation details but cannot establish
parity.

## Delivery order

The shortest non-throwaway path to an indistinguishable interactive coding
agent:

1. differential frame/input harness against the pinned Pi snapshot;
2. exact transcript/editor layout on the final Lua frontend;
3. one provider and auth path end-to-end through the exact agent loop;
4. every interactive state reachable in normal coding-agent use;
5. close coding-agent CLI, session, settings, extension, and provider gaps;
6. final surface inventory against the pinned reference;
7. maintain parity and deliberately port selected upstream Pi changes.

Product-specific work begins from a parity tag as a separate downstream fork.
The extension-first and Lua-policy boundaries remain useful there, but its
roadmap does not live in this repository.

Provider breadth can proceed after one provider exercises the complete
product, but visual and interaction parity are never deferred as polish.
