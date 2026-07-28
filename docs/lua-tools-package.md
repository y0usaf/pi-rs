# Shipped core-tool package

`crates/pi-rs-builtins/tools/` is an ordinary Lua package graph: it uses the
public coding spine only (`pi.effects.v1`, `pi.roots.v1`, `pi.kernel.v1.module`)
plus the one tool declaration path from the agent package
(`pi.agent.tools@1`). Loading order is `paths.lua`, `render.lua`, `locks.lua`,
`read.lua`, `write.lua`, `edit.lua`, `bash.lua`, `init.lua`; only `init.lua`
declares anything.

There is no privileged builtin executor: a tool is a Lua table with an
`execute(call)` function, and the shipped four are declared exactly the way a
file-backed package declares its own.

## Modules

- `pi.tools.paths@1` — `resolver{root, allow_absolute}` returns
  `resolve(path)` (normalized path or `nil, reason`) and `display(path)`.
  Normalization is lexical: `..` above the root is a rejection, not a resolved
  path. Without a `root`, relative paths reach the host working directory and
  absolute paths are refused.
- `pi.tools.render@1` — `number_lines`, `clip` (byte bound plus an explicit
  “… truncated N more bytes” notice), and `diff` (shared prefix/suffix, one
  changed block, bounded rows) — the render data a transcript row shows.
- `pi.tools.locks@1` — cooperative per-path serialization: `acquire`,
  `release`, `guard(path, body)`, plus a per-path `revision`/`bump` used as an
  optimistic write guard. Effects yield, so two mutations of one path can
  interleave inside a dispatch; the lock is the tool policy that prevents it.
- `pi.tools.read@1`, `pi.tools.write@1`, `pi.tools.edit@1`, `pi.tools.bash@1`
  — one tool each, exposing `execute(call, options)`, `declare(registry,
  options)`, and `unregister(registry, name)`.
- `pi.tools.suite@1` — `declare(registry, options)`, `unregister(registry[,
  name])`, `names()`. `options.suppress[name] = true` drops one tool,
  `options.tools[name]` configures one, `options.shared` configures all.

## Tools

| Tool | Arguments | Settlement |
|---|---|---|
| `read` | `path`, `offset`, `limit` | parallel-eligible |
| `write` | `path`, `content` | `serialize` |
| `edit` | `path`, `old_text`, `new_text`, `replace_all`, `expected_revision` | `serialize` |
| `bash` | `command`, `cwd`, `timeout_ms` | `serialize` |

Every result is `{output, is_error, details}`. `output` is the bounded text a
model sees; `details` is render data (`path`, `lines`, `bytes`, `created`,
`added`/`removed`, `diff` rows, `code`, `killed`, `cancelled`, `truncated`,
`revision`). The current agent forwards `output` and `is_error`; promoting
`details` to a public tool-result seam is 4.1 work.

## Options

`read`: `max_bytes` (512 KiB), `max_lines`, `max_output_bytes`.
`write`/`edit`: `max_bytes` (1 MiB), `wait_ms` (lock budget).
`bash`: `shell` (`bash`), `timeout_ms` (120 s), `max_output_bytes` (64 KiB
rendered), `process_max_output_bytes` (1 MiB mechanism bound), `cancelled`
(cancellation predicate), `serialize`.
All tools accept `root` and `allow_absolute` for the path resolver, or a
prebuilt `resolver`.

## Mutation safety

`write`, `edit`, and `bash` run inside `locks.guard`. `write`/`edit` lock the
resolved path; `bash` locks one shared workspace slot because a command's paths
are unknown. A busy path fails with `path is busy: …` instead of racing, and
`edit` can pass `expected_revision` to refuse a stale rewrite. The lock is
always released, including when the tool body fails.

## Process-tree cancellation

The host process effect kills the process it spawned; anything that command
backgrounded survives. `bash` therefore runs the command as a job under
`set -m`, so the job owns a new process group, and prints a marker
(`\1pi-pgid:<gid>\1`) that the tool strips from the rendered output. On
cancellation or timeout the tool kills that group (`TERM`, then `KILL`), so
backgrounded grandchildren die with the command.

Cancellation is observed at output boundaries: the `onData` callback polls the
cancellation predicate (by default the kernel dispatch cancellation handle) and
aborts the run's signal. A command that produces no output is bounded by its
timeout instead — an async effect handle would be needed for a true mid-silence
interrupt, the same limit recorded for parallel tool settlement in 3.3.

## Suppressing and replacing one tool

```lua
local pi = ...
local module = pi.kernel.v1.module
local registry = module.require("pi.agent.tools", "1")
local suite = module.require("pi.tools.suite", "1")

suite.unregister(registry, "bash")            -- suppress
suite.unregister(registry, "read")            -- replace
registry.register({
  name = "read",
  description = "project read",
  owner = "my-package",
  execute = function(call) return { output = "…" } end,
})
```

`crates/pi-rs-builtins/tests/tools_package.rs` drives all of this through the
public kernel transaction with a file-backed driver root.
