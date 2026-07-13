# pi-rs

pi-rs is a minimal, high-performance Rust coding harness with a Lua-authored
product. The installed executable is `pi`. Rust supplies generic terminal, OS,
provider/auth, and persistence mechanisms; ordinary Lua 5.4 packages define the
application, agent, frontend, tools, configuration, and optional sessions.

<p align="center">
  <img src="demo/pi-rs.gif" alt="pi-rs terminal demo" width="900">
</p>

`main` is being rebuilt around this contract. It is not yet a release claim.
The former faithful port is preserved on `pi-rust-rewrite` as historical
provenance, not as the specification for current work.

## Product boundary

The shipped defaults aim for Pi's restrained coding experience in a compact,
checked set of terminal states: editing, streaming/thinking, tool presentation,
queueing/cancellation, representative dialogs/selectors, and session resume.
Those named grids and input journeys may be exact. Behavior outside that set is
pi-rs behavior.

One compatibility boundary is deliberately exhaustive: provider and
authentication mechanisms retain pinned Pi v0.79.0 parity for the supported
model catalog, provider wire protocols, API-key resolution, and Anthropic,
GitHub Copilot, and OpenAI/Codex subscription OAuth. This does **not** imply
whole-product, CLI, session, configuration, package, or TypeScript-extension
compatibility.

See [`DESIGN.md`](DESIGN.md) for the normative boundary and
[`PLAN.md`](PLAN.md) for the ordered implementation gates.

## Architecture

- **Rust = mechanism:** Lua runtime/package loading, watchdogs, immutable
  snapshots, validated action/effect execution, terminal/display primitives,
  async OS operations, provider/auth engines, and a generic durable record
  store.
- **Lua = product policy:** application/agent/frontend/session state machines,
  tools, commands, editor and transcript behavior, themes, keymaps,
  configuration, provider selection, and resource discovery.
- **No privileged builtins:** embedded defaults use the same public modules,
  declarations, capabilities, lifecycle, and watchdogs as file-backed packages.
- **Snapshots in, actions out:** Lua never borrows mutable host state. Rust may
  batch, lay out, clip, and diff Lua-authored display structures, but does not
  choose product appearance or workflow.
- **Bare core boots:** with shipped policy removed, `pi` can still load an
  ordinary file-backed Lua application, accept input, render, run an effect,
  and exit cleanly.

Persistent conversation sessions are optional Lua policy over arbitrary
versioned records. The default session package can be disabled or replaced; an
ephemeral application remains useful without it.

## Storage

pi-rs writes only to XDG roots:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/pi
${XDG_DATA_HOME:-$HOME/.local/share}/pi
${XDG_STATE_HOME:-$HOME/.local/state}/pi
${XDG_CACHE_HOME:-$HOME/.cache}/pi
```

A resource under `~/.pi/agent` may be read only when its corresponding XDG
resource is absent. Canonical and legacy copies are never merged. Every write
targets XDG; pi-rs never rewrites, deletes, or silently migrates a legacy file.

## Performance

Release claims are benchmarked rather than inferred from Rust. The checked
release harness measures startup to an input-ready frame, idle RSS,
input-to-frame latency, retained render cost, Lua dispatch/copy overhead, effect
round trips, release size, and resource cleanup. Numeric budgets are recorded
from the reproducible baseline in PLAN 0.2 and enforced through Nix.

## Build and verify

Nix is the source of truth:

```sh
nix build
nix flake check
nix run
```

`cargo fmt` and `cargo clippy` are sanctioned direct iteration tools. Focused
native tests may aid development, but completion and release claims use the
flake.

The pinned Pi checkout at `ref/pi` commit
`c5582102f51b143fadc05180e0f8aed050e923b3` is needed only to regenerate or
review the named canonical-experience and provider/auth fixtures. It is not a
whole-product oracle.
