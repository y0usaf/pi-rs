//! Upstream `core/slash-commands.ts` and the submit handler's command dispatch from
//! `interactive-mode.ts`: the builtin list as data, the commands that need no selector, picker or
//! dialog, and a notice for the ones that do.

use pi_tui::autocomplete::{CommandEntry, SlashCommand};
use pi_tui::keybindings::get_keybindings;

use super::footer::Totals;

/// A builtin command as `BUILTIN_SLASH_COMMANDS` has it, plus whether the submit handler takes
/// arguments after the name (which is what decides between a command and a message starting with `/`).
pub struct Builtin {
    pub name: &'static str,
    pub description: &'static str,
    pub argument_hint: Option<&'static str>,
    pub takes_arguments: bool,
}

const fn builtin(
    name: &'static str,
    description: &'static str,
    argument_hint: Option<&'static str>,
    takes_arguments: bool,
) -> Builtin {
    Builtin {
        name,
        description,
        argument_hint,
        takes_arguments,
    }
}

pub const BUILTIN_SLASH_COMMANDS: &[Builtin] = &[
    builtin("settings", "Open settings menu", None, false),
    builtin(
        "model",
        "Select model (opens selector UI)",
        Some("<provider/model>"),
        true,
    ),
    builtin(
        "tree",
        "Navigate session tree (switch branches)",
        None,
        false,
    ),
    builtin("thinking", "Set thinking level", Some("<level>"), true),
    builtin(
        "scoped-models",
        "Enable/disable models for Ctrl+P cycling",
        None,
        false,
    ),
    builtin(
        "export",
        "Export session (HTML default, or specify path: .html/.jsonl)",
        None,
        true,
    ),
    builtin(
        "import",
        "Import and resume a session from a JSONL file",
        None,
        true,
    ),
    builtin(
        "share",
        "Share session via a Radius artifact or a GitHub gist",
        None,
        false,
    ),
    builtin(
        "bug",
        "Report a bug to the Pi developers",
        Some("<description>"),
        true,
    ),
    builtin("copy", "Copy last agent message to clipboard", None, false),
    builtin("name", "Set session display name", None, true),
    builtin("session", "Show session info and stats", None, false),
    builtin("changelog", "Show changelog entries", None, false),
    builtin("hotkeys", "Show all keyboard shortcuts", None, false),
    builtin(
        "fork",
        "Create a new fork from a previous user message",
        None,
        false,
    ),
    builtin(
        "clone",
        "Duplicate the current session at the current position",
        None,
        false,
    ),
    builtin(
        "trust",
        "Save project trust decision for future sessions",
        None,
        false,
    ),
    builtin(
        "login",
        "Configure provider authentication",
        Some("<provider>"),
        true,
    ),
    builtin("logout", "Remove provider authentication", None, false),
    builtin("new", "Start a new session", None, false),
    builtin(
        "compact",
        "Manually compact the session context",
        None,
        true,
    ),
    builtin("resume", "Resume a different session", None, false),
    builtin(
        "reload",
        "Reload keybindings, extensions, skills, prompts, themes, and context files",
        None,
        false,
    ),
    builtin("quit", "Quit pi", None, false),
];
/// What the submit handler does with a line that starts with `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    New,
    Session,
    /// `/name [text]`: sets the session's display name, or shows it.
    Name(Option<String>),
    /// `/resume`: opens the session picker.
    Resume,
    Hotkeys,
    Compact(Option<String>),
    /// `/model [search]`: opens the selector, or picks an exact match.
    Model(Option<String>),
    /// `/thinking [level]`: opens the selector, or sets the level.
    Thinking(Option<String>),
    ScopedModels,
    Settings,
    Trust,
    Unavailable(&'static str),
}

