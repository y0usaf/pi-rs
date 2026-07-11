# Serde fixtures

These pin the JSON shapes of `pi-rs-ai-types` against the spec
(`ref/pi` @ `c5582102`, pi v0.79.0). Every fixture must round-trip
(deserialize → serialize) to an identical `serde_json::Value`, modulo JSON
number representation (the harness normalizes all numbers to f64).

Provenance:

- `message_assistant_{tooluse,stop,error,aborted}.json`,
  `message_user_{string,blocks}.json`, `message_toolresult{,_details}.json` —
  recorded by pi from real sessions (`~/.pi/agent/sessions`). The recording
  pi was v0.80.3, which adds a `usage.reasoning` field that does not exist in
  the v0.79.0 spec; it was stripped to match the pinned spec.
- `model_*.json`, `images_model_*.json` — transcribed verbatim from the
  spec's `models.generated.ts` / `image-models.generated.ts` rows.
- `message_user_image.json`, `message_toolresult_image.json`,
  `message_assistant_diagnostics.json`, `message_assistant_google.json`,
  `assistant_images_openrouter.json`, `events_stream.json`,
  `context_full.json` — hand-built to the spec's `types.ts` /
  `utils/diagnostics.ts` unions to cover fields absent from the recorded
  sessions (images, diagnostics, `thoughtSignature`, `textSignature`,
  redacted thinking, the full `AssistantMessageEvent` vocabulary).
