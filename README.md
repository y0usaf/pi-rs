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

## Updating the built-in model catalog

The runtime reads only the reviewed snapshot in
`crates/pi-rs-ai/data/models.json`; it never fetches model metadata. Refresh it
from upstream with:

```sh
nix run .#update-model-catalog
```

For reproducible review or offline regeneration, pin an upstream revision or
use a local checkout:

```sh
nix run .#update-model-catalog -- --revision <git-revision>
nix run .#update-model-catalog -- --source ref/pi --revision c5582102f51b143fadc05180e0f8aed050e923b3
```

The command also updates `models.provenance.json` with the exact revision,
catalog/output hashes, API inventory, and provider/model counts. Reviewed
upstream metadata corrections belong in `scripts/model-catalog-overrides.json`
and require a reason; runtime code must not special-case catalog rows.

Generation rejects unknown model fields, duplicate IDs, and API protocols
outside the reviewed vocabulary. A new protocol is promoted deliberately:
implement and replay-test its transport first, add it to the updater's accepted
API set, then regenerate and pass `nix flake check`. The scheduled
`model-catalog-update.yml` workflow follows the same path and opens a generated
PR only when the reviewed snapshot changes.
