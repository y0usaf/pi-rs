# Shipped configuration package

`crates/pi-rs-builtins/config/` is an ordinary Lua package graph over the
public surface only (`pi.effects.v1`, `pi.records.v1`, `pi.packages.v1`,
`pi.models.v1`, `pi.kernel.v1.module`, `pi.roots.v1.middleware`). Loading order
is `json.lua`, `paths.lua`, `schema.lua`, `trust.lua`, `defaults.lua`,
`apply.lua`, `init.lua`; only `init.lua` registers anything.

No directory name, precedence rule, fallback, trust concept, merge rule, or
default lives in Rust. The host contributes an immutable environment snapshot,
path arithmetic, bounded filesystem metadata, an append-only record store, and
package composition.

## Layers

| Layer | Source | Evaluated as |
|---|---|---|
| `defaults` | `pi.config.defaults@1` | ordinary Lua table |
| `user` | `$XDG_CONFIG_HOME/pi/config.lua`, else `~/.pi/agent/settings.json` | sandboxed chunk / strict JSON |
| `project` | `<root>/.pi/config.lua` | sandboxed chunk, trusted directories only |

The user layer follows the storage contract exactly: the legacy file is read
**only** when the canonical file is absent, and a canonical file that exists but
fails to load is an error rather than a fall-through. Nothing here writes a
configuration file, and nothing ever writes under the legacy root.

A configuration file is **not** a package. It is a chunk loaded with an explicit
environment holding `assert`, `error`, `ipairs`, `next`, `pairs`, `select`,
`tonumber`, `tostring`, `type`, and read-only `math`, `string`, `table`, and
`utf8`. There is no `pi`, no `io`, no `os`, no `require`, and no `load`, and
each standard library is proxied so one file cannot rewrite `string.format` for
the whole VM. The chunk receives a context (`layer`, `project_root`, and the
resolved `paths`) and returns a settings table. Capability arrives the ordinary
way: naming packages in `packages`, which are then loaded through
`pi.packages.v1` like any other package.

## Sections

`pi.config.schema@1` is the single schema; a new section is a row in it, not a
new code path. It is fail-closed — an unknown key or a wrong type is an error
naming its dotted path, because a typo that silently does nothing is the classic
configuration trap.

| Section | Shape |
|---|---|
| `theme` | string |
| `model` | `{provider, id}` |
| `keymaps` | map of string to string |
| `packages` | list of package paths |
| `modules` | list of `{name, version}` |
| `providers` | map of provider name to `{api, base_url, models}` |
| `tools` | `{root, suppress, settings}` |
| `roots` | `{application, agent, frontend, session}` |

Merge policy: records and maps merge key by key; lists and scalars replace
wholesale. Lists replace rather than concatenate, so a lower layer can never
force back an entry a higher layer removed.

Validating a section proves the file is well formed; **applying** it is a
separate step, and `pi.config.apply@1` owns it. Two mechanisms carry
everything:

| Section | Applied through | Result |
|---|---|---|
| `modules` | `pi.kernel.v1.module.require(name, version)` | every named identity resolves, or the reload fails |
| `theme` | `pi.kernel.v1.declare("theme", ...)` | `pi.config.theme` |
| `keymaps` | `pi.kernel.v1.declare("keymap", ...)` | `pi.config.keymap:<binding>`, one per binding, sorted |
| `providers` | `pi.kernel.v1.declare("provider", ...)` | `pi.config.provider:<name>`, sorted |

Declaration ids live in a `pi.config.` namespace, so a configured provider
never silently collides with one a package declared and a consumer can tell
that a declaration came from configuration. Every declaration carries the
`layer` and `origin` file behind it, the same provenance the effective settings
expose. A configured model starts from its reviewed catalog row and takes the
section's `api`/`base_url` overrides, so nothing invents a cost, a context
window, or a token budget; a model the catalog does not carry is an error
naming its dotted path. Full custom model rows are PLAN 6.4.

The plan is built during composition, before anything is published, so a
section the product cannot accept rolls the whole reload back rather than half
applying after the settings already changed.

`tools` and `roots` validate, merge, and publish with everything else, but
nothing acts on them yet: handing `tools` to the shipped suite has to be
reconciled with `defaults/init.lua`'s `pi.builtins.defaults.tool-root`
middleware, and root selection is part of the manifest work in PLAN 4.4.

## Trust

A project's `.pi/config.lua` arrived with a checkout rather than with the user,
so it is never evaluated until that exact directory carries a decision. The
decision is a record in `<state>/pi/trust/trust.jsonl` through
`pi.records.v1` — the generic store, which knows nothing about trust.

