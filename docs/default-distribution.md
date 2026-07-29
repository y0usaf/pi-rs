# Default distribution

The default `pi` is the raw launcher plus one declarative manifest. Nothing is
embedded in the binary, concatenated, or granted a private module: the shipped
files are ordinary Lua packages loaded through the same package transaction as
a file copied into a scratch directory.

## The manifest

`crates/pi-rs-builtins/default.json` is a version 1 launcher manifest:

```json
{ "version": 1, "packages": ["config/json.lua", "…", "defaults/init.lua"] }
```

Package paths resolve relative to the manifest's own directory, so the same
manifest works in the repository, in the Nix store, and in a user's copy. Load
order is the manifest's order: the configuration package, agent modules, the
tool suite, the frontend components, the application coordinator, the session
package, then distribution defaults. A module is listed after the modules it
requires; nothing else in load order is significant, because what runs when is
decided by middleware `order`, not by position in the index.

`crates/pi-rs-app/tests/default_distribution.rs` fails if the manifest and the
shipped package trees ever disagree, so a new module cannot land unindexed.

## Stage order

Five shipped stages compose on the application root's event phase. Lower runs
first, and the last stage to touch a registry owns it — ordering, not
privilege, is what makes a configuration outrank a distribution default.

| Order | Stage | Job |
|---:|---|---|
| `-200` | `pi.builtins.config` | composes/publishes configuration, republishes it into the event, applies the configured model |
| `-100` | `pi.builtins.defaults.model` | picks the first available catalog candidate *when the event still carries no model* |
| `-99` | `pi.builtins.defaults.tool-root` | re-declares the shipped tool suite with the launcher root |
| `-60` | `pi.builtins.session.command` | answers `session` events with a queued `session_result` |
| `-50` | `pi.builtins.config.tools` | re-declares the tool suite from the configured `tools` section |

One further shipped stage runs elsewhere: `pi.builtins.session.record`
(`agent`/`render`, order `100`) folds the settled agent batch into records and
returns it untouched.

## Optional persistence

The session package owns no root and registers only stages, so removing its
three entries from a copied manifest leaves exactly the ephemeral product: the
same frame, the same conversation, no state root, and `session` commands
falling through to the application root's unhandled-event notice. A launch that
never says anything writes no session record either, with or without the
package: a model selection alone does not open a log.

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
