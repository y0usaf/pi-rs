//! Port of `cli/args.ts` — CLI argument parsing and help.
//!
//! Landed subset (recorded): the bare-core entry points (`--login`,
//! `--list-models [search]`, `--provider/--model/--api-key/--thinking`,
//! `--help`, `--version`, positional messages) plus the session
//! selections `--continue`/`-c` and `--session <path|id>` (PLAN 6.2).
//! PLAN 10 adds the pinned non-interactive surface: `--mode text|json|rpc`,
//! `--print`/`-p`, `--export <file>`, `--system-prompt`,
//! `--append-system-prompt`, `--session-dir`, `--name`, `@file` args,
//! and unknown `--flags` collected for extensions (spec: cli/args.ts).
//! Parsing semantics for the landed flags match the spec exactly;
//! `--login` is a pi-rs surface (divergence 3): pi's login lives in
//! interactive `/login`, which the bare core (doctrine 06) must reach
//! without a frontend.

use pi_rs_ai_types::ModelThinkingLevel;
use serde_json::Value;

use crate::config::{APP_NAME, CONFIG_DIR_NAME, ENV_AGENT_DIR, VERSION};
use crate::core::model_resolver::parse_thinking_level;

/// Spec: the `diagnostics` entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub is_error: bool,
    pub message: String,
}

/// Spec: `Args` (landed subset).
#[derive(Debug, Default)]
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
    pub thinking: Option<ModelThinkingLevel>,
    pub help: bool,
    pub version: bool,
    /// `--mode <text|json|rpc>`; invalid values are silently ignored
    /// (spec: parseArgs keeps only valid modes).
    pub mode: Option<String>,
    /// `--list-models [search]`: `Some(None)` without a pattern.
    pub list_models: Option<Option<String>>,
    /// `--login [provider]` (pi-rs surface; default provider anthropic).
    pub login: Option<Option<String>>,
    /// `--continue` / `-c`: continue the most recent session.
    pub continue_recent: bool,
    /// `--session <path|id>`: use a specific session file or partial id.
    pub session: Option<String>,
    /// `--resume` / `-r`: select a session to resume via the selector.
    pub resume: bool,
    /// `--session-dir <dir>`: session storage directory.
    pub session_dir: Option<String>,
    /// `--name` / `-n`: session display name.
    pub name: Option<String>,
    /// `--print` / `-p`: non-interactive text mode (consumes the next
    /// non-flag token as the first message when present).
    pub print: bool,
    /// `--export <file>`: export a session file to HTML and exit.
    pub export: Option<String>,
    /// `@file` arguments, in CLI order (spec: `fileArgs`).
    pub file_args: Vec<String>,
    /// Explicit `--extension` / `-e` Lua sources, in CLI order.
    pub extensions: Vec<String>,
    /// `--no-extensions` / `-ne`: disable discovery/configured sources only.
    pub no_extensions: bool,
    /// `--approve`/`-a` and `--no-approve`/`-na`: explicit project trust.
    pub project_trust_override: Option<bool>,
    /// Unknown `--flags` collected for extensions (spec: `unknownFlags`);
    /// `--flag=value` and `--flag value` keep the value.
    pub unknown_flags: Vec<(String, Value)>,
    pub messages: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Spec: `parseArgs(args)` for the landed flags — including the
