//! Port of `cli/args.ts` — CLI argument parsing and help.
//!
//! Parser semantics match Pi's `parseArgs` exactly for the landing surface,
//! pinned byte-for-byte by the differential in
//! `tests/args-parity` (a Pi-generated oracle from `parseArgs`/`printHelp`).
//! `--login [provider]` is a pi-rs surface (divergence 3): Pi's login lives in
//! interactive `/login`, which the bare core must reach without a frontend.
//! Unknown `--long` flags are collected (Pi hands them to extensions), but the
//! bare core has no extension flags to route them to, so `args` here drops them
//! after the diagnostic (parity with Pi's *parse*, which never errors on them).

use pi_rs_ai_types::ModelThinkingLevel;

use crate::config::{APP_NAME, CONFIG_DIR_NAME, ENV_AGENT_DIR, ENV_SESSION_DIR};
use crate::core::model_resolver::parse_thinking_level;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Mode {
    #[default]
    Text,
    Json,
    Rpc,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Text => write!(f, "text"),
            Mode::Json => write!(f, "json"),
            Mode::Rpc => write!(f, "rpc"),
        }
    }
}

/// Spec: the `diagnostics` entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub is_error: bool,
    pub message: String,
}

/// Spec: `Args` — the full landed surface (subset of Pi's `Args` that the
/// coding-agent product reaches on this base).
#[derive(Debug, Default)]
pub struct Args {
    pub mode: Mode,
    /// True when `--mode <mode>` was given (Pi's `Args.mode` is `Mode |
    /// undefined`; defaulting to Text means we must remember explicitness).
    pub mode_explicit: bool,
    pub print: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
    pub thinking: Option<ModelThinkingLevel>,
    pub help: bool,
    pub version: bool,
    /// `--list-models [search]`: `Some(None)` without a pattern.
    pub list_models: Option<Option<String>>,
    /// `--login [provider]` (pi-rs surface; default provider anthropic).
    pub login: Option<Option<String>>,
    /// `--continue` / `-c`: continue the most recent session.
    pub continue_recent: bool,
    /// `--resume` / `-r`: select a session to resume via the selector.
    pub resume: bool,
    /// `--session <path|id>`: use a specific session file or partial id.
    pub session: Option<String>,
    pub session_id: Option<String>,
    pub fork: Option<String>,
    pub session_dir: Option<String>,
    pub no_session: bool,
    pub name: Option<String>,
    pub models: Vec<String>,
    pub tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub export: Option<String>,
    pub skills: Vec<String>,
    pub no_skills: bool,
    pub prompt_templates: Vec<String>,
    pub no_prompt_templates: bool,
    pub themes: Vec<String>,
    pub no_themes: bool,
    pub no_context_files: bool,
    pub verbose: bool,
    pub offline: bool,
    /// Explicit `--extension` / `-e` Lua sources, in CLI order.
    pub extensions: Vec<String>,
    /// `--no-extensions` / `-ne`: disable discovery/configured sources only.
    pub no_extensions: bool,
    /// `--approve`/`-a` and `--no-approve`/`-na`: explicit project trust.
    pub project_trust_override: Option<bool>,
    pub messages: Vec<String>,
    /// `@file` arguments (spec `Args.fileArgs`), the `@` prefix stripped.
    pub file_args: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

fn optional_value(args: &[String], i: usize) -> Option<&String> {
    args.get(i + 1)
        .filter(|next| !next.starts_with('-') && !next.starts_with('@'))
}

/// Spec: `parseArgs(args)` for the landed surface.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args {
    let args: Vec<String> = args.into_iter().collect();
    let mut result = Args::default();

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].clone();
        if arg == "--help" || arg == "-h" {
            result.help = true;
        } else if arg == "--version" || arg == "-v" {
            result.version = true;
        } else if arg == "--mode" && i + 1 < args.len() {
            i += 1;
            let mode = args[i].as_str();
            // Pi ignores invalid/absent mode values silently.
            result.mode = match mode {
                "text" | "json" | "rpc" => {
                    result.mode_explicit = true;
                    match mode {
                        "text" => Mode::Text,
                        "json" => Mode::Json,
                        "rpc" => Mode::Rpc,
                        _ => unreachable!(),
                    }
                }
                _ => result.mode,
            };
        } else if arg == "--continue" || arg == "-c" {
            result.continue_recent = true;
        } else if arg == "--resume" || arg == "-r" {
            result.resume = true;
        } else if arg == "--provider" && i + 1 < args.len() {
            i += 1;
            result.provider = Some(args[i].clone());
        } else if arg == "--model" && i + 1 < args.len() {
            i += 1;
            result.model = Some(args[i].clone());
        } else if arg == "--api-key" && i + 1 < args.len() {
            i += 1;
            result.api_key = Some(args[i].clone());
        } else if arg == "--system-prompt" && i + 1 < args.len() {
            i += 1;
            result.system_prompt = Some(args[i].clone());
        } else if arg == "--append-system-prompt" && i + 1 < args.len() {
            i += 1;
            result.append_system_prompt.push(args[i].clone());
        } else if arg == "--name" || arg == "-n" {
            if i + 1 < args.len() {
                i += 1;
                result.name = Some(args[i].clone());
            } else {
                result.diagnostics.push(Diagnostic {
                    is_error: true,
                    message: "--name requires a value".into(),
                });
            }
        } else if arg == "--no-session" {
            result.no_session = true;
        } else if arg == "--session" && i + 1 < args.len() {
            i += 1;
            result.session = Some(args[i].clone());
        } else if arg == "--session-id" && i + 1 < args.len() {
            i += 1;
            result.session_id = Some(args[i].clone());
        } else if arg == "--fork" && i + 1 < args.len() {
            i += 1;
            result.fork = Some(args[i].clone());
        } else if arg == "--session-dir" && i + 1 < args.len() {
            i += 1;
            result.session_dir = Some(args[i].clone());
        } else if arg == "--models" && i + 1 < args.len() {
            i += 1;
            result.models = args[i].split(',').map(|s| s.trim().to_owned()).collect();
        } else if arg == "--no-tools" || arg == "-nt" {
            result.no_tools = true;
        } else if arg == "--no-builtin-tools" || arg == "-nbt" {
            result.no_builtin_tools = true;
        } else if (arg == "--tools" || arg == "-t") && i + 1 < args.len() {
            i += 1;
            result.tools = args[i]
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
        } else if (arg == "--exclude-tools" || arg == "-xt") && i + 1 < args.len() {
            i += 1;
            result.exclude_tools = args[i]
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
        } else if arg == "--thinking" && i + 1 < args.len() {
            i += 1;
            let level = args[i].as_str();
            match parse_thinking_level(level) {
                Some(parsed) => result.thinking = Some(parsed),
                None => result.diagnostics.push(Diagnostic {
                    is_error: false,
                    message: format!(
                        "Invalid thinking level \"{level}\". Valid values: off, minimal, low, medium, high, xhigh"
                    ),
                }),
            }
        } else if arg == "--print" || arg == "-p" {
            result.print = true;
            if let Some(next) = args.get(i + 1)
                && !next.starts_with('@')
                && (!next.starts_with('-') || next.starts_with("---"))
            {
                result.messages.push(next.clone());
                i += 1;
            }
        } else if arg == "--export" && i + 1 < args.len() {
            i += 1;
            result.export = Some(args[i].clone());
        } else if (arg == "--extension" || arg == "-e") && i + 1 < args.len() {
            i += 1;
            result.extensions.push(args[i].clone());
        } else if arg == "--no-extensions" || arg == "-ne" {
            result.no_extensions = true;
        } else if arg == "--skill" && i + 1 < args.len() {
            i += 1;
            result.skills.push(args[i].clone());
        } else if arg == "--prompt-template" && i + 1 < args.len() {
            i += 1;
            result.prompt_templates.push(args[i].clone());
        } else if arg == "--theme" && i + 1 < args.len() {
            i += 1;
            result.themes.push(args[i].clone());
        } else if arg == "--no-skills" || arg == "-ns" {
            result.no_skills = true;
        } else if arg == "--no-prompt-templates" || arg == "-np" {
            result.no_prompt_templates = true;
        } else if arg == "--no-themes" {
            result.no_themes = true;
        } else if arg == "--no-context-files" || arg == "-nc" {
            result.no_context_files = true;
        } else if arg == "--list-models" {
            match optional_value(&args, i) {
                Some(pattern) => {
                    result.list_models = Some(Some(pattern.clone()));
                    i += 1;
                }
                None => result.list_models = Some(None),
            }
        } else if arg == "--verbose" {
            result.verbose = true;
        } else if arg == "--approve" || arg == "-a" {
            result.project_trust_override = Some(true);
        } else if arg == "--no-approve" || arg == "-na" {
            result.project_trust_override = Some(false);
        } else if arg == "--offline" {
            result.offline = true;
        } else if arg == "--login" {
            match optional_value(&args, i) {
                Some(provider) => {
                    result.login = Some(Some(provider.clone()));
                    i += 1;
                }
                None => result.login = Some(None),
            }
        } else if let Some(stripped) = arg.strip_prefix('@') {
            // Spec `args.ts`: `@file` → fileArg (strip the @ prefix).
            result.file_args.push(stripped.to_owned());
        } else if let Some(stripped) = arg.strip_prefix("--") {
            // Unknown `--long` flags: Pi collects them for extensions (never an
            // error) and consumes a following non-flag, non-@file token as its
            // value. The bare core has no extension flags to route them to, so
            // they are dropped after this (Pi's parser parity retained).
            match stripped.split_once('=') {
                Some(_) => {}
                None => {
                    let next = args.get(i + 1);
                    if let Some(next) = next
                        && !next.starts_with('-')
                        && !next.starts_with('@')
                    {
                        i += 1;
                    }
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            result.diagnostics.push(Diagnostic {
                is_error: true,
                message: format!("Unknown option: {arg}"),
            });
        } else {
            result.messages.push(arg);
        }
        i += 1;
    }

    result
}

