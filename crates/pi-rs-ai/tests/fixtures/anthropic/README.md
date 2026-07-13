# Anthropic protocol fixtures

`replay_basic.sse` is a deterministic `/v1/messages` stream transcript covering
thinking, text, tool blocks, ping, signatures, split JSON deltas, and terminal
usage. It is local protocol-mechanism input; the exhaustive reviewed request and
stream expectations live in `tests/anthropic-parity/{cases,oracle}.json` and are
replayed by `crates/pi-rs-ai/tests/anthropic_parity.rs`.
