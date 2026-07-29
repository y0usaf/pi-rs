# Shipped session package

`crates/pi-rs-builtins/session/` is an ordinary Lua package graph over the
public surface only (`pi.records.v1`, `pi.effects.v1`, `pi.kernel.v1.module`,
`pi.roots.v1.middleware`). Loading order is `records.lua`, `store.lua`,
`init.lua`; only `init.lua` registers anything.

Persistence is an **addition**, never a requirement. The package registers two
stages and owns no root, so a distribution that does not load it is exactly the
ephemeral application from PLAN 3.6: nothing is written, no directory is
created, and a `session` event reaches the application root as an ordinary
unhandled event.

No record schema, session name, branch meaning, compaction rule, retention rule,
or directory choice lives in Rust. The host contributes an append-only record
store that never interprets a record, bounded filesystem effects, path
arithmetic, an immutable environment snapshot, and root middleware.

## Stages

| Stage | Kind/phase | Order | Job |
|---|---|---|---|
| `pi.builtins.session.record` | `agent` / `render` | `100` | folds the settled agent batch into records and appends them |
| `pi.builtins.session.command` | `application` / `event` | `-60` | answers `session` events with one queued `session_result` action |

The recording stage runs **after** the agent root settles and returns the batch
unchanged, so persistence can never alter, delay, or fail a turn. An unwritable
session directory costs the run one diagnostic in `session_result.error`, not
the conversation.

What is recorded comes from the agent's *public* action vocabulary, so a
replacement session package sees exactly what this one sees, and a replacement
agent that emits the same actions is persisted with no change here.

| Action | Record |
|---|---|
| `agent_turn_start` | `message` (`user`) |
| `agent_steered` | `message` (`user`) |
| `agent_message` | `message` (`assistant`) |
| `agent_tool_result` | `message` (`tool`) with `call_id`, `name`, `ok` |
| `agent_configured` | `model`, when the model id changes |
| `agent_error`, `agent_cancelled` | `note` |
| `agent_reset` | ends the log; the next turn opens a new one |

`agent_message` publishes the settled text and a tool-call count, not the
provider content blocks, so a persisted assistant turn carries its text and the
tool results that follow carry their own id, name, and output.

## Records

`pi.session.records@1` is pure: it names no destination, opens no store, and
performs no effect. The log is append-only, so every later fact is a later
record rather than an edit, and reconstruction is a left fold with no seeking.

| Record | Meaning when folded |
|---|---|
| `header` | identity of this log: `id`, `created_ms`, optional `title`/`model`/`parent` |
| `message` | one `user`, `assistant`, or `tool` message appended to the conversation |
| `title` | renames the session from this point on |
| `model` | the model in force from this point on |
| `compaction` | replaces messages `1..through` with one summary message |
| `branch` | re-identifies the log after a prefix copy |
| `note` | provenance or diagnostic text; never part of the conversation |

Writing is fail-closed: every constructor validates and raises, naming its
dotted path, so a malformed record never reaches disk. Folding is the opposite
and deliberately tolerant, because a log is durable and outlives the package
that wrote it. An unknown record kind, a missing header, or a compaction
pointing past the end is counted and described in `diagnostics` rather than
raising, so a log written by another package still yields whatever conversation
it does contain.

Text is truncated to `max_text_bytes` (16 KiB) per field and flagged with
`truncated`. The record store refuses a record larger than 1 MiB and a single
tool can emit far more, so losing the tail of one turn is the recoverable
choice; a refused append would lose the turn itself.

## Storage

`pi.session.store@1` turns the generic record store into sessions. It still
names no directory: every entry point takes an explicit `directory` (where
writes go) and an optional read-only `legacy` directory. Choosing those two is
`init.lua`'s job, and it asks the one path policy, `pi.config.paths@1`, for the
`sessions` resource — `$XDG_STATE_HOME/pi/sessions` with `~/.pi/agent/sessions`
as the legacy counterpart. Without that module loaded the package refuses to
write at all rather than inventing a second directory rule.

Two rules make the legacy directory safe:

1. `list` reads both directories and labels every row `canonical` or `legacy`;
   nothing is written to a legacy path, ever.
2. `resume` on a legacy-only id copies the log forward into the canonical
   directory first and continues there, exactly as the credential store
   promotes a legacy row on first write. The inherited file is left
   byte-for-byte as it was, and the promoted copy records a `note`.

A session id is also its file name, so it is restricted to letters, digits,
`.`, `-`, and `_`. A generated id is `<UTC timestamp>-<counter>`, which sorts
oldest-first in a directory listing. Two processes starting a session in the
same second can collide on that name; a collision retries with the next counter
value rather than failing, while an id the caller *asked* for is tried once.

A live log holds its own exclusive file lock for as long as the package is
alive. It is a scope resource of the package, so disposing the package closes
the store and releases the lock — not Lua garbage collection. A live log is
therefore never reopened to be read: `status` and `describe` answer from the
fold already in memory.

## Commands

An application event `{kind = "session", command = ...}` is answered by the
command stage, which stops the chain and queues one `session_result` action.
Every result carries `command` and `ok`; a refusal carries `error` and changes
nothing.

| Command | Effect |
|---|---|
| `status` | live session report, resolved directories, and the last storage error |
| `list` | canonical and legacy rows plus diagnostics for files that could not be read |
| `describe{id}` | folds one log and returns its reconstructed `conversation` |
| `resume{id}` | opens a log for appending, promoting a legacy one first |
| `new` | ends the live log; the next recorded turn opens another |
| `name{title}` | appends a `title` record |
| `compact{through, summary}` | appends a `compaction` record |
| `branch{id, records}` | copies a prefix to a new log, re-identifies it, and makes it live |
| `retain{keep}` | removes all but the `keep` most recently modified canonical logs |
| `close` | closes the live log and releases its lock |

Retention never removes three things: a legacy log (pi-rs does not own that
directory), the live log, and a log the listing could not open at all. A locked
or damaged file is diagnosed, not deleted, so retention cannot destroy the one
log a reader most wants to look at.

## Replacing it

The seam is public. A package that registers its own `agent`/`render` stage and
writes through `pi.records.v1` persists whatever schema it likes at whatever
destination it likes, with no shipped session package loaded and no privileged
ordering —
`crates/pi-rs-builtins/tests/session_package.rs::a_file_backed_replacement_persists_a_different_schema`
is that package, in full, as a file-backed fixture.

## Acceptance

`crates/pi-rs-builtins/tests/session_package.rs` drives every scenario through
the public kernel transaction: suppression, file-backed replacement, XDG-only
writes, tool settlement, the shipped agent's real vocabulary, resume, reset,
naming, compaction and its refusal, branching, retention, legacy promotion, a
torn log, a foreign log, a stale handle, package disposal releasing the lock,
truncation, an unusable state root, and an unknown command.
