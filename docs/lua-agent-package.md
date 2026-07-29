# Shipped agent package

`crates/pi-rs-builtins/agent/` is an ordinary Lua package graph: it uses the
public coding spine only (`pi.roots.v1`, `pi.models.v1`, `pi.effects.v1`,
`pi.kernel.v1.module`) and has no persistence dependency. Loading order is
`queue.lua`, `tools.lua`, `turn.lua`, `init.lua`; the first three only define
modules, and `init.lua` registers the `agent` root `pi.builtins.agent` at
priority `0`. Registering another agent root with a higher priority replaces
the whole transition policy without touching the frontend.

## Modules

- `pi.agent.queue@1` — `new(limit)` returns a bounded FIFO with `push`
  (`false, reason` when full), `take`, `drain`, `clear`, `len`.
- `pi.agent.tools@1` — the single tool declaration path:
  `register{name, description, parameters, execute(call), serialize, owner}`,
  plus `unregister`, `find`, `list`, and `declarations()` (the provider-facing
  `{name, description, parameters}` rows). `execute` receives
  `{id, name, arguments}` and returns a string or `{output, is_error}`.
  `serialize = true` marks a tool whose calls must not interleave.
- `pi.agent.turn@1` — `new(config)` returns the reducer used by the root.
  `config` accepts `model`, `options`, `system_prompt`, and `limits`
  (`max_retries = 2`, `max_tool_iterations = 8`, `max_follow_ups = 4`,
  `max_requests = 16`, `max_events = 256`, `queue_limit = 64`).

## Events in

| `event.kind` | Meaning |
|---|---|
| `configure` | set `model`, `options`, `system_prompt`, `limits` |
| `prompt` | run one turn from `text` (optionally setting `model`/`options`) |
| `steer` | queue a user message that joins the next request of the active turn |
| `follow_up` | queue a prompt that runs as another turn once the current one completes |
| `interrupt` | cancel the in-flight request and the next turn start |
| `status` | report conversation length, queue depths, and declared tools |
| `reset` | drop the conversation and all queues |

## Actions out

`agent_turn_start`, `agent_status`, `agent_text_delta`, `agent_message`,
`agent_thinking_delta`, `agent_thinking`, `agent_tool_group`,
`agent_tool_start`, `agent_tool_result`, `agent_retry`, `agent_error`,
`agent_cancelled`, `agent_steered`, `agent_follow_up`, `agent_queued`,
`agent_configured`, `agent_reset`, `agent_diagnostic`.

Actions are data. Rendering, transcript shape, and user messaging belong to the
frontend package; the agent never submits display batches.

## Turn policy

1. The prompt is appended, then queued steering messages join the request.
2. One request streams through `pi.models.v1.stream`; text deltas render
   incrementally as `agent_text_delta`, and provider reasoning is named
   separately as `agent_thinking_delta`/`agent_thinking` so a frontend can
   hide it without hiding the reply.
3. A retryable failure (transport error or `stopReason == "error"`) is retried
   up to `max_retries`, each retry announced by `agent_retry`; exhaustion emits
   one `agent_error`. A missing model is not retried.
4. Tool calls are read from the settled assistant message. Consecutive
   parallel-eligible calls settle as one group; a `serialize` tool, or a call
   naming an undeclared tool, settles alone in call order. Every call produces
   a `toolResult` message and an `agent_tool_result` action, including
   failures, so the turn continues.
5. The turn ends when the model stops requesting tools and no steering remains,
   bounded by `max_requests` and `max_tool_iterations`.
6. Follow-ups then run as further turns, bounded by `max_follow_ups`.

Settlement inside a group is bounded and ordered by the public blocking effect
surface; concurrent execution of a parallel group needs an async effect handle
that the public surface does not expose yet.

## Cancellation

A queued `interrupt` (or kernel dispatch cancellation) aborts the request's
signal at the next provider event and stops the crossing, emitting
`agent_cancelled` with the partial text. The interrupt is consumed by the
cancelled turn, so the next prompt runs normally.