/// Upstream's dispatch: a builtin name alone, or with arguments when it takes them. Anything else,
/// an unknown name included, is a message for the model.
pub fn dispatch(text: &str) -> Option<Command> {
    let rest = text.strip_prefix('/')?;
    let (name, arguments) = match rest.split_once(' ') {
        Some((name, arguments)) => (name, Some(arguments)),
        None => (rest, None),
    };
    let builtin = BUILTIN_SLASH_COMMANDS.iter().find(|c| c.name == name)?;
    if arguments.is_some() && !builtin.takes_arguments {
        return None;
    }
    Some(match name {
        "quit" => Command::Quit,
        "new" => Command::New,
        "session" => Command::Session,
        "name" => Command::Name(
            arguments
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned),
        ),
        "hotkeys" => Command::Hotkeys,
        "compact" => Command::Compact(
            arguments
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned),
        ),
        "model" => Command::Model(
            arguments
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned),
        ),
        "thinking" => Command::Thinking(
            arguments
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned),
        ),
        "resume" => Command::Resume,
        "scoped-models" => Command::ScopedModels,
        "settings" => Command::Settings,
        "trust" => Command::Trust,
        _ => Command::Unavailable(builtin.name),
    })
}

/// The builtins as autocomplete entries.
pub fn autocomplete_commands() -> Vec<CommandEntry> {
    BUILTIN_SLASH_COMMANDS
        .iter()
        .map(|c| {
            CommandEntry::Command(SlashCommand {
                name: c.name.to_string(),
                description: Some(c.description.to_string()),
                argument_hint: c.argument_hint.map(str::to_owned),
                argument_completions: None,
            })
        })
        .collect()
}

/// The keys the loop itself acts on: upstream's `app.*` bindings.
pub const INTERRUPT: &str = "escape";
pub const CLEAR: &str = "ctrl+c";
pub const EXIT: &str = "ctrl+d";
pub const EXPAND_TOOLS: &str = "ctrl+o";

const NAVIGATION: &[(&[&str], &str)] = &[
    (
        &[
            "tui.editor.cursorUp",
            "tui.editor.cursorDown",
            "tui.editor.cursorLeft",
            "tui.editor.cursorRight",
        ],
        "Move cursor / browse history",
    ),
    (
        &["tui.editor.cursorWordLeft", "tui.editor.cursorWordRight"],
        "Move by word",
    ),
    (&["tui.editor.cursorLineStart"], "Start of line"),
    (&["tui.editor.cursorLineEnd"], "End of line"),
    (&["tui.editor.jumpForward"], "Jump forward to character"),
    (&["tui.editor.jumpBackward"], "Jump backward to character"),
    (
        &["tui.editor.pageUp", "tui.editor.pageDown"],
        "Scroll by page",
    ),
];

const EDITING: &[(&[&str], &str)] = &[
    (&["tui.input.submit"], "Send message"),
    (&["tui.input.newLine"], "New line"),
    (&["tui.editor.deleteWordBackward"], "Delete word backwards"),
    (&["tui.editor.deleteWordForward"], "Delete word forwards"),
    (&["tui.editor.deleteToLineStart"], "Delete to start of line"),
    (&["tui.editor.deleteToLineEnd"], "Delete to end of line"),
    (&["tui.editor.yank"], "Paste the most-recently-deleted text"),
    (
        &["tui.editor.yankPop"],
        "Cycle through the deleted text after pasting",
    ),
    (&["tui.editor.undo"], "Undo"),
];

const OTHER: &[(&[&str], &str)] = &[(&["tui.input.tab"], "Path completion / accept autocomplete")];

const APP: &[(&[&str], &str)] = &[
    (&[INTERRUPT], "Cancel autocomplete / abort streaming"),
    (&[CLEAR], "Clear editor (first) / exit (second)"),
    (&[EXIT], "Exit (when editor is empty)"),
    (&[EXPAND_TOOLS], "Toggle tool output expansion"),
];

fn capitalize(part: &str) -> String {
    match part.chars().next() {
        Some(first) => first.to_uppercase().collect::<String>() + &part[first.len_utf8()..],
        None => String::new(),
    }
}

