# pi-rs

Pi's TypeScript packages ([`earendil-works/pi`](https://github.com/earendil-works/pi) 1.1.0) ported to Rust, one crate per package, for programs written in Rust that don't want a Node process next to them.

It won't: ship a CLI or a UI; bring an async runtime; follow redirects or speak through proxies; talk to every provider Pi does (three wire formats are real, the rest are yours to add behind the `Api` trait); store your credentials (you hand it keys or a resolver, and it never reads or writes a token file); run on Windows.

## Crates

| Pi package | crate | state |
| --- | --- | --- |
| `@earendil-works/pi-ai` | `crates/ai` (`pi-ai`) | here; a subset of providers and logins, checked against the TypeScript by `crates/ai/parity` |
| `@earendil-works/chord` | `crates/chord` (`pi-chord`) | here; only the replicated state (delta ops, tracker, state) |
| `@earendil-works/pi-env` | `crates/env` (`pi-env`) | here; `ExecutionEnv` and the local implementation, no remote SSH environment |
| `@earendil-works/pi-durable` | `crates/durable` (`pi-durable`) | here; see [its README](crates/durable/README.md) |
| `pi-agent-core`, `pi-coding-agent`, `pi-tui`, `pi-codemode`, `pi-mcp`, `pi-client`, `pi-protocol`, `pi-server`, `pi-telemetry`, `pi-evals` | | not yet |

`crates/rt` (`pi-rt`) is not a Pi package. It holds what Node gives Pi for free: `CancelToken` in place of `AbortSignal`, and the single-threaded turn that makes threads take turns like JS tasks between `await`s.

The env edge is reversed from Pi. Pi keeps `ExecutionEnv` and its local Node implementation in pi-durable (`packages/durable/src/env`), and its pi-env adds remote environments on top, so Pi's env depends on durable. Here pi-env holds the trait and the local implementation, and durable depends on env.

pi-durable re-exports the crates under it (`pi_durable::{ai, chord, env, cancel, turn}`), so code written against pi-durable-rs keeps its paths.

## Build and checks

```
nix build
nix flake check
```

`nix flake check` builds the workspace with and without provider features, runs `clippy` and `rustfmt`, the loopback end-to-end examples, and both parity checks.

## Licence

MIT, as Pi.
