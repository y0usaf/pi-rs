# pi-coding-agent

Pi's coding agent ([`@earendil-works/pi-coding-agent`](https://github.com/earendil-works/pi)) as the `pi-rs` binary. It runs print mode (`pi-rs -p "..."`), the two headless modes (`--mode json`, `--mode rpc`) and a first port of the interactive mode on [pi-durable](../durable)'s harness and reads the same files as `pi`: settings, `auth.json`, `models.json`, AGENTS.md / CLAUDE.md, SYSTEM.md, APPEND_SYSTEM.md and skills.

Settings are upstream's own layer: the global `<agentDir>/settings.json` and the trusted project's `.pi/settings.json`, deep-merged with the project winning, every schema key with upstream's default, and setters that write back through a lock, replacing only the fields the session changed so keys this port does not know survive. pi-durable's auto-compaction follows them: `compaction.enabled`, `reserveTokens`, `keepRecentTokens` and the per-model overrides, with upstream's 16384 / 20000 when unset.

It won't: show `/tree`, forking or settings screens, or the model and thinking selectors; run any slash command but `/quit`, `/new`, `/name`, `/session`, `/resume`, `/hotkeys` and `/compact`; answer most of the RPC commands; load extensions, prompt templates, themes or MCP servers; log in (it reads the credentials `pi` stores, and refreshes Anthropic and OpenAI Codex OAuth tokens); fork or branch sessions; export a session as HTML.

The flags for what it won't do, `--export`, `--fork`, `--extension`, `--prompt-template`, `--theme`, `--use-theme`, `--tui-mode` and `--verbose`, are refused before anything starts: `pi-rs: --export is not implemented yet`, exit 1. The flags that turn a resource off — `--no-extensions`, `--no-mcp`, `--no-themes`, `--no-prompt-templates` — and `--offline`, which would skip a model-catalog refresh this port never makes, are accepted and change nothing. A flag no option or extension defines is `Error: Unknown option: --foo`.

Its behaviour and `pi-ai`'s follow the upstream TypeScript at `bdb1517`; `pi-durable`'s follows `f1b2e77f` ([its README](../durable/README.md)). The parity scripts below compare the two on the same inputs.

## Try it

```sh
cargo run -p pi-coding-agent --bin pi-rs -- -p --model anthropic/claude-sonnet-4-5 "list the files here"
echo "summarize this" | cargo run -p pi-coding-agent --bin pi-rs -- @notes.md
```

Without `-p`, on a terminal, it opens the interactive mode (below). `--continue` (`-c`) goes on with the newest session for the working directory; `--session-id NAME` names a new session `NAME.db` and reopens it when it is there; `--no-session` keeps nothing.

`--list-models [search]` prints the models whose provider has auth, as `pi --list-models` does: provider, model, context, max-out, thinking and images, sorted by provider then model id, fuzzy-matched against the search pattern, then exits.

## Interactive mode

`cargo run -p pi-coding-agent --bin pi-rs` (or `pi-rs -c` to go on with the last session) opens a first port of upstream's interactive mode on pi-tui: the editor, the user's messages on their blue background, the assistant's reply streamed as Markdown, thinking in italics, one row per tool call (pending, green or red, with the built-in tools' previews and the bash timer), the working spinner and the two-line footer (cwd and git branch; tokens, cost, context use and model). Keys: Enter sends (while a reply runs it steers), Escape aborts the reply, Ctrl+C clears the editor and twice quits, Ctrl+D quits on an empty editor, Ctrl+O expands tool output. Typing `/` lists the builtin commands and `@` completes file paths through `fd`. `/quit`, `/new` (a new session file in the same folder), `/name [text]`, `/session`, `/resume` (the session picker), `/hotkeys`, `/compact [instructions]`, `/tree`, `/fork` and `/clone` work; every other builtin answers that it is not available in pi-rs yet, and an unknown `/x` goes to the model as a message, as upstream does. The screen is built from pi-durable's agent events (`Conversation::events`), so a session killed mid-reply (even `kill -9`) reopens with `-c` showing what was stored, and the harness resumes the unfinished turn. Colors are upstream's dark theme, truecolor when `COLORTERM` says so.