Asking a question creates nothing: the store, its directory, and its lock appear
on the first decision. Recording a decision a directory already carries appends
nothing (`changed = false`), so replaying a startup never grows the file, and
revoking is an ordinary later record that leaves the history readable.

## Publication

`pi.config.settings@1` composes, publishes, and inspects:

- `reload(options)` and `ensure(options)` — compose and publish;
- `effective()`, `provenance()`, `leaves()`, `revision()` — the published
  configuration, the layer and file behind every dotted leaf, and the revision;
- `sources()`, `errors()` — every considered layer with its outcome
  (`selected`, `absent`, `invalid`, `untrusted`, `denied`, `unavailable`);
- `resources()`, `roots()` — the resource matrix and the storage roots;
- `trust(directory, decision)`, `trust_decision(directory)`, `trust_list()`;
- `declarations()`, `modules()` — what the published configuration applied:
  the declaration rows it produced and the module identities it resolved.

Publication is atomic: discovery, evaluation, validation, merging, and package
loading all complete before anything visible changes. Any failure leaves the
previous settings, provenance, package generation, and revision exactly as they
were. Recomposing an unchanged configuration publishes nothing, keeps the
revision, and does not reload packages; a package already loaded is retained
rather than restarted, and only packages that left the selection are disposed —
after the swap, so a disposal failure cannot half-apply a configuration.

Declarations are the one derived thing that cannot simply be swapped.
`pi.kernel.v1.declare` refuses a second declaration of one kind and id, and a
declaration lives exactly as long as the package scope that made it — and the
configuration package's own scope outlives every reload, so it can never
re-declare its own theme. The staged plan is therefore replayed by a tiny
package the configuration loads and disposes like any other
(`pi.config.declarations`, a two-line constant chunk): disposing it retracts
its declarations so the next revision may declare the same stable ids.
Because the ids are stable, the order is dispose-then-load; if the replay is
refused, the previous plan is put back and the reload rolls back.

The observations move even when the policy does not: `sources()` and `errors()`
always describe the most recent attempt, because the reason a reload was refused
is exactly what a user needs to see.

## Resources

`pi.config.paths@1` resolves each resource canonical-first with a per-resource
legacy fallback, and every destination is canonical:

| Resource | Canonical | Legacy |
|---|---|---|
| `config` | `<config>/config.lua` | `<legacy>/settings.json` |
| `packages` | `<data>/packages` | `<legacy>/packages` |
| `sessions` | `<state>/sessions` | `<legacy>/sessions` |
| `credentials` | `<state>/credentials.json` | `<legacy>/auth.json` |
| `cache` | `<cache>` | `<legacy>/cache` |
| `trust` | `<state>/trust` | — |

An explicit absolute `XDG_*_HOME` wins for its class; an empty value means
"unset"; a relative one is ignored with a diagnostic rather than resolved
against the working directory. Without a usable root the class is `unavailable`,
never the current directory.

## Application seam

`init.lua` registers one application event middleware, `pi.builtins.config`
(order `-200`), which composes on the first dispatch — when the launcher context
first names the project root — and republishes `event.config` and
`event.config_revision` into every event, so a package reads policy from its
snapshot instead of reaching for a module. Recomposition is the explicit
`config_reload` event, never per-dispatch work. When the configuration names a
model that the catalog offers, the middleware sets `event.model` unless the
event already carries one, so a later package may still choose its own.

A broken configuration file never blocks startup: it publishes diagnostics and
leaves the event untouched.

## Acceptance

`crates/pi-rs-builtins/tests/config_package.rs` drives 19 deterministic
scenarios through the public kernel transaction: canonical-over-legacy
precedence, legacy-only fallback with reported unknown keys, no fall-through
from a broken canonical file, the trust matrix (undecided, trusted, repeated,
revoked, and another directory), rollback across four failure modes with the
package generation intact, idempotent recomposition and generation swap,
duplicate package refusal, complete two-directional provenance coverage, the
resource matrix, `$HOME` defaults with a refused relative override, model policy
changing when the file changes, the sandbox refusing host capability, a
zero-configuration run that writes nothing and declares nothing, the absence of
any host configuration module, theme/keymap/provider declarations read back
through `pi.kernel.v1.registered`, declaration replacement and retraction
across reloads with exactly one declaration package alive, an unknown
configured model failing the reload with its declarations intact, and module
pinning with its own rollback.
