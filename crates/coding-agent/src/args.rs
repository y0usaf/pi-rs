//! `cli/args.ts`: the command line, parsed the way upstream parses it.

/// `--mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Json,
    Rpc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub error: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<Vec<String>>,
    pub thinking: Option<String>,
    pub continue_: bool,
    pub resume: bool,
    pub help: bool,
    pub version: bool,
    pub mode: Option<Mode>,
    pub name: Option<String>,
    pub no_session: bool,
    pub session: Option<String>,
    pub session_id: Option<String>,
    pub fork: Option<String>,
    pub session_dir: Option<String>,
    pub models: Option<Vec<String>>,
    pub tools: Option<Vec<String>>,
    pub exclude_tools: Option<Vec<String>>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub extensions: Option<Vec<String>>,
    pub no_extensions: bool,
    pub no_mcp: bool,
    pub print: bool,
    pub export: Option<String>,
    pub no_skills: bool,
    pub skills: Option<Vec<String>>,
    pub prompt_templates: Option<Vec<String>>,
    pub no_prompt_templates: bool,
    pub themes: Option<Vec<String>>,
    pub use_theme: Option<String>,
    pub no_themes: bool,
    pub no_context_files: bool,
    /// `--list-models` with its search pattern, or `Some(None)` without one.
    pub list_models: Option<Option<String>>,
    pub offline: bool,
    pub tui_mode: Option<String>,
    pub verbose: bool,
    pub project_trust_override: Option<bool>,
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    /// Unknown `--flags` (extension flags upstream): name and value (`None` for a bare flag).
    pub unknown_flags: Vec<(String, Option<String>)>,
    pub diagnostics: Vec<Diagnostic>,
}

pub const VALID_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

pub fn is_valid_thinking_level(level: &str) -> bool {
    VALID_THINKING_LEVELS.contains(&level)
}

fn is_tool_modifier(entry: &str) -> bool {
    entry.starts_with('+') || entry.starts_with('-')
}

/// `getToolListError`.
pub fn tool_list_error(entries: &[String]) -> Option<String> {
    let modifiers: Vec<&String> = entries.iter().filter(|e| is_tool_modifier(e)).collect();
    if modifiers.is_empty() {
        return None;
    }
    if modifiers.len() < entries.len() {
        return Some("tool names cannot be mixed with +name or -name entries".to_owned());
    }
    modifiers
        .iter()
        .find(|e| e.contains('*'))
        .map(|p| format!("+name and -name entries take exact tool names, not patterns: {p}"))
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        error: true,
        message: message.into(),
    }
}