`/tree` is upstream's session tree (its filter keys, search, folding and branch markers): picking an entry moves the session's leaf there, which pi-durable spells as a forked conversation, and a picked user message reopens in the editor. Two things the tree shows upstream are missing: entry labels (and the label keys and timestamps that go with them) and branch summaries, so picking an entry never asks whether to summarize the branch.

`/fork` lists the session's user messages and `/clone` takes the current leaf; both write a new session file holding the active path up to that point and switch the screen to it, as upstream's branch files do, and `/fork` puts the picked message back in the editor. Each says so in the transcript and names the file it wrote, where upstream says "Forked to new session" or "Cloned to new session" on the status line.

Not there yet: the remaining slash commands (`/model`, `/thinking`, `/settings` and the rest), `!` bash, image display, syntax highlighting in code blocks, the light theme, the editor's embedded status, and upstream's keybinding configuration. A resumed session from another folder keeps the current folder, where upstream switches to the session's own; `/resume` reports in the transcript, where upstream uses the status line.

## Headless modes

`--mode json` is print mode with the session's events as one JSON object per line on stdout instead of the final text, so a client sees the assistant's text as it streams: `pi-rs --mode json -p "list the files here"`. `--mode rpc` takes the same commands upstream's RPC mode does as JSON lines on stdin, and answers them on that one stdout stream beside the events, so a client reads the reply and the stream it caused in order. It answers `prompt`, `steer`, `follow_up`, `abort`, `new_session`, `get_state`, `set_model`, `get_available_models`, `set_thinking_level`, `get_available_thinking_levels`, `compact`, `get_messages`, `get_last_assistant_text`, `get_entries`, `get_commands` and `set_session_name`; every other command upstream carries answers `pi-rs: <command> is not implemented in RPC mode`, and a type no mode has answers `Unknown command: <type>`. Slash text in a `prompt` is a prompt, since no extension or template here claims the name, as upstream's RPC mode does.

## Sessions

Each run is one pi-durable conversation in a SQLite file, `<agentDir>/sessions/--<cwd>--/<uuidv7>.db` (or under `--session-dir`, `$PI_CODING_AGENT_SESSION_DIR`, or the `sessionDir` setting). `--session FILE` opens that file, `--session-id NAME` writes to `NAME.db` next to it, `--continue` reopens the newest branch, and `--resume` picks from the sessions in this folder or in all of them. A branch of the tree is another conversation in the same file, forked at the entry the session moved to. `--fork <path|id>` copies a session into a new file under a new id, refusing one whose run is still in flight, and leaves the source untouched. These are not upstream's JSONL session files, and `pi` cannot open them. A session's display name and the session it was forked from are one session-scope document (`pi.session-info`) inside the file, so a copy of the file carries them.

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
- `--mode json` writes no `session` header line, because the header upstream writes describes its JSONL session file and these sessions are SQLite. It also carries pi-durable's own events beside upstream's (`agent_changed`, `submission`, `usage_changed`, `inbox_update`, `deferred_poll`), and fewer `message_update` lines: the harness folds a message's first and last commit into its `message_start` and `message_end`, so `text_start`, `text_end` and a tool call's `toolcall_start` / `toolcall_delta` / `toolcall_end` arrive only when the block lands mid-message, and `tool_execution_update` only for a tool that reports partial output, which no built-in does.
- Assistant messages in that stream carry pi-ai's fields: `durationMs` and `responseId`, no `thinkingLevel`, and a tool call with no `partialArgs` or `streamIndex`; `toolsAdded` carries no `constrainedSampling`, and a tool call's `structuredContent` is the tool's own details object.
- `--list-models` prints the auth each provider has and the catalog pi-ai ships, so its rows for a model match `pi --list-models`'s only while both binaries carry the same catalog: a different `pi` version can list another context window, or a model this crate lacks.