fn capitalize_parts(key: &str) -> String {
    key.split('+').map(capitalize).collect::<Vec<_>>().join("+")
}

fn bound_key(id: &str) -> String {
    get_keybindings()
        .keys(id)
        .iter()
        .map(|key| capitalize_parts(key))
        .collect::<Vec<_>>()
        .join("/")
}

fn rows(table: &[(&[&str], &str)], label: fn(&str) -> String) -> String {
    table
        .iter()
        .map(|(keys, action)| {
            let shown = keys
                .iter()
                .map(|key| label(key))
                .collect::<Vec<_>>()
                .join("` / `");
            format!("| `{shown}` | {action} |\n")
        })
        .collect()
}

fn section(title: &str, body: &str) -> String {
    format!("**{title}**\n| Key | Action |\n|-----|--------|\n{body}\n")
}

/// `handleHotkeysCommand`, over the keybindings this mode actually reads.
pub fn hotkeys() -> String {
    let other =
        rows(OTHER, bound_key) + &rows(APP, capitalize_parts) + "| `/` | Slash commands |\n";
    let mut out = String::from("**Keyboard Shortcuts**\n\n");
    out.push_str(&section("Navigation", &rows(NAVIGATION, bound_key)));
    out.push_str(&section("Editing", &rows(EDITING, bound_key)));
    out.push_str(&section("Other", &other));
    out
}

/// Messages and tool calls in the session, as upstream's `getSessionStats` counts them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: usize,
    pub user: usize,
    pub assistant: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
}

/// What `/session` prints.
pub struct SessionInfo {
    pub name: Option<String>,
    pub file: Option<String>,
    pub id: u64,
    pub counts: Counts,
    pub totals: Totals,
    pub context_tokens: Option<u64>,
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `handleSessionCommand`, without upstream's cache-warming section, which pi-rs has no
/// equivalent of.
pub fn session(info: &SessionInfo) -> String {
    let counts = &info.counts;
    let totals = &info.totals;
    let mut out = String::from("**Session Info**\n\n");
    if let Some(name) = &info.name {
        out.push_str(&format!("Name: {name}\n"));
    }
    out.push_str(&format!(
        "File: {}\nID: {}\n\n",
        info.file.as_deref().unwrap_or("In-memory"),
        info.id
    ));
    out.push_str(&format!(
        "**Messages**\nTotal: {}\nUser: {}\nAssistant: {}\nTools: {} calls, {} results\n\n",
        counts.total, counts.user, counts.assistant, counts.tool_calls, counts.tool_results
    ));
    let input = totals.input + totals.cache_read + totals.cache_write;
    out.push_str(&format!("**Tokens**\nInput: {}\n", thousands(input)));
    if input > 0 && (totals.cache_read > 0 || totals.cache_write > 0) {
        let hit_rate = totals.cache_read as f64 * 100.0 / input as f64;
        out.push_str(&format!(
            "  Cached: {} ({hit_rate:.1}%)\n",
            thousands(totals.cache_read)
        ));
        out.push_str(&format!(
            "  Uncached: {}\n",
            thousands(totals.input + totals.cache_write)
        ));
    }
    out.push_str(&format!(
        "Output: {}\nTotal: {}\n",
        thousands(totals.output),
        thousands(input + totals.output)
    ));
    if let Some(tokens) = info.context_tokens {
        out.push_str(&format!(
            "\n**Context**\nLast request: {}\n",
            thousands(tokens)
        ));
    }
    if totals.cost > 0.0 {
        out.push_str(&format!("\n**Cost**\nTotal: ${:.3}\n", totals.cost));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_is_listed_once_with_a_description() {
        let mut names: Vec<&'static str> = BUILTIN_SLASH_COMMANDS.iter().map(|c| c.name).collect();
        let listed = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), listed);
        assert!(
            BUILTIN_SLASH_COMMANDS
                .iter()
                .all(|c| !c.description.is_empty())
        );
    }