/// `parseArgs`.
pub fn parse(args: &[String]) -> Args {
    let mut r = Args::default();
    let mut i = 0;
    // `args[i + 1]` when there is one.
    let next = |i: usize| args.get(i + 1).map(String::as_str);
    while i < args.len() {
        let arg = args[i].as_str();
        let has_next = i + 1 < args.len();
        let push = |list: &mut Option<Vec<String>>, value: &str| {
            list.get_or_insert_with(Vec::new).push(value.to_owned())
        };
        match arg {
            "--" => {
                for positional in &args[i + 1..] {
                    match positional.strip_prefix('@') {
                        Some(file) => r.file_args.push(file.to_owned()),
                        None => r.messages.push(positional.clone()),
                    }
                }
                break;
            }
            "--help" | "-h" => r.help = true,
            "--version" | "-v" => r.version = true,
            "--mode" => match next(i) {
                None => r
                    .diagnostics
                    .push(error("--mode requires text, json, or rpc")),
                Some(m) if m.starts_with('-') => r
                    .diagnostics
                    .push(error("--mode requires text, json, or rpc")),
                Some(m) => {
                    i += 1;
                    r.mode = match m {
                        "text" => Some(Mode::Text),
                        "json" => Some(Mode::Json),
                        "rpc" => Some(Mode::Rpc),
                        _ => {
                            r.diagnostics.push(error(format!(
                                "Invalid mode \"{m}\". Valid values: text, json, rpc"
                            )));
                            r.mode
                        }
                    };
                }
            },
            "--continue" | "-c" => r.continue_ = true,
            "--resume" | "-r" => r.resume = true,
            "--provider" if has_next => {
                i += 1;
                r.provider = Some(args[i].clone());
            }
            "--model" if has_next => {
                i += 1;
                r.model = Some(args[i].clone());
            }
            "--api-key" if has_next => {
                i += 1;
                r.api_key = Some(args[i].clone());
            }
            "--system-prompt" if has_next => {
                i += 1;
                r.system_prompt = Some(args[i].clone());
            }
            "--append-system-prompt" if has_next => {
                i += 1;
                push(&mut r.append_system_prompt, &args[i]);
            }
            "--name" | "-n" => {
                if has_next {
                    i += 1;
                    r.name = Some(args[i].clone());
                } else {
                    r.diagnostics.push(error("--name requires a value"));
                }
            }
            "--no-session" => r.no_session = true,
            "--session" if has_next => {
                i += 1;
                r.session = Some(args[i].clone());
            }
            "--session-id" if has_next => {
                i += 1;
                r.session_id = Some(args[i].clone());
            }
            "--fork" if has_next => {
                i += 1;
                r.fork = Some(args[i].clone());
            }
            "--session-dir" if has_next => {
                i += 1;
                r.session_dir = Some(args[i].clone());
            }
            "--models" if has_next => {
                i += 1;
                r.models = Some(split_list(&args[i]));
            }
            "--no-tools" | "-nt" => r.no_tools = true,
            "--no-builtin-tools" | "-nbt" => r.no_builtin_tools = true,
            "--tools" | "-t" if has_next => {
                i += 1;
                let tools = split_list(&args[i]);
                match tool_list_error(&tools) {
                    Some(e) => r.diagnostics.push(error(format!("{arg}: {e}"))),
                    None => r.tools = Some(tools),
                }
            }
            "--exclude-tools" | "-xt" if has_next => {
                i += 1;
                r.exclude_tools = Some(split_list(&args[i]));
            }
            "--thinking" if has_next => {
                i += 1;
                let level = &args[i];
                if is_valid_thinking_level(level) {
                    r.thinking = Some(level.clone());
                } else {
                    r.diagnostics.push(Diagnostic {
                        error: false,
                        message: format!(
                            "Invalid thinking level \"{level}\". Valid values: {}",
                            VALID_THINKING_LEVELS.join(", ")
                        ),
                    });
                }
            }
            "--print" | "-p" => {
                r.print = true;
                if let Some(n) = next(i)
                    && !n.starts_with('@')
                    && (!n.starts_with('-') || n.starts_with("---"))
                {
                    r.messages.push(n.to_owned());
                    i += 1;
                }
            }
            "--export" if has_next => {
                i += 1;
                r.export = Some(args[i].clone());
            }
            "--extension" | "-e" if has_next => {
                i += 1;
                push(&mut r.extensions, &args[i]);
            }
            "--no-extensions" | "-ne" => r.no_extensions = true,
            "--no-mcp" => r.no_mcp = true,
            "--skill" if has_next => {
                i += 1;
                push(&mut r.skills, &args[i]);
            }
            "--prompt-template" if has_next => {
                i += 1;
                push(&mut r.prompt_templates, &args[i]);
            }
            "--theme" if has_next => {
                i += 1;
                push(&mut r.themes, &args[i]);
            }
            "--use-theme" => match next(i) {
                Some(t) if !t.starts_with('-') => {
                    r.use_theme = Some(t.to_owned());
                    i += 1;
                }
                _ => r
                    .diagnostics
                    .push(error("--use-theme requires a theme name")),
            },
            "--no-skills" | "-ns" => r.no_skills = true,
            "--no-prompt-templates" | "-np" => r.no_prompt_templates = true,
            "--no-themes" => r.no_themes = true,
            "--no-context-files" | "-nc" => r.no_context_files = true,
            "--list-models" => match next(i) {
                Some(p) if !p.starts_with('-') && !p.starts_with('@') => {
                    r.list_models = Some(Some(p.to_owned()));
                    i += 1;
                }
                _ => r.list_models = Some(None),
            },
            "--tui-mode" => match next(i) {
                Some(m @ ("regular" | "fullscreen")) => {
                    r.tui_mode = Some(m.to_owned());
                    i += 1;
                }
                None => r
                    .diagnostics
                    .push(error("--tui-mode requires regular or fullscreen")),
                Some(m) if m.starts_with('-') => r
                    .diagnostics
                    .push(error("--tui-mode requires regular or fullscreen")),
                Some(m) => {
                    i += 1;
                    r.diagnostics.push(error(format!(
                        "Invalid TUI mode \"{m}\". Valid values: regular, fullscreen"
                    )));
                }
            },
            "--verbose" => r.verbose = true,
            "--approve" | "-a" => r.project_trust_override = Some(true),
            "--no-approve" | "-na" => r.project_trust_override = Some(false),
            "--offline" => r.offline = true,
            _ if arg.starts_with('@') => r.file_args.push(arg[1..].to_owned()),
            _ if arg.starts_with("--") => match arg.find('=') {
                Some(eq) => r
                    .unknown_flags
                    .push((arg[2..eq].to_owned(), Some(arg[eq + 1..].to_owned()))),
                None => match next(i) {
                    Some(n) if !n.starts_with('-') && !n.starts_with('@') => {
                        r.unknown_flags
                            .push((arg[2..].to_owned(), Some(n.to_owned())));
                        i += 1;
                    }
                    _ => r.unknown_flags.push((arg[2..].to_owned(), None)),
                },
            },
            _ if arg.starts_with('-') => {
                r.diagnostics.push(error(format!("Unknown option: {arg}")))
            }
            _ => r.messages.push(arg.to_owned()),
        }
        i += 1;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(items: &[&str]) -> Args {
        parse(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn print_takes_next_message() {
        let a = p(&["-p", "hello", "--model", "x/y", "@f.txt", "world"]);
        assert!(a.print);
        assert_eq!(a.messages, ["hello", "world"]);
        assert_eq!(a.model.as_deref(), Some("x/y"));
        assert_eq!(a.file_args, ["f.txt"]);
    }

    #[test]
    fn tools_and_errors() {
        let a = p(&[
            "--tools",
            "read, ls",
            "-x",
            "--thinking",
            "huge",
            "--mode",
            "nope",
        ]);
        assert_eq!(a.tools.unwrap(), ["read", "ls"]);
        assert_eq!(a.diagnostics.len(), 3);
        assert_eq!(a.diagnostics[0].message, "Unknown option: -x");
        let b = p(&["--tools", "read,+ls"]);
        assert_eq!(
            b.diagnostics[0].message,
            "--tools: tool names cannot be mixed with +name or -name entries"
        );
    }

    #[test]
    fn session_id_with_help_list_models_and_no_session() {
        let a = p(&["--session-id", "ephemeral-id", "--help"]);
        assert_eq!(a.session_id.as_deref(), Some("ephemeral-id"));
        assert!(a.help);
        let b = p(&["--session-id", "ephemeral-id", "--list-models"]);
        assert_eq!(b.session_id.as_deref(), Some("ephemeral-id"));
        assert_eq!(b.list_models, Some(None));
        let c = p(&["--session-id", "ephemeral-id", "--no-session"]);
        assert_eq!(c.session_id.as_deref(), Some("ephemeral-id"));
        assert!(c.no_session);
    }

    #[test]
    fn unknown_flags() {
        let a = p(&["--foo=1", "--bar", "v", "--baz", "--", "-q", "@a"]);
        assert_eq!(
            a.unknown_flags,
            [
                ("foo".into(), Some("1".into())),
                ("bar".into(), Some("v".into())),
                ("baz".into(), None)
            ]
        );
        assert_eq!(a.messages, ["-q"]);
        assert_eq!(a.file_args, ["a"]);
    }
}