/// Spec: `printHelp()` — full landed help mirroring Pi's help text.
pub fn help_text() -> String {
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

Extensions can register additional flags (e.g., --plan from plan-mode extension).

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
  ANT_LING_API_KEY                 - Ant Ling API key
  OPENAI_API_KEY                   - OpenAI GPT API key
  AZURE_OPENAI_API_KEY             - Azure OpenAI API key
  AZURE_OPENAI_BASE_URL            - Azure OpenAI/Cognitive Services base URL (e.g. https://{{resource}}.openai.azure.com)
  AZURE_OPENAI_RESOURCE_NAME       - Azure OpenAI resource name (alternative to base URL)
  AZURE_OPENAI_API_VERSION         - Azure OpenAI API version (default: v1)
  AZURE_OPENAI_DEPLOYMENT_NAME_MAP - Azure OpenAI model=deployment map (comma-separated)
  DEEPSEEK_API_KEY                 - DeepSeek API key
  NVIDIA_API_KEY                   - NVIDIA NIM API key
  GEMINI_API_KEY                   - Google Gemini API key
  GROQ_API_KEY                     - Groq API key
  CEREBRAS_API_KEY                 - Cerebras API key
  XAI_API_KEY                      - xAI Grok API key
  FIREWORKS_API_KEY                - Fireworks API key
  TOGETHER_API_KEY                 - Together AI API key
  OPENROUTER_API_KEY               - OpenRouter API key
  AI_GATEWAY_API_KEY               - Vercel AI Gateway API key
  ZAI_API_KEY                      - ZAI API key
  ZAI_CODING_CN_API_KEY            - ZAI Coding Plan API key (China)
  MISTRAL_API_KEY                  - Mistral API key
  MINIMAX_API_KEY                  - MiniMax API key
  MOONSHOT_API_KEY                 - Moonshot AI API key
  OPENCODE_API_KEY                 - OpenCode Zen/OpenCode Go API key
  KIMI_API_KEY                     - Kimi For Coding API key
  CLOUDFLARE_API_KEY               - Cloudflare API token (Workers AI and AI Gateway)
  CLOUDFLARE_ACCOUNT_ID            - Cloudflare account id (required for both)
  CLOUDFLARE_GATEWAY_ID            - Cloudflare AI Gateway slug (required for AI Gateway)
  XIAOMI_API_KEY                   - Xiaomi MiMo API key (api.xiaomimimo.com billing)
  XIAOMI_TOKEN_PLAN_CN_API_KEY     - Xiaomi MiMo Token Plan API key (China region)
  XIAOMI_TOKEN_PLAN_AMS_API_KEY    - Xiaomi MiMo Token Plan API key (Amsterdam region)
  XIAOMI_TOKEN_PLAN_SGP_API_KEY    - Xiaomi MiMo Token Plan API key (Singapore region)
  AWS_PROFILE                      - AWS profile for Amazon Bedrock
  AWS_ACCESS_KEY_ID                - AWS access key for Amazon Bedrock
  AWS_SECRET_ACCESS_KEY            - AWS secret key for Amazon Bedrock
  AWS_BEARER_TOKEN_BEDROCK         - Bedrock API key (bearer token)
  AWS_REGION                       - AWS region for Amazon Bedrock (e.g., us-east-1)
  {ENV_AGENT_DIR:<32} - Config directory (default: ~/{CONFIG_DIR_NAME}/agent)
  {ENV_SESSION_DIR:<32} - Session storage directory (overridden by --session-dir)
  PI_PACKAGE_DIR                   - Override package directory (for Nix/Guix store paths)
  PI_OFFLINE                       - Disable startup network operations when set to 1/true/yes
  PI_TELEMETRY                     - Override install telemetry when set to 1/true/yes or 0/false/no
  PI_SHARE_VIEWER_URL              - Base URL for /share command (default: https://pi.dev/session/)

Built-in Tool Names:
  read   - Read file contents
  bash   - Execute bash commands
  edit   - Edit files with find/replace
  write  - Write files (creates/overwrites)
  grep   - Search file contents (read-only, off by default)
  find   - Find files by glob pattern (read-only, off by default)
  ls     - List directory contents (read-only, off by default)

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
    fn file_args_are_collected_without_at_prefix() {
        let args = parse(&["@note.txt", "@dir/other.md", "hi", "--print"]);
        assert_eq!(args.file_args, ["note.txt", "dir/other.md"]);
        // Messages are still collected independently.
        assert_eq!(args.messages, vec!["hi".to_owned()]);
        assert!(args.print);
        // An empty `@` is a fileArg of "" (Pi strips the prefix).
        let args = parse(&["@"]);
        assert_eq!(args.file_args, vec!["".to_owned()]);
    }
}
