# OpenAI Chat Completions protocol fixtures

Replay fixtures for `protocols::openai_completions` against the spec
(`ref/pi` @ `c5582102`, pi v0.79.0, `providers/openai-completions.ts`).

Provenance:

- `replay_basic.sse` — hand-built to OpenAI's documented Chat
  Completions streaming shapes (`chat.completion.chunk` deltas with
  `reasoning_content`, text content, split `tool_calls` argument
  frames, a trailing usage-only chunk, and the `[DONE]` sentinel),
  matching the exact chunk fields `openai-completions.ts` consumes.
  **Pending:** replacement with a transcript captured live from
  pi v0.79.0 once credentials are available (tracked in PLAN.md).
- `replay_basic.message.json` / `replay_basic.events.json` — expected
  final `AssistantMessage` and event sequence, hand-derived by
  executing the spec's `streamOpenAICompletions` handler logic over the
  transcript. Events are recorded without their `partial`/`message`
  snapshots; `usage.cost` is zeroed in the fixture and computed by the
  harness via `calculate_cost` (cost math is pinned separately in
  `pi-rs-ai-types`).
- `params_apikey.request.json` — expected
  `ChatCompletionCreateParamsStreaming` body, hand-derived from
  `buildParams`/`convertMessages`/`convertTools` for the full-featured
  request in `tests/openai_completions.rs` (developer-role system
  prompt, same-model thinking replayed as `reasoning_content` via its
  signature, tool-call argument stringification, tool result folding,
  OpenAI-style `reasoning_effort`, tool_choice, `store: false` and
  streaming usage).
