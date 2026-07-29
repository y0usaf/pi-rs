# Shipped application/frontend package

`crates/pi-rs-builtins/frontend/` is an ordinary Lua package graph over the
public coding spine only (`pi.roots.v1`, `pi.terminal.v1`, `pi.kernel.v1.module`).
It has no persistence dependency and no host privilege. Loading order is
`keys.lua`, `editor.lua`, `transcript.lua`, `chrome.lua`, `view.lua`,
`init.lua`, `application.lua`; the first five only define modules.

`init.lua` registers the frontend root `pi.builtins.frontend` and
`application.lua` registers the application root `pi.builtins.application`,
both at priority `0`. Registering either kind at a higher priority replaces
that half of the product without forking the other.

## Modules

- `pi.frontend.keys@1` — `decode(events[, limit])` turns one terminal input
  batch into named keys (`text`, `submit`, `follow_up`, `newline`,
  `backspace`, `left`, `right`, `up`, `down`, `home`, `end`, `clear_line`,
  `toggle_thinking`, `interrupt`, `eof`, `escape`, `unknown`). Printable bytes
  are consumed as whole runs, so a paste is one key, never one key per
  character. alt+enter decodes to `follow_up`; whether that queues a message
  or inserts a line is the root's policy, not the decoder's.
- `pi.frontend.editor@1` — `new(limits)` returns the multiline prompt buffer:
  `insert`, `newline`, `backspace`, `delete`, `move(direction)`, `clear_line`,
  `clear`, `text`, `is_empty`, `lines`, `cursor`. Limits are
  `max_lines = 64`, `max_line_bytes = 4096`; the cursor is UTF-8 aware and
  control bytes never enter the buffer.
- `pi.frontend.transcript@1` — `new(limits)` returns the bounded entry list and
  the presentation policy over it: `user`, `assistant_delta`, `assistant_done`,
  `thinking_delta`, `thinking`, `tool_start`, `tool_result`, `notice`,
  `status(key, level, text)`, `queue(name, text)`, `unqueue(name, text)`,
  `queued`, `clear_queue`, `set_option`, `option`, `lines(width, limit)`,
  `rows`, `len`, `clear`. Limits are `max_entries = 200`,
  `max_entry_bytes = 4096`, `max_argument = 120`, `max_output = 120`,
  `max_block_rows = 64`, `max_queue_rows = 16`. A streaming assistant or
  reasoning entry grows in place, so deltas update one retained block.
  `status` is a keyed notice: re-announcing the same key rewrites the row
  already in the transcript instead of appending a second one.

  `lines(width, limit)` is the whole appearance of the transcript. Each entry
  becomes a **block** of full-width display lines, blocks are separated by one
  untouched row, and text starts at column 1:

  | Block | Appearance |
  |---|---|
  | user | background `#343541`, one padded row above and below, text `#d4d4d4` |
  | assistant | unstyled text, no padding rows |
  | thinking | italic `#808080`; `Thinking...` when `thinking_visible` is false |
  | tool, running | background `#282832`, bold `#d4d4d4` name, `#8abeb7` arguments |
  | tool, succeeded | the same block on `#283228`; the call collapses to what ran |
  | tool, failed | the same block on `#3c2828`, plus the bounded output |
  | notice, error | `#cc6666` |
  | notice, info/warn | `#666666` |
  | queue | `#666666`; `Steering: …` rows, then `Follow-up: …` rows, then the hint |

  Only the newest `limit` lines are built, so a frame costs one viewport
  whatever the history length, and a call is summarised as its name plus its
  scalar arguments in key order so the block reads like the command that ran.

  The `queue` block is the one block that is not history. `queue(name, text)`
  mirrors a message the agent accepted (`steer` or `follow_up`) and
  `unqueue(name, text)` drops it again when the turn drains it into an
  ordinary user block, so the same text is never on screen twice. The block
  is pinned under the newest entry, holds at most `max_queue_rows` messages,
  puts every steering row before every follow-up row, truncates instead of
  wrapping, and ends with one hint row naming the key that restores them.

  Every row of that table is a **declaration**, not a private branch. Each
  shipped block is declared through the one generic declaration path,
  `pi.kernel.v1.declare("renderer", definition)`, with
  `surface = "transcript.block"` and one claimed `entry` kind, so a single
  block is replaceable without forking the frontend root:

  ```lua
  pi.kernel.v1.declare("renderer", {
    id      = "my.package.user-block",
    surface = "transcript.block",
    entry   = "user",   -- user | assistant | thinking | tool | notice | queue
    order   = 10,       -- shipped blocks declare 0
    render  = function(entry, context)
      return { context.line({ { text = "> " .. entry.text } }) }
    end,
  })
  ```

  `render` returns display lines — `{ runs = { { text = ..., style = ... } } }`,
  the shape `pi.frontend.view` already consumes — and receives a per-frame
  `context` of `width`, `body` (usable text width, one column narrower than
  the block), a copy of `limits`, a copy of `options`, `line(runs[, fill])`,
  and `padded(fill)`. `pi.frontend.transcript@1` also exports `palette` (the
  reviewed colors), `default_options`, and `declarations()` (the shipped
  rows), so a renderer may reuse any of them.

  `options` are the frame's presentation policy: bounded scalars set with
  `set_option(key, value)` (at most 32 keys, strings/numbers/booleans only)
  and read by any renderer. The shipped `thinking_visible` option is what
  ctrl-t toggles, so collapsing reasoning needs no block-specific host
  surface — a replacement renderer reads the same option, and a package may
  add its own.

  The winner for an entry kind is the **last** matching declaration in
  registered order — `order`, then source, then id — so a positive `order`
  wins deterministically rather than by load order, and disposing the
  declaring package restores the shipped block. Resolution is one bounded host
  read per frame, not per block. Renderer output is bounded like any other
  policy: a block is clipped to `max_block_rows + 2` lines, a malformed line
  is dropped, and an entry kind nothing claims still renders its text
  unstyled, so removing presentation never silently removes content.
