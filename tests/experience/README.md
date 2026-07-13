# Canonical experience fixture v1

`canonical-v1.json` is the complete bounded Pi-experience oracle selected by
PLAN 0.2. It was reviewed from Pi v0.79.0 at
`c5582102f51b143fadc05180e0f8aed050e923b3`, with truecolor enabled and images
and OSC 8 hyperlinks disabled. It intentionally covers only:

- startup;
- prompt editing;
- streaming and visible/hidden thinking;
- one tool call/result;
- steering and follow-up queues;
- cancellation;
- one selector and one confirmation dialog;
- an explicitly resumed session.

The `source` and `from` fields preserve the reviewed observation provenance.
Selected cells came from these pinned capture checkpoints:

| Source capture | Selected checkpoints |
|---|---|
| `basic-turn.pi.json` | `startup` |
| `editor-turn.pi.json` | `typed-wrap`, `moved` |
| `provider-turn.pi.json` | `working`, `streaming`, `tool-streaming`, `tool-pending`, `tool-executed`, `cancel-streaming`, `cancelled` |
| `shell-turn.pi.json` | `thinking-hidden`, `thinking-visible`, `steer-queued`, `followup-queued` |
| `selector-turn.pi.json` | `selector-open`, `selector-filter`, `selector-cancel` |
| `extension-ui-turn.pi.json` | `commands-confirm`, `commands-notify` |
| `resume-turn.pi.json` | `startup` after opening the explicit session fixture |

Unselected adjacent behavior is not a pi-rs requirement.

## Format

The top-level `format` and `version` identify the schema. Styles are deduplicated
in the `styles` palette. Each frame records `[columns, rows]`, cursor state,
trimmed glyph rows, style spans, and optional wide-cell origins.

Glyph rows use one character per terminal cell:

- `░` = untouched empty cell;
- `␠` = a written space;
- missing row tails and missing bottom rows = untouched empty cells.

A style span is `[row, start, end-exclusive, style-name]`. Input is represented
as reviewable UTF-8 text, named terminal keys, or exact hexadecimal bytes. A
step's `from` field names the source checkpoint/state to which its input applies;
it permits a small observation set without silently claiming that omitted
adjacent frames are required.

The Rust-only checker is:

```console
ui-diff --check tests/experience/canonical-v1.json
ui-diff --compare expected.json actual.json
ui-diff --self-test tests/experience/canonical-v1.json
```

`--check` validates the closed coverage set and requires parse → canonical
pretty-print to reproduce the file byte-for-byte. `--compare` reports the first
input byte or terminal cell mismatch. `--self-test` proves both negative
controls. Normal checks never execute Pi, Node, Bun, or TypeScript.
