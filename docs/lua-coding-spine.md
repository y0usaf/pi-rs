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
- `dispatch(kind, event[, context])` → runs another root from inside an active
  dispatch and returns its settled batch as ordinary data. The nested root gets
  its own transaction; nothing publishes implicitly, so the caller republishes
  chosen actions through `action`. Nesting shares the caller's watchdog budget,
  is depth-capped at 8, and rejects recursion into a kind already on the stack.
- `middleware.register(definition)` → registers one bounded stage around a root
  kind. `kind` and `handler(snapshot)` are required; `id` must be a non-empty
  string, `phase` is `"event"` (default) or `"render"`, and `order` (default
  `0`) breaks ties before registration sequence.

  An `event` stage runs before the resolved root and receives
  `{version, root, phase, event, context, actions}`. It may return a
  replacement `event`, a replacement `actions` array, and `stop = true` to skip
  the remaining stages and the root; the queued actions then become the batch.
  A `render` stage runs after the root settles, receives
  `{version, root, phase, event, actions}`, and may return a replacement
  `actions` array; an explicit empty array suppresses the batch, and a failing
  transform rolls the whole dispatch back so nothing publishes.

  Stages apply to nested dispatches too. Snapshot payloads are read-only views:
  a kept action must be returned as a plain table. Registrations are
  scope-owned, so disposing or rolling back a package removes its stages;
  identical `kind/phase/id` from a different source conflicts.

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
- `display_schema_version` → currently `3`; use it as `batch.version`.

A display batch contains `viewport`, `root`, and a flat `nodes` array. Each node
has stable numeric `id`, `rect`, and `content={kind="group"}` or
`content={kind="text", runs={{text=...}}, ...}`, or
`content={kind="image", data="<base64>", protocol="kitty"|"iterm2"}`. A run may
add `style` and `link="<target>"`, which presents that run's cells inside one OSC
8 hyperlink. An image node's `rect` is its placement, in cells, and is emitted as
one out-of-band escape rather than as glyphs. Optional child, focus, cursor,
clip, style, link, and wrapping fields are display data, not Rust UI policy.

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
