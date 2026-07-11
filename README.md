# pi-rs

[Pi](https://github.com/badlogic/pi-mono)'s coding agent, ported to Rust.
The installed executable is `pi` and it uses Pi's runtime identity, including
`~/.pi/agent`, project-local `.pi/`, and `PI_CODING_AGENT_*` overrides.

The spec is Pi **v0.79.0**, with the development oracle kept in the ignored
`ref/pi` checkout at commit `c5582102`. The goal is strict visual, behavioral,
and configuration parity. Product experiments belong in downstream forks
rather than this compatibility port. `DESIGN.md` records the divergences and
architecture; `PLAN.md` is the ordered parity ladder.

First-party product behavior (tools, agent loop, interactive frontend, themes)
ships as embedded Lua over public bindings; Rust provides mechanism only:
runtime, provider transport, terminal, OS, and persistence.

Build and test:

```sh
cargo test --workspace
nix flake check
```