/// spec's optional-value rule for `--list-models` (a following token
/// that is not a flag and not an `@file` is the search pattern).
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args {
    let args: Vec<String> = args.into_iter().collect();
    let mut result = Args::default();

    let optional_value = |i: usize| -> Option<&String> {
        args.get(i + 1)
            .filter(|next| !next.starts_with('-') && !next.starts_with('@'))
    };

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--help" | "-h" => result.help = true,
            "--version" | "-v" => result.version = true,
            "--continue" | "-c" => result.continue_recent = true,
            "--resume" | "-r" => result.resume = true,
            "--approve" | "-a" => result.project_trust_override = Some(true),
            "--no-approve" | "-na" => result.project_trust_override = Some(false),
            "--no-extensions" | "-ne" => result.no_extensions = true,
            "--no-session" | "--no-tools" | "-nt" | "--no-builtin-tools" | "-nbt"
            | "--no-skills" | "-ns" | "--no-prompt-templates" | "-np" | "--no-themes"
            | "--no-context-files" | "-nc" | "--offline" | "--verbose" => {
                // Recognized spec flags whose product surfaces are not yet
                // landed; parse-and-ignore keeps the argument matrix stable
                // (the surfaces they gate are PLAN 7/9.x rows).
            }
            "--print" | "-p" => {
                result.print = true;
                let next = args.get(i + 1);
                if let Some(next) = next {
                    let consume = !next.starts_with('@')
                        && (!next.starts_with('-') || next.starts_with("---"));
                    if consume {
                        result.messages.push(next.clone());
                        i += 1;
                    }
                }
            }
            "--mode" if i + 1 < args.len() => {
                i += 1;
                match args[i].as_str() {
                    "text" | "json" | "rpc" => result.mode = Some(args[i].clone()),
                    // Spec: invalid --mode values are silently ignored.
                    _ => {}
                }
            }
            "--export" if i + 1 < args.len() => {
                i += 1;
                result.export = Some(args[i].clone());
            }
            "--system-prompt" if i + 1 < args.len() => {
                i += 1;
                result.system_prompt = Some(args[i].clone());
            }
            "--append-system-prompt" if i + 1 < args.len() => {
                i += 1;
                result.append_system_prompt.push(args[i].clone());
            }
            "--session-dir" if i + 1 < args.len() => {
                i += 1;
                result.session_dir = Some(args[i].clone());
            }
            "--name" | "-n" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.name = Some(args[i].clone());
                } else {
                    result.diagnostics.push(Diagnostic {
                        is_error: true,
                        message: "--name requires a value".to_owned(),
                    });
                }
            }
            "--extension" | "-e" if i + 1 < args.len() => {
                i += 1;
                result.extensions.push(args[i].clone());
            }
            "--session" if i + 1 < args.len() => {
                i += 1;
                result.session = Some(args[i].clone());
            }
            "--provider" if i + 1 < args.len() => {
                i += 1;
                result.provider = Some(args[i].clone());
            }
            "--model" if i + 1 < args.len() => {
                i += 1;
                result.model = Some(args[i].clone());
            }
            "--api-key" if i + 1 < args.len() => {
                i += 1;
                result.api_key = Some(args[i].clone());
            }
            "--thinking" if i + 1 < args.len() => {
                i += 1;
                let level = args[i].as_str();
                match parse_thinking_level(level) {
                    Some(parsed) => result.thinking = Some(parsed),
                    None => result.diagnostics.push(Diagnostic {
                        is_error: false,
                        message: format!(
                            "Invalid thinking level \"{level}\". Valid values: off, minimal, low, medium, high, xhigh, max"
                        ),
                    }),
                }
            }
            "--list-models" => match optional_value(i) {
                Some(pattern) => {
                    result.list_models = Some(Some(pattern.clone()));
                    i += 1;
                }
                None => result.list_models = Some(None),
            },
            "--login" => match optional_value(i) {
                Some(provider) => {
                    result.login = Some(Some(provider.clone()));
                    i += 1;
                }
                None => result.login = Some(None),
            },
            _ if arg.starts_with("--") => {
                // Spec: unknown `--flags` are extension flags. `--flag=value`
                // and `--flag value` keep the value; bare flags are boolean.
                let eq_index = arg.find('=');
                let (name, value) = match eq_index {
                    Some(index) => (&arg[2..index], Some(&arg[index + 1..])),
                    None => {
                        let name = &arg[2..];
                        let next = args.get(i + 1);
                        let consumed = next.is_some_and(|next| {
                            !next.starts_with('-') && !next.starts_with('@')
                        });
                        if consumed {
                            i += 1;
                            (name, Some(args[i].as_str()))
                        } else {
                            (name, None)
                        }
                    }
                };
                let value = match value {
                    Some(value) => Value::String(value.to_owned()),
                    None => Value::Bool(true),
                };
                result.unknown_flags.push((name.to_owned(), value));
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                result.diagnostics.push(Diagnostic {
                    is_error: true,
                    message: format!("Unknown option: {arg}"),
                });
            }
            _ if arg.starts_with('@') => {
                result.file_args.push(arg[1..].to_owned());
            }
            _ => result.messages.push(arg.to_owned()),
        }
        i += 1;
    }

    result
}

