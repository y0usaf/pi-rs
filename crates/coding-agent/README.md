# pi-coding-agent

Pi's coding agent ([`@earendil-works/pi-coding-agent`](https://github.com/earendil-works/pi)) without the interactive TUI, as the `pi-rs` binary. It runs print mode (`pi-rs -p "..."`) on [pi-durable](../durable)'s harness and reads the same files as `pi`: settings, `auth.json`, `models.json`, AGENTS.md / CLAUDE.md, SYSTEM.md, APPEND_SYSTEM.md and skills.

It won't: run the TUI, `--mode json` or `--mode rpc`; load extensions, prompt templates, themes or MCP servers; log in (it reads the credentials `pi` stores, and refreshes Anthropic and OpenAI Codex OAuth tokens); compact a conversation or recover from a cut reply; fork or branch sessions.

The behaviour follows the upstream TypeScript at `be14d9732`, and the parity scripts below compare the two on the same inputs.

## Try it

```sh
cargo run -p pi-coding-agent --bin pi-rs -- -p --model anthropic/claude-sonnet-4-5 "list the files here"
echo "summarize this" | cargo run -p pi-coding-agent --bin pi-rs -- @notes.md
```

Without `-p` it still runs print mode, since there is no TUI. `--continue` (`-c`) goes on with the newest session for the working directory; `--no-session` keeps nothing.

## Sessions

Each run is one pi-durable conversation in a SQLite file, `<agentDir>/sessions/--<cwd>--/<uuidv7>.db` (or under `--session-dir`, `$PI_CODING_AGENT_SESSION_DIR`, or the `sessionDir` setting). `--session FILE` opens that file. These are not upstream's JSONL session files, and `pi` cannot open them.

## What matches upstream, and how it is checked

`parity/run.sh` (with `PI_TS` pointing at an upstream checkout with `node_modules` installed, and the models catalog hydrated) runs each check against the TypeScript and against this crate:

- tools (`tools.json`): `read`, `write`, `edit`, `bash`, `grep`, `find` and `ls` on the same files, compared on the exact text, details and error messages they return;
- model resolution (`models.json`): `--model`, `--provider`, `:thinking` suffixes, `--models` scopes and their warnings;
- the system prompt (`prompt.json`): the tools list and rules, context files up the directory tree (with worktree shadowing), SYSTEM.md and APPEND_SYSTEM.md with project trust, `--system-prompt` and `--append-system-prompt`, and skills discovery with frontmatter, ignore files and name collisions;
- print mode end to end (`e2e.json`): `pi -p` and `pi-rs -p` against one scripted OpenAI-compatible server, compared on every request they send (system prompt, tool declarations, messages, tool results) and on stdout, stderr and the exit code.

## Where it differs

- Invalid tool arguments and unknown tools: pi-durable's harness answers these itself, as `<harness>\n[error] ...\n</harness>` and `Tool X is not available`, where upstream sends the bare validation message and `Tool X not found`.
- A reply cut by the output limit: upstream drops it and tries compact-and-retry, then prints nothing; pi-rs prints the partial reply and exits 1 with the truncation message.
- HTTP errors from OpenAI-compatible servers print the whole JSON body (a pi-ai difference, listed in `crates/ai/parity/known.json`), and openai-completions tool declarations carry `strict: false`.
- Skill frontmatter is read by a small YAML reader covering what skills use (plain, quoted and block scalars, booleans); other YAML in a skill file is skipped rather than rejected.
- The docs paths in the system prompt name `$PI_PACKAGE_DIR`, or the directory of the `pi-rs` binary, since there is no npm package.
- `fd` and `rg` are used from the agent's bin directory or `PATH`; they are not downloaded.
