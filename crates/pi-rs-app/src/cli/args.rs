//! Port of `cli/args.ts` — CLI argument parsing and help.
//!
//! Landed subset (recorded): the bare-core entry points (`--login`,
//! `--list-models [search]`, `--provider/--model/--api-key/--thinking`,
//! `--help`, `--version`, positional messages) plus the session
//! selections `--continue`/`-c` and `--session <path|id>` (PLAN 6.2).
//! Parsing semantics for the landed flags match the spec exactly; the
//! remaining flags (`--resume`'s selector — PLAN 6.3 — `--fork`,
//! `--session-id`, `--session-dir`, `--no-session`, tools, resources,
//! modes, `@file` args) land with their rungs and full `--help`-surface
//! parity is item 10/11's acceptance. `--login` is a pi-rs surface
//! (divergence 3): pi's login lives in interactive `/login`, which the
//! bare core (doctrine 06) must reach without a frontend.

use pi_rs_ai_types::ModelThinkingLevel;

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
    pub thinking: Option<ModelThinkingLevel>,
    pub help: bool,
    pub version: bool,
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
    /// `--approve`/`-a` and `--no-approve`/`-na`: explicit project trust.
    pub project_trust_override: Option<bool>,
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
                // Spec: unknown `--flags` are collected for extensions;
                // the bare core has no extension flags to hand them to.
                result.diagnostics.push(Diagnostic {
                    is_error: true,
                    message: format!("Unknown option: {arg}"),
                });
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                result.diagnostics.push(Diagnostic {
                    is_error: true,
                    message: format!("Unknown option: {arg}"),
                });
            }
            _ => result.messages.push(arg.to_owned()),
        }
        i += 1;
    }

    result
}

/// Spec: `printHelp()` — the landed surface only; full parity is WS9.
pub fn help_text() -> String {
    format!(
        "{APP_NAME} - AI coding assistant with read, bash, edit, write tools

Usage:
  {APP_NAME} [options] [messages...]

Options:
  --provider <name>              Provider name
  --model <pattern>              Model pattern or ID (supports \"provider/id\" and optional \":<thinking>\")
  --api-key <key>                API key (defaults to env vars)
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh, max
  --list-models [search]         List available models (with optional fuzzy search)
  --login [provider]             Log into a provider via OAuth (default: anthropic)
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial UUID
  --approve, -a                    Trust project resources for this session
  --no-approve, -na                Do not trust project resources for this session
  --help, -h                     Show this help
  --version, -v                  Show version number

Environment Variables:
  ANTHROPIC_API_KEY                - Anthropic Claude API key
  ANTHROPIC_OAUTH_TOKEN            - Anthropic OAuth token (alternative to API key)
  {ENV_AGENT_DIR}            - Config directory (default: ~/{CONFIG_DIR_NAME}/agent)

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
}