/// Spec: `printHelp()` — the full pinned surface (PLAN 10/11 acceptance).
/// Extension flags render only when the runtime knows them; the bare
/// launcher prints the static text.
pub fn help_text(extension_flags: &[String]) -> String {
    let extension_flags_text = if extension_flags.is_empty() {
        String::new()
    } else {
        format!(
            "\n{}\n{}\n",
            "Extension CLI Flags:".to_owned(),
            extension_flags
                .iter()
                .map(|flag| format!("  --{flag}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "{APP_NAME} - AI coding assistant with read, bash, edit, write tools

Usage:
  {APP_NAME} [options] [@files...] [messages...]

Commands:
  {APP_NAME} install <source> [-l]     Install extension source and add to settings
  {APP_NAME} remove <source> [-l]      Remove extension source from settings
  {APP_NAME} uninstall <source> [-l]   Alias for remove
  {APP_NAME} update [source|self|pi]   Update pi and installed extensions
  {APP_NAME} list                      List installed extensions from settings
  {APP_NAME} config                    Open TUI to enable/disable package resources
  {APP_NAME} <command> --help          Show help for install/remove/uninstall/update/list

Options:
  --provider <name>              Provider name (default: google)
  --model <pattern>              Model pattern or ID (supports \"provider/id\" and optional \":<thinking>\")
  --api-key <key>                API key (defaults to env vars)
  --system-prompt <text>         System prompt (default: coding assistant prompt)
  --append-system-prompt <text>  Append text or file contents to the system prompt (can be used multiple times)
  --mode <mode>                  Output mode: text (default), json, or rpc
  --print, -p                    Non-interactive mode: process prompt and exit
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial UUID
  --session-id <id>              Use exact project session ID, creating it if missing
  --fork <path|id>               Fork specific session file or partial UUID into a new session
  --session-dir <dir>            Directory for session storage and lookup
  --no-session                   Don't save session (ephemeral)
  --name, -n <name>              Set session display name
  --models <patterns>            Comma-separated model patterns for Ctrl+P cycling
                                 Supports globs (anthropic/*, *sonnet*) and fuzzy matching
  --no-tools, -nt                Disable all tools by default (built-in and extension)
  --no-builtin-tools, -nbt       Disable built-in tools by default but keep extension/custom tools enabled
  --tools, -t <tools>            Comma-separated allowlist of tool names to enable
                                 Applies to built-in, extension, and custom tools
  --exclude-tools, -xt <tools>   Comma-separated denylist of tool names to disable
                                 Applies to built-in, extension, and custom tools
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh
  --extension, -e <path>         Load an extension file (can be used multiple times)
  --no-extensions, -ne           Disable extension discovery (explicit -e paths still work)
  --skill <path>                 Load a skill file or directory (can be used multiple times)
  --no-skills, -ns               Disable skills discovery and loading
  --prompt-template <path>       Load a prompt template file or directory (can be used multiple times)
  --no-prompt-templates, -np     Disable prompt template discovery and loading
  --theme <path>                 Load a theme file or directory (can be used multiple times)
  --no-themes                    Disable theme discovery and loading
  --no-context-files, -nc        Disable AGENTS.md and CLAUDE.md discovery and loading
  --export <file>                Export session file to HTML and exit
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup (overrides quietStartup setting)
  --approve, -a                  Trust project-local files for this run
  --no-approve, -na              Ignore project-local files for this run
  --offline                      Disable startup network operations (same as PI_OFFLINE=1)
  --help, -h                     Show this help
  --version, -v                  Show version number

Extensions can register additional flags (e.g., --plan from plan-mode extension).{extension_flags_text}
Examples:
  # Interactive mode
  {APP_NAME}

  # Interactive mode with initial prompt
  {APP_NAME} \"List all .ts files in src/\"

  # Include files in initial message
  {APP_NAME} @prompt.md @image.png \"What color is the sky?\"

  # Non-interactive mode (process and exit)
  {APP_NAME} -p \"List all .ts files in src/\"

  # Multiple messages (interactive)
  {APP_NAME} \"Read package.json\" \"What dependencies do we have?\"

  # Continue previous session
  {APP_NAME} --continue \"What did we discuss?\"

  # Start a named session
  {APP_NAME} --name \"Refactor auth module\"

  # Use different model
  {APP_NAME} --provider openai --model gpt-4o-mini \"Help me refactor this code\"

  # Use model with provider prefix (no --provider needed)
  {APP_NAME} --model openai/gpt-4o \"Help me refactor this code\"

  # Use model with thinking level shorthand
  {APP_NAME} --model sonnet:high \"Solve this complex problem\"

  # Limit model cycling to specific models
  {APP_NAME} --models claude-sonnet,claude-haiku,gpt-4o

  # Limit to a specific provider with glob pattern
  {APP_NAME} --models \"github-copilot/*\"

  # Cycle models with fixed thinking levels
  {APP_NAME} --models sonnet:high,haiku:low

  # Start with a specific thinking level
  {APP_NAME} --thinking high \"Solve this complex problem\"

  # Read-only mode (no file modifications possible)
  {APP_NAME} --tools read,grep,find,ls -p \"Review the code in src/\"

  # Disable one tool while keeping the rest available
  {APP_NAME} --exclude-tools ask_question

  # Export a session file to HTML
  {APP_NAME} --export ~/{CONFIG_DIR_NAME}/agent/sessions/--path--/session.jsonl
  {APP_NAME} --export session.jsonl output.html

Environment Variables:
  ANTHROPIC_API_KEY                - Anthropic Claude API key
  ANTHROPIC_OAUTH_TOKEN            - Anthropic OAuth token (alternative to API key)
  {ENV_AGENT_DIR}            - Config directory (default: ~/{CONFIG_DIR_NAME}/agent)
  {ENV_AGENT_DIR}            - Config directory (default: ~/{CONFIG_DIR_NAME}/agent)
  PI_OFFLINE                       - Disable startup network operations when set to 1/true/yes

pi {VERSION} — run without a message in a terminal for interactive mode,
or run `pi \"prompt\"` for one-shot output.
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn list_models_optional_pattern() {
        assert_eq!(parse(&["--list-models"]).list_models, Some(None));
        assert_eq!(
            parse(&["--list-models", "opus"]).list_models,
            Some(Some("opus".to_owned()))
        );
        // A following flag is not a pattern.
        let args = parse(&["--list-models", "--version"]);
        assert_eq!(args.list_models, Some(None));
        assert!(args.version);
    }

    #[test]
    fn model_and_provider_take_values() {
        let args = parse(&["--provider", "anthropic", "--model", "opus", "hi there"]);
        assert_eq!(args.provider.as_deref(), Some("anthropic"));
        assert_eq!(args.model.as_deref(), Some("opus"));
        assert_eq!(args.messages, vec!["hi there".to_owned()]);
    }

    #[test]
    fn invalid_thinking_level_warns() {
        let args = parse(&["--thinking", "ultra"]);
        assert!(args.thinking.is_none());
        assert_eq!(args.diagnostics.len(), 1);
        assert!(!args.diagnostics[0].is_error);
    }

    #[test]
    fn session_flags_parse() {
        let args = parse(&["--continue", "hello"]);
        assert!(args.continue_recent);
        assert_eq!(args.messages, vec!["hello".to_owned()]);
        let args = parse(&["-c"]);
        assert!(args.continue_recent);
        let args = parse(&["--session", "abc123"]);
        assert_eq!(args.session.as_deref(), Some("abc123"));
        let args = parse(&["--resume"]);
        assert!(args.resume);
        let args = parse(&["-r"]);
        assert!(args.resume);
    }

    #[test]
    fn extension_flags_parse_in_order() {
        let args = parse(&["-e", "one.lua", "--extension", "two.lua", "--no-extensions"]);
        assert_eq!(args.extensions, ["one.lua", "two.lua"]);
        assert!(args.no_extensions);
    }
    #[test]
    fn project_trust_overrides_parse() {
        assert_eq!(parse(&["--approve"]).project_trust_override, Some(true));
        assert_eq!(parse(&["-a"]).project_trust_override, Some(true));
        assert_eq!(parse(&["--no-approve"]).project_trust_override, Some(false));
        assert_eq!(parse(&["-na"]).project_trust_override, Some(false));
    }
    #[test]
    fn unknown_single_dash_flag_is_error() {
        let args = parse(&["-zz"]);
        assert_eq!(
            args.diagnostics,
            vec![Diagnostic {
                is_error: true,
                message: "Unknown option: -zz".to_owned()
            }]
        );
    }
    #[test]
    fn mode_flag_selects_valid_modes_and_ignores_invalid() {
        assert_eq!(parse(&["--mode", "rpc"]).mode.as_deref(), Some("rpc"));
        assert_eq!(parse(&["--mode", "json"]).mode.as_deref(), Some("json"));
        assert_eq!(parse(&["--mode", "text"]).mode.as_deref(), Some("text"));
        // Spec: invalid mode values are consumed without a diagnostic.
        let args = parse(&["--mode", "bogus"]);
        assert_eq!(args.mode, None);
        assert!(args.diagnostics.is_empty());
    }
    #[test]
    fn print_flag_consumes_following_message() {
        let args = parse(&["-p", "hello"]);
        assert!(args.print);
        assert_eq!(args.messages, ["hello"]);
        let args = parse(&["--print"]);
        assert!(args.print);
        assert!(args.messages.is_empty());
        // Flags and @files are not consumed by -p.
        let args = parse(&["-p", "--version"]);
        assert!(args.print);
        assert!(args.version);
        assert!(args.messages.is_empty());
        let args = parse(&["-p", "@file.md"]);
        assert!(args.print);
        assert!(args.messages.is_empty());
        assert_eq!(args.file_args, ["file.md"]);
        // --- is a "message-like" token per the spec.
        let args = parse(&["-p", "---"]);
        assert!(args.print);
        assert_eq!(args.messages, ["---"]);
    }
    #[test]
    fn export_and_system_prompt_parse() {
        let args = parse(&["--export", "session.jsonl", "out.html"]);
        assert_eq!(args.export.as_deref(), Some("session.jsonl"));
        assert_eq!(args.messages, ["out.html"]);
        let args = parse(&["--system-prompt", "be concise", "hello"]);
        assert_eq!(args.system_prompt.as_deref(), Some("be concise"));
        assert_eq!(args.messages, ["hello"]);
        let args = parse(&[
            "--append-system-prompt",
            "a",
            "--append-system-prompt",
            "b",
        ]);
        assert_eq!(args.append_system_prompt, ["a", "b"]);
        let args = parse(&["--session-dir", "/tmp/sess"]);
        assert_eq!(args.session_dir.as_deref(), Some("/tmp/sess"));
    }
    #[test]
    fn name_requires_value() {
        let args = parse(&["--name", "Refactor"]);
        assert_eq!(args.name.as_deref(), Some("Refactor"));
        assert!(args.diagnostics.is_empty());
        let args = parse(&["--name"]);
        assert_eq!(args.name, None);
        assert_eq!(args.diagnostics.len(), 1);
        assert!(args.diagnostics[0].is_error);
        assert_eq!(args.diagnostics[0].message, "--name requires a value");
        // -na (no-approve) is not -n.
        let args = parse(&["-na"]);
        assert_eq!(args.project_trust_override, Some(false));
        assert_eq!(args.name, None);
    }
    #[test]
    fn file_args_and_unknown_flags_collect() {
        let args = parse(&["@prompt.md", "@image.png", "hello"]);
        assert_eq!(args.file_args, ["prompt.md", "image.png"]);
        assert_eq!(args.messages, ["hello"]);
        let args = parse(&["--plan", "review"]);
        assert_eq!(args.unknown_flags, [("plan".to_owned(), serde_json::json!("review"))]);
        assert!(args.diagnostics.is_empty());
        let args = parse(&["--plan"]);
        assert_eq!(args.unknown_flags, [("plan".to_owned(), serde_json::json!(true))]);
        let args = parse(&["--plan=review"]);
        assert_eq!(args.unknown_flags, [("plan".to_owned(), serde_json::json!("review"))]);
        let args = parse(&["--plan", "--version"]);
        assert_eq!(args.unknown_flags, [("plan".to_owned(), serde_json::json!(true))]);
        assert!(args.version);
    }
}
