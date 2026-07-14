# Lua mechanism API

The inherited Pi-compatibility extension API is not part of pi-rs. Ordinary Lua
packages receive one compact, versioned mechanism table; embedded provenance
adds no members or privileges.

The complete top-level surface for PLAN 3.1 is:

- `pi.kernel.v1`: package transaction primitives;
- `pi.roots.v1`: application/agent/frontend root facade;
- `pi.terminal.v1`: batched terminal input + retained display submission;
- `pi.models.v1`: catalog lookup + bounded provider event streaming;
- `pi.effects.v1`: bounded filesystem, process, timer, and cancellation effects.

No top-level event bus, command/tool registry, runtime/session/config/settings,
trust/auth UI, `pi.ai`, `pi.tui`, `pi.fs`, `pi.exec`, or `pi.http` compatibility
member is installed. Product declarations and richer capabilities return only
with a demonstrated file-backed consumer.

## Package transaction

A package is a Lua chunk receiving `pi` as `...`. File, memory, and embedded
sources use the same loader, watchdog, publication transaction, scope ownership,
and disposal path.

`pi.kernel.v1` provides:

- `root(definition)`, `declare(kind, definition)`, and `registered(kind)`;
- `action(kind, payload)` and `effect(kind, payload)` queues;
- exact-version `module.define`, `module.require`, and `module.list`;
- immutable `read_handle(value)`, dispatch `cancellation()`, and scoped
  `resource(disposer)`.

Actions/effects publish only after a successful dispatch. Snapshots contain
`version`, `generation`, `scope`, immutable `event`, and immutable `context`.

`pi.roots.v1.register(definition)` accepts `kind = "application" | "agent" |
"frontend"`; `action`, `cancellation`, and `module` are the same canonical
kernel operations.

## Terminal

`pi.terminal.v1.input_buffer()` returns a bounded decoder with `feed`, `flush`,
`clear`, and `buffer`. `display([limits])` returns a retained display handle with
`submit`, `revision`, and `reset_presentation`. Submit complete versioned trees
using `display_schema_version`; no callback occurs per byte or cell.

## Models

`pi.models.v1.find(provider, id)` returns a catalog model or `nil`.
`stream(model, context, options, on_event)` streams complete provider events and
returns the final message. `options.max_events` defaults to 256 and is limited to
`1..=1024`; transport cancellation uses `options.signal`.

## Effects

`pi.effects.v1.fs.read(path[, max_bytes])` and `write(path, contents)` are capped
at 8 MiB. `process.run(program[, args][, options])` caps timeout at five minutes
and output at 8 MiB. `timer.sleep(milliseconds[, signal])` is scope-owned.
`cancellation.new()` returns a signal with `abort`, `is_aborted`, and `wait`.
All queued work is cancelled and settled before package disposal completes.