    #[test]
    fn every_builtin_dispatches_to_something_other_than_a_message() {
        for builtin in BUILTIN_SLASH_COMMANDS {
            assert!(
                dispatch(&format!("/{}", builtin.name)).is_some(),
                "{} is not dispatched",
                builtin.name
            );
        }
    }

    #[test]
    fn dispatch_matches_upstreams_exact_and_prefixed_forms() {
        assert_eq!(dispatch("/quit"), Some(Command::Quit));
        assert_eq!(dispatch("/new"), Some(Command::New));
        assert_eq!(dispatch("/session"), Some(Command::Session));
        assert_eq!(dispatch("/name"), Some(Command::Name(None)));
        assert_eq!(
            dispatch("/name night shift"),
            Some(Command::Name(Some("night shift".into())))
        );
        assert_eq!(dispatch("/hotkeys"), Some(Command::Hotkeys));
        assert_eq!(dispatch("/compact"), Some(Command::Compact(None)));
        assert_eq!(
            dispatch("/compact keep the plan"),
            Some(Command::Compact(Some("keep the plan".into())))
        );
        assert_eq!(dispatch("/compact  "), Some(Command::Compact(None)));
        assert_eq!(
            dispatch("/model gpt-5"),
            Some(Command::Model(Some("gpt-5".into())))
        );
        assert_eq!(dispatch("/model"), Some(Command::Model(None)));
        assert_eq!(
            dispatch("/thinking high"),
            Some(Command::Thinking(Some("high".into())))
        );
        assert_eq!(dispatch("/settings"), Some(Command::Settings));
        assert_eq!(dispatch("/scoped-models"), Some(Command::ScopedModels));
        assert_eq!(dispatch("/trust"), Some(Command::Trust));
        assert_eq!(dispatch("/quit now"), None);
        assert_eq!(dispatch("/session extra"), None);
        assert_eq!(dispatch("/nope"), None);
        assert_eq!(dispatch("/"), None);
        assert_eq!(dispatch("plain text"), None);
    }

    #[test]
    fn autocomplete_lists_the_builtins_with_hints() {
        let entries = autocomplete_commands();
        assert_eq!(entries.len(), BUILTIN_SLASH_COMMANDS.len());
        let model = entries
            .iter()
            .find_map(|entry| match entry {
                CommandEntry::Command(command) if command.name == "model" => Some(command),
                _ => None,
            })
            .expect("model is a builtin");
        assert_eq!(model.argument_hint.as_deref(), Some("<provider/model>"));
        assert_eq!(
            model.description.as_deref(),
            Some("Select model (opens selector UI)")
        );
    }

    #[test]
    fn hotkeys_lists_the_modes_own_keys() {
        let table = hotkeys();
        assert!(table.contains("**Navigation**"));
        assert!(table.contains("| `Enter` | Send message |"));
        assert!(table.contains("| `Escape` | Cancel autocomplete / abort streaming |"));
        assert!(table.contains("| `/` | Slash commands |"));
    }

    #[test]
    fn session_prints_totals_with_grouped_digits() {
        let info = SessionInfo {
            name: Some("night shift".into()),
            file: None,
            id: 7,
            counts: Counts {
                total: 4,
                user: 2,
                assistant: 1,
                tool_calls: 3,
                tool_results: 3,
            },
            totals: Totals {
                input: 1500,
                output: 250,
                cache_read: 500,
                cache_write: 0,
                cost: 0.25,
            },
            context_tokens: Some(2250),
        };
        let text = session(&info);
        assert!(text.contains("Name: night shift"));
        assert!(text.contains("File: In-memory"));
        assert!(text.contains("ID: 7"));
        assert!(text.contains("Tools: 3 calls, 3 results"));
        assert!(text.contains("Input: 2,000"));
        assert!(text.contains("Cached: 500 (25.0%)"));
        assert!(text.contains("Uncached: 1,500"));
        assert!(text.contains("Total: 2,250"));
        assert!(text.contains("Total: $0.250"));
    }
}
