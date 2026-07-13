# pi-rs — Pi's coding agent in Rust

pi-rs is a faithful Rust port of [Pi](https://github.com/badlogic/pi-mono)'s
coding agent. The vendored Pi v0.79.0 snapshot at `ref/pi` (commit `c5582102`)
is the product specification for everything the coding agent does. Where this
document and the spec disagree, the spec wins unless the difference appears in
the exhaustive list below. A locked-decision row alone cannot authorize a
product-visible difference.

Three commitments:

1. **Parity outside the exception list.** Given the same terminal,
   credentials, model, input, provider/tool responses, and equivalent Lua
   configuration, a user must not be able to distinguish the parity release
   from Pi's coding agent: rendering, input behavior, commands, errors,
   requests, persistence, and session behavior all match.
2. **Compatibility maintenance.** pi-rs remains the faithful port. Custom
   products and behavior start as downstream forks after the parity gate rather
   than turning this repository into a dialect.
3. **Extension-first supersurface.** The shipped product is assembled from
   independently replaceable Lua units using only public capabilities available
   to user extensions. The authoring surface is a strict capability superset of
   Pi's pinned extension surface, proven both by the shipped product and by a
   maintained external-extension dogfood suite. Additive authoring capability
   may exceed Pi; default product behavior may not.

Rust is the implementation language. A different implementation is not a
license to invent a different product. The port is still in progress; parity
is a release gate, not a claim that unfinished milestones already conform.

## Exhaustive differences from Pi

Everything not listed here must look, feel, and behave **1:1 with Pi's coding
agent**. This is a closed exception list, not examples of permitted drift.

1. **Rust implementation.** Pi is TypeScript; pi-rs is Rust. Implementation
   details are allowed to differ, but this item authorizes no observable
   difference.
2. **Lua-only configuration.** Global `~/.pi/agent/config.lua` and project
   `.pi/config.lua` are the sole user configuration entry points. They replace
   Pi's JSON configuration files. Settings, keybindings, models/providers,
   themes, resource selection, and extension declarations all go through Lua.
   Pi's configuration UI and commands must produce the same effective outcome,
   persisted as Lua. Formats that carry user data/content rather than
   configuration—sessions, credentials, project instructions, skills, and
   prompt-template content—remain Pi-compatible.
3. **Lua extensions.** Extensions use Lua (mlua, vendored 5.4), not
   TypeScript/JavaScript. Pi's extension examples arrive as translations, not
   copy-paste—and each translation is a conformance test of the bridge. A Pi
   coding-agent example that cannot translate is a bridge bug, not an example
   we skip.
4. **Lua package contents.** Packages distribute Lua configuration/extensions,
   modules, and data rather than TypeScript/JavaScript. Transport and package
   command behavior remain Pi-compatible: npm registry archives, Git URLs/refs,
   and local paths. `package.json` is inert distribution/manifest metadata, not
   a configuration entry point or executable extension source.
5. **Everything first-party is independently replaceable Lua on the public
   extension surface—no Rust or monolithic-Lua shortcutting.** Rust is mechanism
   only. If a piece has product behavior a user could plausibly want to change—
   including tools, agent policy, commands, compaction, transcript renderers,
   status areas, and the interactive frontend—it is a declared unit in the
   builtins layer, registered through the same API user extensions use, and can
   be disabled and replaced without changing Rust. Merely placing a feature in
   a large embedded Lua chunk does not satisfy this rule when the feature can be
   an extension of its own. The Rust substrate provides what a Lua table cannot
   express: runtime, OS and terminal bindings, provider transport, and
   persistence. Placement calls are recorded in the locked-decisions table;
   the default is a public Lua declaration.
6. **The public Lua mechanism surface exceeds Pi's implementation API.** Rust
   bindings must expose, to builtins and user extensions alike, the low-level
   capabilities needed to author the product and the maintained extension
   dogfood suite. This is capability equivalence, not Node API emulation: Lua-
   native HTTP, process, lifecycle, filesystem, network, crypto, rendering,
   agent, and session mechanisms may use different names and shapes. Additive
   authoring capability may not alter shipped Pi-compatible behavior. Product
   experiments still belong in a downstream fork.

Any consequence of these differences is permitted only where unavoidable. A
new exception requires an explicit addition here, a concrete necessity, and a
test before release. Convenience, incomplete implementation, and taste are not
exceptions.

## Product boundary

pi-rs ports Pi's coding-agent product plus the bounded public extension
platform used to construct and independently replace it:

- **AI and auth:** model types, provider protocols, streaming, credentials,
  OAuth, model discovery, and the provider behavior the coding agent exposes;
- **agent runtime:** the message/tool loop, steering, follow-ups, cancellation,
  usage, and state required by the coding agent;
- **coding agent:** CLI modes, interactive terminal UI, tools, sessions,
  settings, prompts, themes, commands, extensions, and other user-visible
  behavior;
- **terminal UI:** the rendering and input machinery required to make the
  coding agent visually and interactively identical to Pi;
- **extension platform:** the complete pinned Pi extension contract, the public
  mechanisms used to assemble every first-party policy unit, and the bounded
  capability superset exercised by the maintained extension dogfood suite.

Explicitly out of scope:

- Pi products other than the coding agent — the Discord bot and any unrelated
  application, integration, or demo;
- a general-purpose agent framework whose abstractions are exercised by
  neither the shipped builtins, pinned Pi examples, nor the maintained dogfood
  suite and would delay parity;
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
the reference wins unless the difference is in the exhaustive list above.

Parity has four inseparable dimensions outside the exhaustive exception list:

1. **Visual:** identical terminal cells for stable frames—text, whitespace,
   borders, color, attributes, cursor, wrapping, clipping, and ordering.
2. **Interactive:** identical key handling, editor behavior, focus, dialogs,
   command flow, cancellation, and resize behavior.
3. **Behavioral:** identical requests, tool semantics, streaming transitions,
   errors, effective settings, sessions, and observable CLI behavior.
4. **Data compatibility:** the same relevant credentials, session data,
   project instructions, skills, and prompt-template content. Configuration
   source files and extension/package source code are the explicit Lua
   exceptions above; equivalent Lua configuration must produce Pi's outcome.

"Usable," "similar," and "inspired by Pi" are not acceptance criteria.
Temporary labels or placeholder components—such as `you:` and `assistant:`
transcript prefixes that Pi does not render—are bugs, even if a pi-rs-only test
expects them. Tests are corrected to the reference, never the reverse.

## Doctrine conformance

| Doctrine | Status | Notes |
|---|---|---|
| 01 extension-first core | follows (strengthened) | Difference 5 is The Rule with both escape hatches removed: first-party behavior may shortcut through neither Rust nor an indivisible embedded-Lua monolith. Every replaceable unit lives in the builtins layer, uses the public surface, and has ablation/replacement evidence. |
| 02 snapshot in, actions out | follows | Events in as tables, results out as tables; per-dispatch watchdog bounding *continuous* Lua execution (every host await resets the window — long-lived loops accumulate unbounded total time, busy loops still die). Async seam: handlers are coroutines that may await host futures (locked below), and `pi.spawn` starts background coroutines scoped to their dispatch — still no live `&mut` host references. |
| 03 daemon + thin client | deferred | Applies to downstream products, not the compatibility port. |
| 04 declarative front, idempotent executor | n/a | No system-config surface. |
| 05 one declaration mechanism | follows | Every unit of a kind—including applications/frontends, render middleware, lifecycle resources, commands, tools, and shipped defaults—is declared through one public mechanism; source names and hardcoded launcher branches are not declaration mechanisms. |
| 06 bare core must boot | follows | Bare = substrate with zero packs: `pi --login`, `pi --list-models`, `pi "prompt"` streaming a raw completion. CI also ablates each builtin pack and replaces representative policy units with ordinary file-backed extensions. |
| 07 nix source of truth | follows | crane flake; `cargo fmt`/`clippy` sanctioned exceptions. |

## Locked decisions

| Decision | Choice | Rationale |
|---|---|---|
| Spec | Pi v0.79.0, `ref/pi` @ `c5582102`, frozen | Port a snapshot, then deliberately adopt upstream changes with recorded parity evidence. |
| Runtime identity | Binary `pi`; app name `pi`; global root `~/.pi/agent`; project root `.pi/`; `PI_CODING_AGENT_*` overrides | Product identity stays drop-in. Compatible data remains shared; config source differs explicitly: `config.lua`, not Pi's JSON configuration files. Repository and Rust crate names remain `pi-rs*` so implementation identity does not leak into product behavior. |
| Configuration language | Lua only: `~/.pi/agent/config.lua` + `.pi/config.lua` | Restores Phi's one-language configuration model. All configurable declarations pass through the public Lua surface; interactive mutations persist as Lua. No `settings.json`, `keybindings.json`, `models.json`, or theme JSON compatibility promise. Final. |
| Extension language | Lua (difference 3) | JS runtime considered for verbatim example reuse; rejected—embed weight, and translation-as-conformance is judged worth the cost. Final. |
| Mechanism/policy line | Rust substrate = Lua runtime (registries, event bus, watchdog), provider transport (HTTP/SSE/OAuth/streaming), terminal mechanism (raw mode, differential cell renderer, input decoding, components) exposed as bindings, OS bindings, and persistence. Lua = configuration, tools, agent loop orchestration, slash commands, frontend/chrome, compaction policy, resource discovery/registration, themes, and session naming—everything with product behavior. Skill and prompt-template content retains Pi's formats. | Differences 2 and 5 made mechanical. A placement that puts behavior in Rust needs a row here. |
| Async seam | Lua handlers run as coroutines and may await host futures (LLM streams, subprocesses, timers); dispatch-scoped work and runtime/session-scoped resources are distinct public lifetimes | Required for the loop, frontend, and long-lived user extensions to be Lua-authored without leaked tasks. Runtime/session resources have explicit cancellation and disposal, never implicit access to mutable frontend state. |
| Workspace layout | `pi-rs-ai{,-types,-auth}` ← `packages/ai`; `pi-rs-agent` ← `packages/agent`; `pi-rs-tui` ← `packages/tui`; `pi-rs-app` ← `packages/coding-agent`; `pi-rs-host` ← `core/extensions`; `pi-rs-session` ← `core/session-manager` | Crate granularity may differ from package granularity; parity is judged by behavior (fixtures and frames), not module diffs. |
| `pi-rs-ai` structure | Layered compression, not Pi-mirrored: `types → auth → transport (written once) → protocols (wire mapping only) → registry`. ~5 wire protocols as trait impls; everything else is a catalog data row, zero Rust. Model catalog is data (`data/models.json`), never hand code. OAuth = PKCE engine + device-code engine + flows-as-data; irreducibly weird flows stay code sharing the machinery. | Pi's per-provider file sprawl is the thing being compressed. Parity contract: catalog diff vs Pi's registry + recorded-fixture replay. |
| Provider/auth scope | Catalog and provider surface match Pi's coding agent. Interactive `/login` OAuth includes every pinned subscription provider: Claude Pro/Max (Anthropic), GitHub Copilot, and ChatGPT Plus/Pro (Codex); API keys everywhere else. | One PKCE engine + one RFC 8628 device-code engine are shared; Codex and Copilot retain only their irreducibly distinct exchanges/model setup. |
| Diff algorithm | jsdiff 8.0.4 ported 1:1 as a Rust mechanism binding (`pi.diff.lines/words/unified_patch`), pinned by an oracle generated from the vendored library | Pi consumes jsdiff as a third-party library; the algorithm is cross-cutting mechanism (written once), while what to diff and how to present it stays Lua (`edit-diff.lua`, diff rendering). |
| Image processing | photon 0.3.4 (pi's `@silvia-odwyer/photon-node` WASM build) ported as a Rust mechanism binding on the jsdiff split: the slice pi uses — `resizeImageInProcess`, `convertToPng`, EXIF orientation — over the exact dependency stack the WASM was compiled with (`image` =0.24.9, `png` 0.17.14, `flate2` 1.0.34, `miniz_oxide` 0.8.0, jpeg-decoder 0.3.1 `platform_independent`), pinned byte-for-byte by a vendored-library oracle (`tests/image-parity`). What to read/note/attach stays Lua (`read.lua`, `messages.lua`, `interactive.lua`). | Same split as jsdiff/hljs: the library is third-party mechanism; presentation and wiring are Lua. Encoded image bytes reach provider requests, so "equivalent requests" requires encoder-level parity. |
| Syntax highlighting | highlight.js 10.7.3 ported as a Rust mechanism binding (`pi.hljs.*`): the parse engine interprets grammar *data* compiled by the vendored library itself (`scripts/hljs-grammars` → `crates/pi-rs-host/data/hljs-grammars.json`), pinned by a vendored-library oracle. Pi's own layer—`renderHighlightedHtml`, theme mapping, `highlightCode` policy—stays Lua. Current milestone scope is the `getLanguageFromPath` targets + Pi-pinned fence tags, closed over sublanguages (41 languages); the final parity audit must widen this to Pi's full reachable behavior. The current fallback difference is unfinished work, not a release exception. | Same split as jsdiff: the library is third-party mechanism, grammars are catalog data (never hand code), presentation is Lua. |
| Code standard | Typed errors per layer (`thiserror`); no `unwrap`/`expect`/`panic!` in library crates (clippy `deny`); cross-cutting mechanisms (retry, SSE decode, truncation, cancellation, rendering) written exactly once; behavior pinned by golden fixtures before refactors touch it. | "Clean" must be checkable in CI or it erodes. |
| Exerciser rule | Every new public authoring capability lands with a file-backed user-extension exerciser unless it is covered by a translated pinned example or dogfood extension; any exception is recorded in the commit and inventory | The surface must be proven from outside the builtins layer, not only by code loaded from synthetic sources. |
| First-party assembly | A declarative builtins manifest selects independently disableable Lua packages and generic application roles; Rust launches a role from the public registry and never names a product command such as `pi-rs-interactive` | Loading policy is bootstrap mechanism; the identity, composition, and behavior of the default product remain replaceable Lua policy. Source identity is provenance only and grants no semantic privilege. |
| Extension capability target | Strict superset of pinned Pi v0.79.0 plus a checked capability manifest derived initially from `pi-flake` commit `94694da7321ce74aa7b82c13db7e60e28c0caba6` (15 extensions, hosted there by Pi 0.80.6) | The dogfood revision is an authoring-surface oracle, not a promotion of the product behavior spec. Required additive mechanisms may land now; newer product-visible behavior requires a deliberate Pi spec promotion. CI consumes checked manifests, Lua translations, and deterministic fixtures, never an ambient sibling checkout. |
| Shipped defaults | Embedded Lua packages via `include_str!`, declared in the builtins manifest and loaded through the public package/registration API | Difference 5. The flake source filter must include every embedded asset type. Each package is independently ablatable and replaceable; concatenation for embedding may not hide a private composition API. |
| Sandboxing / permissions | Pi's stance: none in core, full user permissions; trust gates extension loading only | Written down so plain-Lua `os`/`io` access is a choice, not an accident. |
| Extension distribution | Pi-compatible npm (`npm:` registry archives), Git URL/ref, and local-path transport; package contents are Lua configuration/extensions/modules/data. The existing `package.json` `pi` manifest remains inert package metadata and points at Lua resources. Package policy may invoke npm/git through public process/filesystem mechanisms, but pi-rs never evaluates package JavaScript or exposes Node module resolution; pure-Lua dependencies are packaged or vendored. | This preserves the pinned `package-manager.ts` source grammar, project/user install roots, identity/dedupe, offline cache behavior, and install/remove/list/update/config outcomes while satisfying differences 2–4. npm is retained as an archive registry and lifecycle transport, not as the extension runtime. Git/local alone would make existing `npm:` package commands observably incompatible and require a broader exhaustive exception. |
| Ablation | Code, docs, tests, and fixtures outside the product boundary or superseded by a port may be deleted freely | Git is the attic. Carrying dead weight costs more than resurrection. |

The exhaustive list above is the sole authority for product-visible differences.

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
  pi-rs-app        thin binary: cli args, generic role selection            (mechanism)
    src/builtins/       declarative manifest + independently replaceable
                        Lua packages: tools, agent, frontend, commands,
                        render policy, themes, compaction, skills           (policy)
examples/        file-backed public-surface exercisers — conformance suite
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
- a generated first-party construction inventory mapping every policy unit to
  its public declaration, owning builtin package, disable path, and replacement
  test;
- bare-core, per-package ablation, and file-backed replacement tests proving
  that synthetic source identity grants no capability;
- a checked external-extension capability manifest and executable Lua
  translations of the maintained dogfood suite, including its long-lived
  process/network and global-render-composition cases;
- focused protocol fixtures plus `cargo test --workspace` and the Nix checks.

A parity work item is complete only when its exposed product behavior is
compared with Pi. An additive authoring-surface item is complete only when a
file-backed consumer exercises its public contract and the unchanged default
product still passes the Pi differential suites. A pi-rs-only unit test can
protect implementation details but establishes neither parity nor public
availability.

## Delivery order

The shortest non-throwaway path to an indistinguishable interactive coding
agent:

1. differential frame/input harness against the pinned Pi snapshot;
2. exact transcript/editor layout on the final Lua frontend;
3. one provider and auth path end-to-end through the exact agent loop;
4. every interactive state reachable in normal coding-agent use;
5. close coding-agent CLI, session, settings, extension, and provider gaps;
6. replace hardwired first-party assembly with a declarative builtins layer and
   close the construction, ablation, and replacement inventories;
7. close the additive mechanism supersurface against the maintained dogfood
   suite without changing default Pi behavior;
8. final parity and authoring-surface audits;
9. maintain parity and deliberately port selected upstream Pi changes.

Product-specific work begins from a parity tag as a separate downstream fork.
The extension-first and Lua-policy boundaries remain useful there, but its
roadmap does not live in this repository.

Provider breadth can proceed after one provider exercises the complete
product, but visual and interaction parity are never deferred as polish.
