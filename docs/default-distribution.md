# Default distribution

The default `pi` is the raw launcher plus one declarative manifest. Nothing is
embedded in the binary, concatenated, or granted a private module: the shipped
files are ordinary Lua packages loaded through the same package transaction as
a file copied into a scratch directory.

## The manifest

`crates/pi-rs-builtins/default.json` is a version 1 launcher manifest:

```json
{ "version": 1, "packages": ["agent/queue.lua", "…", "defaults/init.lua"] }
```

Package paths resolve relative to the manifest's own directory, so the same
manifest works in the repository, in the Nix store, and in a user's copy. Load
order is the manifest's order: agent modules, the tool suite, the frontend
components, the application coordinator, then distribution defaults.

`crates/pi-rs-app/tests/default_distribution.rs` fails if the manifest and the
shipped package trees ever disagree, so a new module cannot land unindexed.

## Selection precedence

1. `--package FILE` (repeatable) — explicit packages, loaded after any manifest.
2. `--manifest FILE` — an explicit manifest.
3. `PI_PACKAGE_MANIFEST` — the environment selection.
4. Nothing selected — the raw launcher prints guidance and exits cleanly
   (`nix run .#pi-core`).

The Nix wrapper sets `PI_PACKAGE_MANIFEST` with `--set-default`, so every
explicit selection above still wins. `nix run .#pi-core` remains the unwrapped
zero-builtin target.

## Distribution defaults

`crates/pi-rs-builtins/defaults/init.lua` is the only policy the distribution
adds on top of the agent, tool, and frontend packages. It holds two decisions,
both as public application event middleware:

- `pi.builtins.defaults.model` — injects the first catalog model from a
  declared candidate list (`anthropic/claude-sonnet-4-5`, then
  `openai/gpt-5.1`, then `openrouter/anthropic/claude-sonnet-4.5`) into the
  startup event when it carries no model. Credentials are never read here:
  `pi.models.v1.stream` resolves the provider's supported credential itself, so
  a live session needs only a working key, and a missing or rejected one
  settles as an agent error that the frontend renders as credential guidance.
- `pi.builtins.defaults.tool-root` — re-declares the shipped tool suite with
  `root = snapshot.context.root` on the first application dispatch, so relative
  tool paths resolve against the launcher root rather than the process
  directory. (This closes the open question from the tool package.)

Both stages are replaceable: register the same `kind`/`phase`/`id` from your
own package, configure another model with a `configure` event, or drop the
defaults package from a copied manifest.

## Replacing the distribution

```sh
cp -r "$(nix build --print-out-paths .#pi-rs)/share/pi/packages" ./my-pi
$EDITOR my-pi/defaults/init.lua
pi --manifest my-pi/default.json
```

The copied tree behaves identically to the shipped one — an assertion in the
distribution test, not a promise.
