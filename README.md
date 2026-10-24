# pi-rs

Pi's TypeScript packages ([`earendil-works/pi`](https://github.com/earendil-works/pi) 1.1.0) ported to Rust, one crate per package, for programs written in Rust that don't want a Node process next to them.

`nix run github:y0usaf/pi-rs` runs `pi-rs`, Pi's coding agent on pi-durable: print mode and a first port of the interactive mode, reading the same settings and credentials as `pi` (see [its README](crates/coding-agent/README.md)).

It won't: bring an async runtime; follow redirects or speak through proxies; talk to every provider Pi does (three wire formats are real, the rest are yours to add behind the `Api` trait); store your credentials (you hand it keys or a resolver, and it never reads or writes a token file); run on Windows.

## Crates

| Pi package | crate | state |
| --- | --- | --- |
| `@earendil-works/pi-ai` | `crates/ai` (`pi-ai`) | here; a subset of providers and logins, checked against the TypeScript by `crates/ai/parity` |
| `@earendil-works/chord` | `crates/chord` (`pi-chord`) | here; only the replicated state (delta ops, tracker, state) |
| `@earendil-works/pi-env` | `crates/env` (`pi-env`) | here; `ExecutionEnv` and the local implementation, no remote SSH environment |
| `@earendil-works/pi-durable` | `crates/durable` (`pi-durable`) | here; see [its README](crates/durable/README.md) |
| `@earendil-works/pi-tui` | `crates/tui` (`pi-tui`) | here; components checked against upstream renders by `crates/tui/tests` |
| `@earendil-works/pi-coding-agent` | `crates/coding-agent` (`pi-coding-agent`, binary `pi-rs`) | partly; see [its README](crates/coding-agent/README.md) |
| `pi-agent-core`, `pi-codemode`, `pi-mcp`, `pi-client`, `pi-protocol`, `pi-server`, `pi-telemetry`, `pi-evals` | | not yet |

`crates/rt` (`pi-rt`) is not a Pi package. It holds what Node gives Pi for free: `CancelToken` in place of `AbortSignal`, and the single-threaded turn that makes threads take turns like JS tasks between `await`s.

The env edge is reversed from Pi. Pi keeps `ExecutionEnv` and its local Node implementation in pi-durable (`packages/durable/src/env`), and its pi-env adds remote environments on top, so Pi's env depends on durable. Here pi-env holds the trait and the local implementation, and durable depends on env.

pi-durable re-exports the crates under it (`pi_durable::{ai, chord, env, cancel, turn}`), so code written against pi-durable-rs keeps its paths.

## Chat window

`examples/chat` (`pi-chat`) is a minimal [GPUI](https://github.com/gpui-ce/gpui-ce) window on pi-durable, built outside the workspace so the crates stay free of GPUI. It keeps its conversations in a SQLite file and draws the open one as the store changes.

```
nix run github:y0usaf/pi-rs#chat
```

Kill it in the middle of a reply, with `kill -9` if you like, and run it again: it reopens the latest conversation, and the interrupted reply is asked for again and finishes. The file is `$XDG_STATE_HOME/pi-chat/chat.db` unless you pass a path. It needs Vulkan, and has only been run on Wayland.

Under the message box, the model picker lists pi-ai's faux model and every model whose provider has its key in the environment, such as `AI_GATEWAY_API_KEY`; models that reason get a thinking level beside it. Both are stored with the conversation. `PI_CHAT_MODEL=PROVIDER/MODEL` picks the model a new chat starts with.

| Key | Does |
| --- | --- |
| Ctrl+B | show or hide the chats |
| Ctrl+N | start a new chat |
| Ctrl+M | pick a model; type to filter |
| Ctrl+T | step through thinking levels |
| Esc | stop the reply |
| Ctrl+= / Ctrl+- / Ctrl+0 | zoom in, out, back to 100%, kept in `$XDG_STATE_HOME/pi-chat/zoom` |

## Build and checks

```
nix build
nix flake check
```

`nix flake check` builds the workspace with and without provider features, runs `clippy` and `rustfmt`, the loopback end-to-end examples, both parity checks, and the chat window's build.

## Licence

MIT, as Pi.
