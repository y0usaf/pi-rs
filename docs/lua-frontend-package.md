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
  batch into named keys (`text`, `submit`, `newline`, `backspace`, `left`,
  `right`, `up`, `down`, `home`, `end`, `clear_line`, `interrupt`, `eof`,
  `escape`, `unknown`). Printable bytes are consumed as whole runs, so a paste
  is one key, never one key per character.
- `pi.frontend.editor@1` — `new(limits)` returns the multiline prompt buffer:
  `insert`, `newline`, `backspace`, `delete`, `move(direction)`, `clear_line`,
  `clear`, `text`, `is_empty`, `lines`, `cursor`. Limits are
  `max_lines = 64`, `max_line_bytes = 4096`; the cursor is UTF-8 aware and
  control bytes never enter the buffer.
- `pi.frontend.transcript@1` — `new(limits)` returns the bounded row list:
  `user`, `assistant_delta`, `assistant_done`, `tool_start`, `tool_result`,
  `notice`, `rows`, `len`, `clear`. Limits are `max_rows = 200`,
  `max_row_bytes = 4096`, `max_tool_output = 120`. A streaming assistant row is
  appended in place, so deltas update one retained row.
- `pi.frontend.chrome@1` — header, footer hints, status word, and
  `guidance_for(reason)`, which maps an agent error reason to one actionable
  line (missing model, missing/rejected credentials, declared turn limits).
- `pi.frontend.view@1` — `build(state)` returns one display batch. Node
  identities are stable (`1` root, `2` header, `3` transcript, `4` editor,
  `5` footer, `6` guidance, `100+` transcript rows, `200+` editor rows), so an
  unchanged region is retained and only changed cells are painted.

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
| `status` | report size, status, guidance, and current input text |

Actions out: `ansi` (display mechanism), plus the intents
`frontend_submit`, `frontend_interrupt`, `frontend_exit`, `frontend_ready`,
`frontend_closed`, `frontend_status`, `frontend_diagnostic`. The frontend never
dispatches the agent and holds no host state.

Key routing goes through the focused component (`editor` today) rather than a
per-key branch, so a later component joins without reshaping the root. Enter
submits, alt+enter inserts a line, ctrl+c requests an interrupt, and ctrl+d on
an empty prompt requests exit.

## Application root

The application root is the coordinator. It answers host events (`startup`,
`configure`, `input`, `resize`, `prompt`, `shutdown`) by dispatching the
frontend and agent roots through `pi.roots.v1.dispatch`, and republishes **only**
`ansi` and `shutdown` — the two action kinds Rust interprets. Every other
action stays inside the Lua roots.

One journey: `input` → frontend decodes and renders → `frontend_submit` →
agent `prompt` → the settled agent batch is handed back to the frontend as one
`agent` event → each transcript change renders, so assistant text and tool rows
appear incrementally rather than at turn end.

Snapshot payloads are read-only views, so anything kept across dispatches (the
selected model) or sent back through another dispatch is copied into a plain
table first.

## Evidence

`crates/pi-rs-builtins/tests/frontend_package.rs` drives 12 deterministic
journeys through the public application root with a registered fixture api:
input-ready startup frame, typed prompt with incremental streaming frames, tool
start/result rows, interrupt then cancelled turn, ctrl+d shutdown, resize
repaint, missing-model guidance, rejected-credential guidance with bounded
retry, multiline editing, a file-backed frontend replacement, a file-backed
application replacement driving the shipped frontend, and a file-backed render
middleware wrapping the shipped frame.

## Known gaps

- Terminal size is not yet a public mechanism: the frontend starts at 80×24 and
  adopts real dimensions from a `resize` event. The launcher gains that event
  when the size/resize mechanism lands.
- Rows are clipped, not wrapped, and the transcript has no scrollback commands;
  richer presentation is PLAN 5.1–5.3.