- `pi.frontend.chrome@1` — header, footer hints, status word, and
  `guidance_for(reason)`, which maps an agent error reason to one actionable
  line (missing model, missing/rejected credentials, declared turn limits).
- `pi.frontend.view@1` — `build(state)` returns one display batch. Node
  identities are stable (`1` root, `2` header, `3` transcript, `4` editor,
  `5` footer, `6` guidance, `100+` transcript rows, `200+` editor rows), so an
  unchanged region is retained and only changed cells are painted. The
  transcript is bottom anchored: the newest block sits against the prompt and
  older lines scroll off the top.

## Frontend root

Events in:

| `event.kind` | Meaning |
|---|---|
| `startup` | full input-ready repaint, then `frontend_ready` |
| `configure` | set the displayed model label |
| `input` | decode raw bytes, route keys to the focused component, render |
| `agent` | fold an agent action batch (`event.actions`) into rows and status |
| `resize` | adopt new `columns`/`rows` and repaint everything |
| `notice` | append one notice row |
| `shutdown` | closing frame plus `frontend_closed` |
| `status` | report size, status, guidance, current input text, and the pending queue |

Actions out: `ansi` (display mechanism), plus the intents
`frontend_submit`, `frontend_interrupt`, `frontend_exit`, `frontend_ready`,
`frontend_closed`, `frontend_status`, `frontend_diagnostic`. The frontend never
dispatches the agent and holds no host state.

Key routing goes through the focused component (`editor` today) rather than a
per-key branch, so a later component joins without reshaping the root. Enter
submits, alt+enter inserts a line, ctrl+c requests an interrupt, ctrl+d on
an empty prompt requests exit, and ctrl+t toggles thinking blocks.

While the agent reports `streaming`, those two keys mean something else:
enter offers the line to the agent's steering queue and alt+enter offers it
as a follow-up, both as `frontend_submit` with `queue = "steer"` or
`queue = "follow_up"`. Nothing is shown optimistically — a pending row
appears only when the agent answers `agent_queued` with `accepted = true`,
and a refused message becomes a warning notice rather than vanishing.

## Application root

The application root is the coordinator. It answers host events (`startup`,
`configure`, `input`, `resize`, `prompt`, `shutdown`) by dispatching the
frontend and agent roots through `pi.roots.v1.dispatch`, and republishes **only**
`ansi` and `shutdown` — the two action kinds Rust interprets. Every other
action stays inside the Lua roots.

One journey: `input` → frontend decodes and renders → `frontend_submit` →
agent `prompt` → the settled agent batch is handed back to the frontend as one
`agent` event → each transcript change renders, so assistant text and tool rows
appear incrementally rather than at turn end. A `frontend_submit` carrying
`queue = "steer"` or `queue = "follow_up"` takes the same journey to the
agent's `steer`/`follow_up` events instead of `prompt`.

Snapshot payloads are read-only views, so anything kept across dispatches (the
selected model) or sent back through another dispatch is copied into a plain
table first.

## Evidence

`crates/pi-rs-builtins/tests/frontend_package.rs` drives 16 deterministic
journeys through the public application root with a registered fixture api:
input-ready startup frame, typed prompt with incremental streaming frames, tool
start/result rows, interrupt then cancelled turn, ctrl+d shutdown, resize
repaint, missing-model guidance, rejected-credential guidance with bounded
retry, multiline editing, a file-backed frontend replacement, a file-backed
application replacement driving the shipped frontend, a file-backed render
middleware wrapping the shipped frame, a file-backed `renderer` declaration
replacing only the user block while the shipped assistant block and chrome
survive, two competing renderers proving `order` decides the winner rather
than load order, a line typed during a turn travelling frontend → application
→ agent → pending row, and a file-backed replacement of the queue block.
Those journeys assert what the frame *says*, with styling stripped.

`crates/pi-rs-app/tests/transcript_presentation.rs` asserts what the transcript
*looks like*: it replays the shipped frontend's ANSI into a terminal emulator
and compares the transcript region of the screen, cell by cell, with the
canonical `tool-pending`, `cancelled`, `thinking-hidden`, `thinking-visible`,
`steering-queued`, and `follow-up-queued` checkpoints in
`tests/experience/canonical-v1.json`. It also holds the two presentation
budgets: a streamed delta repaints only its own row, and a 400-turn transcript
still costs one viewport per frame.

## Known gaps

- Terminal size is not yet a public mechanism: the frontend starts at 80×24 and
  adopts real dimensions from a `resize` event. The launcher gains that event
  when the size/resize mechanism lands.
- Transcript text wraps at the block width, but no canonical checkpoint records
  a wrapped transcript line, so the wrap points are pi-rs policy rather than a
  matched observation. There are no scrollback commands.
- Warning, retry, and compaction blocks reuse the notice styling instead of
  having their own.
- The queue block's hint row names alt+up, which the shipped decoder does not
  understand yet: restoring queued messages to the editor is PLAN 5.2 keymap
  work. Queueing itself also cannot be reached by typing in the shipped
  loop, which reads input and settles a whole turn one dispatch at a time;
  PLAN 5.4 owns turns that span dispatches.
- The header, working indicator, prompt chrome, and status line are still the
  minimal 3.x ones; PLAN 5.2–5.3 own them.
