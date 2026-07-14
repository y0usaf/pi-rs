# Lua coding spine v1

This is the complete public slice needed by the first file-backed coding journey.
It is intentionally smaller than the eventual product API. Rust supplies generic
mechanisms; application, agent, frontend, tool, and shutdown policy stay in Lua.

## Loading

`pi --package FILE` loads ordinary Lua files in command-line order. A versioned
manifest selects the same files declaratively:

```json
{"version":1,"packages":["agent.lua","frontend.lua","application.lua"]}
```

Manifest paths are relative to the manifest directory. Explicit `--package`
paths are relative to `--root`. `--manifest FILE` selects a manifest explicitly;
a distribution wrapper may set `PI_PACKAGE_MANIFEST` instead. Neither mechanism
knows builtin names or changes package capability.

A package is a Lua chunk receiving `pi` as `...`. Embedded, memory, and file
sources enter one package transaction with identical APIs, watchdogs, scope
cleanup, and conflict rules; source text is attribution only. A raw invocation
with no package/manifest prints guidance and exits successfully.

## Transaction roots: `pi.roots.v1`

- `register(definition)` → the sole kernel root-registration path. This slice
  accepts `kind = "application" | "agent" | "frontend"`; definitions also have
  `id`, `dispatch(snapshot)`, optional `active` (default `true`), and optional
  `priority` (default `0`).
- `action(kind, payload)` → queues one validated action for publication only
  after a successful dispatch.
- `cancellation()` → read-only cancellation handle for the current dispatch
  (`is_cancelled()`, `wait()`).
- `module` → exact-version `define`, `require`, and `list` mechanism shared with
  `pi.kernel.v1.module`.

Every dispatch receives a read-only snapshot:

```lua
{
  version = 1, generation = ..., scope = ...,
  event = <immutable JSON>, context = <immutable JSON>,
}
```

Lua never receives mutable host state. A failed/timed-out dispatch publishes no
actions. A package may queue `action("shutdown", {...})`; interpretation is
application policy, while the launcher always disposes all package scopes before
process exit.

## Terminal/display: `pi.terminal.v1`

- `input_buffer()` → bounded terminal-byte decoder. `feed(bytes)` and `flush()`
  return batches of `{kind="data"|"paste", data=...}`; no callback occurs per
  byte.
- `display([limits])` → retained display handle. `submit(batch)` validates and
  submits one complete display tree, returning revision, ANSI diff, identity
  delta, and bounded work counters.
- `display_schema_version` → currently `1`; use it as `batch.version`.

A display batch contains `viewport`, `root`, and a flat `nodes` array. Each node
has stable numeric `id`, `rect`, and `content={kind="group"}` or
`content={kind="text", runs={{text=...}}, ...}`. Optional child, focus, cursor,
clip, style, and wrapping fields are display data, not Rust UI policy.

## Models: `pi.models.v1`

- `find(provider, model_id)` → catalog model table or `nil`.
- `stream(model, context, options, on_event)` → final assistant message. Events
  cross as complete typed provider events. `options.max_events` defaults to 256
  and must be `1..=1024`; exceeding it cancels the Lua crossing with an error.
  Transport options such as `apiKey`, `timeoutMs`, `maxRetries`, and `signal`
  remain mechanism data.

Provider selection, prompts, context construction, event folding, retries, and
presentation are Lua policy.

## Effects: `pi.effects.v1`

`fs`:

- `read(path[, max_bytes])` → UTF-8 text. Default 1 MiB; allowed maximum 8 MiB.
- `write(path, contents)` → writes at most 8 MiB.

`process`:

- `run(program[, args][, options])` →
  `{stdout, stderr, code, killed}`.
- `options.timeout_ms` defaults to 30 seconds and is limited to 5 minutes.
- `options.max_output_bytes` defaults to 1 MiB and is limited to 8 MiB.
- `options.cwd`, `options.signal`, and `options.onData(chunk)` are optional.

`cancellation.new()` returns an explicit signal with `abort()`, `is_aborted()`,
and `wait()`. Filesystem/process work crosses the shared bounded effect queue,
is owned by the calling package scope, and is cancelled/settled on package
disposal or host shutdown.

## Minimal package

```lua
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

roots.register({
  kind = "application", id = "minimal",
  dispatch = function(snapshot)
    local input = terminal.input_buffer()
    local events = input:feed(snapshot.event.bytes or "")
    roots.action("observed_input", { events = events })
    roots.action("shutdown", { reason = "done" })
  end,
})
```
