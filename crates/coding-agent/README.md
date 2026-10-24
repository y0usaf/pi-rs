# pi-coding-agent

Pi's coding agent ([`@earendil-works/pi-coding-agent`](https://github.com/earendil-works/pi)) as the `pi-rs` binary. It runs print mode (`pi-rs -p "..."`) and a first port of the interactive mode on [pi-durable](../durable)'s harness and reads the same files as `pi`: settings, `auth.json`, `models.json`, AGENTS.md / CLAUDE.md, SYSTEM.md, APPEND_SYSTEM.md and skills.

Settings are upstream's own layer: the global `<agentDir>/settings.json` and the trusted project's `.pi/settings.json`, deep-merged with the project winning, every schema key with upstream's default, and setters that write back through a lock, replacing only the fields the session changed so keys this port does not know survive. pi-durable's auto-compaction follows them: `compaction.enabled`, `reserveTokens`, `keepRecentTokens` and the per-model overrides, with upstream's 16384 / 20000 when unset.

It won't: show upstream's session picker, `/tree`, forking or settings screens, or the model and thinking selectors; run any slash command but `/quit`, `/new`, `/session`, `/hotkeys` and `/compact`; run `--mode json` or `--mode rpc`; load extensions, prompt templates, themes or MCP servers; log in (it reads the credentials `pi` stores, and refreshes Anthropic and OpenAI Codex OAuth tokens); fork or branch sessions; export a session as HTML.

The flags for what it won't do, `--export`, `--fork`, `--resume`, `--name`, `--extension`, `--prompt-template`, `--theme`, `--use-theme`, `--tui-mode` and `--verbose`, are refused before anything starts: `pi-rs: --export is not implemented yet`, exit 1. The flags that turn a resource off — `--no-extensions`, `--no-mcp`, `--no-themes`, `--no-prompt-templates` — and `--offline`, which would skip a model-catalog refresh this port never makes, are accepted and change nothing. A flag no option or extension defines is `Error: Unknown option: --foo`.

Its behaviour and `pi-ai`'s follow the upstream TypeScript at `bdb1517`; `pi-durable`'s follows `f1b2e77f` ([its README](../durable/README.md)). The parity scripts below compare the two on the same inputs.

## Try it

```sh
cargo run -p pi-coding-agent --bin pi-rs -- -p --model anthropic/claude-sonnet-4-5 "list the files here"
echo "summarize this" | cargo run -p pi-coding-agent --bin pi-rs -- @notes.md
```

Without `-p`, on a terminal, it opens the interactive mode (below). `--continue` (`-c`) goes on with the newest session for the working directory; `--session-id NAME` names a new session `NAME.db` and reopens it when it is there; `--no-session` keeps nothing.

`--list-models [search]` prints the models whose provider has auth, as `pi --list-models` does: provider, model, context, max-out, thinking and images, sorted by provider then model id, fuzzy-matched against the search pattern, then exits.

## Interactive mode

`cargo run -p pi-coding-agent --bin pi-rs` (or `pi-rs -c` to go on with the last session) opens a first port of upstream's interactive mode on pi-tui: the editor, the user's messages on their blue background, the assistant's reply streamed as Markdown, thinking in italics, one row per tool call (pending, green or red, with the built-in tools' previews and the bash timer), the working spinner and the two-line footer (cwd and git branch; tokens, cost, context use and model). Keys: Enter sends (while a reply runs it steers), Escape aborts the reply, Ctrl+C clears the editor and twice quits, Ctrl+D quits on an empty editor, Ctrl+O expands tool output. Typing `/` lists the builtin commands and `@` completes file paths through `fd`. `/quit`, `/new` (a new context in the same session file), `/session`, `/hotkeys` and `/compact [instructions]` work; every other builtin answers that it is not available in pi-rs yet, and an unknown `/x` goes to the model as a message, as upstream does. The screen is built from pi-durable's agent events (`Conversation::events`), so a session killed mid-reply (even `kill -9`) reopens with `-c` showing what was stored, and the harness resumes the unfinished turn. Colors are upstream's dark theme, truecolor when `COLORTERM` says so.

Not there yet: the session picker (`--resume`), `/tree`, forking, settings screens, model and thinking selectors, the remaining slash commands (`/model`, `/thinking`, `/tree`, `/settings`, `/fork`, `/resume` and the rest), `!` bash, image display, syntax highlighting in code blocks, the light theme, the editor's embedded status, and upstream's keybinding configuration.

## Sessions

Each run is one pi-durable conversation in a SQLite file, `<agentDir>/sessions/--<cwd>--/<uuidv7>.db` (or under `--session-dir`, `$PI_CODING_AGENT_SESSION_DIR`, or the `sessionDir` setting). `--session FILE` opens that file, `--session-id NAME` writes to `NAME.db` next to it. These are not upstream's JSONL session files, and `pi` cannot open them.

## What matches upstream, and how it is checked

`parity/run.sh` (with `PI_TS` pointing at an upstream checkout with `node_modules` installed, and the models catalog hydrated) runs each check against the TypeScript and against this crate:

- tools (`tools.json`): `read`, `write`, `edit`, `bash`, `grep`, `find` and `ls` on the same files, compared on the exact text, details and error messages they return;
- model resolution (`models.json`): `--model`, `--provider`, `:thinking` suffixes, `--models` scopes and their warnings;
- the system prompt (`prompt.json`): the tools list and rules, context files up the directory tree (with worktree shadowing), SYSTEM.md and APPEND_SYSTEM.md with project trust, `--system-prompt` and `--append-system-prompt`, and skills discovery with frontmatter, ignore files and name collisions;
- print mode end to end (`e2e.json`): `pi -p` and `pi-rs -p` against one scripted OpenAI-compatible server, compared on every request they send (system prompt, tool declarations, messages, tool results) and on stdout, stderr and the exit code.

## Where it differs

- HTTP errors from OpenAI-compatible servers print the whole JSON body (a pi-ai difference, listed in `crates/ai/parity/known.json`), and openai-completions tool declarations carry `strict: false`.
- Skill frontmatter is read by a small YAML reader covering what skills use (plain, quoted and block scalars, booleans); other YAML in a skill file is skipped rather than rejected.
- The docs paths in the system prompt name `$PI_PACKAGE_DIR`, or the directory of the `pi-rs` binary, since there is no npm package.
- `fd` and `rg` are used from the agent's bin directory or `PATH`; they are not downloaded.
- `--list-models` prints the auth each provider has and the catalog pi-ai ships, so its rows for a model match `pi --list-models`'s only while both binaries carry the same catalog: a different `pi` version can list another context window, or a model this crate lacks.
