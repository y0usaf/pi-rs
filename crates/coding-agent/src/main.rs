//! `pi-rs`: pi's coding agent. On a terminal it opens the interactive mode; `-p` (or piped
//! input) runs print mode; `--mode json` is not supported yet. Sessions are pi-durable sqlite files.

use std::io::{IsTerminal, Read};
use std::process::ExitCode;
use std::sync::Arc;

use pi_coding_agent::args::{self, Args, Mode};
use pi_coding_agent::config::{self, VERSION};
use pi_coding_agent::extensions;
use pi_coding_agent::interactive;
use pi_coding_agent::list_models;
use pi_coding_agent::mcp;
use pi_coding_agent::models::ModelRegistry;
use pi_coding_agent::modes;
use pi_coding_agent::paths::{PathInput, join, normalize_path, resolve_read_path};
use pi_coding_agent::print::{self, RunInput, ToolSettings};
use pi_coding_agent::prompt_templates;
use pi_coding_agent::run_config::{self, Diagnostic, level_name};
use pi_coding_agent::session_fork;
use pi_coding_agent::session_list;
use pi_coding_agent::session_open;
use pi_coding_agent::session_tree;
use pi_coding_agent::settings::{
    self, SettingsManager, collect_diagnostics, deduplicate_diagnostics,
};
use pi_coding_agent::stdout;
use pi_coding_agent::system_prompt::{self, AssembleOptions};
use pi_coding_agent::tools::mime;
use pi_durable::ai::types::{ImageContent, ImageType, UserBlock};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::types::CompactionOverrides;
use pi_durable::storage::memory::MemoryStorage;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::Storage;

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(1)
}

/// `readPipedStdin`: all of stdin when it is not a terminal, trimmed; None when empty.
fn piped_stdin() -> Option<String> {
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut data = String::new();
    std::io::stdin().read_to_string(&mut data).ok()?;
    let trimmed = data.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `processFileArguments`: text files inline in `<file>` tags, images attached.
fn file_arguments(files: &[String], cwd: &str) -> Result<(String, Vec<UserBlock>), String> {
    let mut text = String::new();
    let mut images = Vec::new();
    for file in files {
        let path = resolve_read_path(file, cwd);
        let meta =
            std::fs::metadata(&path).map_err(|_| format!("Error: File not found: {path}"))?;
        if meta.len() == 0 {
            continue;
        }
        let bytes =
            std::fs::read(&path).map_err(|e| format!("Error: Could not read file {path}: {e}"))?;
        match mime::detect_supported_image_mime_type(&bytes) {
            Some(mime_type) => {
                images.push(UserBlock::Image(ImageContent {
                    kind: ImageType,
                    data: mime::base64(&bytes),
                    mime_type: mime_type.to_string(),
                }));
                text.push_str(&format!("<file name=\"{path}\"></file>\n"));
            }
            None => {
                let content = String::from_utf8_lossy(&bytes);
                let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
                text.push_str(&format!("<file name=\"{path}\">\n{content}\n</file>\n"));
            }
        }
    }
    Ok((text, images))
}

/// The flags pi-rs parses but has no behaviour for yet, in the order upstream reads them.
fn unimplemented_flag(parsed: &Args) -> Option<&'static str> {
    let flags = [
        (parsed.export.is_some(), "--export"),
        (parsed.tui_mode.is_some(), "--tui-mode"),
        (parsed.verbose, "--verbose"),
    ];
    flags.iter().find(|(set, _)| *set).map(|(_, name)| *name)
}

/// `forkFrom`: the copy records the session file it came from (`parentSession`).
fn record_parent(
    path: &str,
    source: &str,
    registry: &Arc<ModelRegistry>,
    cwd: &str,
    settings: &SettingsManager,
) -> Result<(), String> {
    let cancel = CancelToken::new();
    let storage: Arc<dyn Storage> = Arc::new(
        SqliteStorage::open(path, &Default::default()).map_err(|e| format!("Error: {e}"))?,
    );
    let harness = print::open_harness(
        storage,
        print::providers(Arc::clone(registry), &placeholder()),
        print::extension(&[], Vec::new(), tool_settings(cwd, settings, &[])),
        CompactionOverrides::default(),
    )
    .map_err(|e| e.to_string())?;
    let root = print::root(&harness, &cancel).map_err(|e| e.to_string())?;
    pi_coding_agent::session_info::set_parent_session(&root, &cancel, source)
        .map_err(|e| e.to_string())?;
    print::close(&harness, &cancel).map_err(|e| e.to_string())
}

/// `assertValidSessionId`.
fn valid_session_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let alnum = |b: &u8| b.is_ascii_alphanumeric();
    match (bytes.first(), bytes.last()) {
        (Some(first), Some(last)) if alnum(first) && alnum(last) => bytes
            .iter()
            .all(|b| alnum(b) || matches!(b, b'.' | b'_' | b'-')),
        _ => false,
    }
}

/// `validateSessionIdFlags`.
fn validate_session_id(parsed: &Args) -> Result<(), String> {
    let Some(id) = &parsed.session_id else {
        return Ok(());
    };
    let conflicts: Vec<&str> = [
        parsed.session.as_ref().map(|_| "--session"),
        parsed.continue_.then_some("--continue"),
        parsed.resume.then_some("--resume"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !conflicts.is_empty() {
        return Err(format!(
            "Error: --session-id cannot be combined with {}",
            conflicts.join(", ")
        ));
    }
    if !valid_session_id(id) {
        return Err(
            "Error: Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character"
                .into(),
        );
    }
    Ok(())
}

/// `validateForkFlags`.
fn validate_fork(parsed: &Args) -> Result<(), String> {
    if parsed.fork.is_none() {
        return Ok(());
    }
    let conflicts: Vec<&str> = [
        parsed.session.as_ref().map(|_| "--session"),
        parsed.continue_.then_some("--continue"),
        parsed.resume.then_some("--resume"),
        parsed.no_session.then_some("--no-session"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !conflicts.is_empty() {
        return Err(format!(
            "Error: --fork cannot be combined with {}",
            conflicts.join(", ")
        ));
    }
    Ok(())
}

fn report(diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        eprintln!("{}", d.render());
    }
}

fn run() -> Result<ExitCode, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = args::parse(&argv);
    let mut had_error = false;
    for d in &parsed.diagnostics {
        eprintln!(
            "{}: {}",
            if d.error { "Error" } else { "Warning" },
            d.message
        );
        had_error |= d.error;
    }
    if !parsed.unknown_flags.is_empty() {
        let names: Vec<String> = parsed
            .unknown_flags
            .iter()
            .map(|(name, _)| format!("--{name}"))
            .collect();
        eprintln!(
            "Error: Unknown option{}: {}",
            if names.len() == 1 { "" } else { "s" },
            names.join(", ")
        );
        had_error = true;
    }
    if had_error {
        return Ok(ExitCode::from(1));
    }
    if parsed.version {
        println!("{VERSION}");
        return Ok(ExitCode::SUCCESS);
    }
    if parsed.help {
        println!(
            "pi-rs {VERSION}: pi's coding agent.\n\nUsage: pi-rs [options] [@files...] [messages...]\n\nSee upstream `pi --help` for the options; pi-rs runs the interactive mode on a terminal, print mode with -p."
        );
        return Ok(ExitCode::SUCCESS);
    }
    validate_session_id(&parsed)?;
    if parsed
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err("Error: --name requires a non-empty value".into());
    }
    validate_fork(&parsed)?;
    if let Some(flag) = unimplemented_flag(&parsed) {
        return Err(format!("pi-rs: {flag} is not implemented yet"));
    }
    if let Some(search) = &parsed.list_models {
        let registry = ModelRegistry::load(&config::agent_dir());
        list_models::list(&registry, search.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    let mode = parsed.mode;
    if matches!(mode, Some(Mode::Rpc)) && !parsed.file_args.is_empty() {
        return Err("Error: @file arguments are not supported in RPC mode".into());
    }

    let cwd = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    let agent_dir = config::agent_dir();
    let mut startup = SettingsManager::create(&cwd, &agent_dir, false);
    let default_trust = startup.default_project_trust().to_owned();
    let trusted = settings::resolve_project_trust(
        &cwd,
        &agent_dir,
        parsed.project_trust_override,
        &default_trust,
    )?;
    let mut settings = SettingsManager::create(&cwd, &agent_dir, trusted);
    let mut reported = collect_diagnostics(&mut startup);
    reported.extend(collect_diagnostics(&mut settings));
    for diagnostic in deduplicate_diagnostics(&reported) {
        eprintln!("Warning: {}", diagnostic.message);
    }

    let loaded_templates = prompt_templates::load(&prompt_templates::Input {
        cwd: &cwd,
        agent_dir: &agent_dir,
        project_trusted: trusted,
        no_defaults: parsed.no_prompt_templates,
        cli_paths: parsed.prompt_templates.as_deref().unwrap_or(&[]),
    });
    for warning in &loaded_templates.warnings {
        eprintln!("{warning}");
    }
    let templates = loaded_templates.templates;

    let tools = run_config::select_tools(&parsed, &settings)?;
    let registry = Arc::new(ModelRegistry::load(&agent_dir));
    let mcp = mcp::Servers::start(&agent_dir, &cwd, trusted, parsed.no_mcp);
    let mcp_tools = mcp.registrations();
    // Upstream reports MCP trouble through the UI, which print mode does not have.
    let mut mcp_notices: Vec<String> = mcp.problems();
    mcp_notices.extend(mcp.unreachable_warning());
    let extensions = extensions::load(
        &cwd,
        &agent_dir,
        parsed.extensions.as_deref().unwrap_or(&[]),
        parsed.no_extensions,
        &settings,
        extensions::mode_name(parsed.mode),
        parsed.mode.is_none(),
    );

    // The session: a durable sqlite file, or memory with --no-session.
    let initial_name = parsed
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let sessions = session_list::dir(&agent_dir, &cwd, &parsed, &settings);
    let mut fork_source: Option<(String, String)> = None;
    let session_path = if parsed.no_session {
        None
    } else if let Some(fork) = &parsed.fork {
        let source = session_fork::resolve(&sessions, fork)
            .ok_or_else(|| format!("No session found matching '{fork}'"))?;
        let copy = session_fork::copy(&source, &sessions, parsed.session_id.as_deref())?;
        fork_source = Some((copy.clone(), source));
        Some(copy)
    } else if let Some(path) = &parsed.session {
        Some(normalize_path(path, PathInput::default()))
    } else if parsed.resume {
        if !std::io::stdin().is_terminal() {
            return Err("Error: --resume needs a terminal".into());
        }
        match interactive::selectors::session::select_session(&sessions, &agent_dir)? {
            Some(path) => Some(path),
            None => {
                println!("No session selected");
                return Ok(ExitCode::SUCCESS);
            }
        }
    } else if parsed.continue_ {
        session_list::latest(&sessions).or_else(|| Some(session_list::new_path(&sessions)))
    } else if let Some(id) = &parsed.session_id {
        let path = join(&sessions, &format!("{id}.db"));
        if !std::path::Path::new(&path).exists() {
            eprintln!(
                "Warning: No project session found with id '{id}'; creating a new session with that id."
            );
        }
        Some(path)
    } else {
        Some(session_list::new_path(&sessions))
    };
    let assembled = system_prompt::assemble(&AssembleOptions {
        cwd: &cwd,
        agent_dir: &agent_dir,
        project_trusted: trusted,
        system_prompt: parsed.system_prompt.as_deref(),
        append_system_prompt: parsed.append_system_prompt.as_deref(),
        no_context_files: parsed.no_context_files,
        no_skills: parsed.no_skills,
        skill_paths: parsed.skills.as_deref().unwrap_or(&[]),
        tools: &tools,
    });
    for w in &assembled.warnings {
        eprintln!("{w}");
    }
    let mut sections = system_prompt::build_sections(&assembled.input);
    if let Some(section) = mcp.section() {
        sections.push((
            "mcp_servers".to_owned(),
            format!("<mcp_servers>\n{section}\n</mcp_servers>"),
        ));
    }

    // A first look at the stored conversation, for --continue's model and thinking level.
    let cancel = CancelToken::new();
    let (session_model, session_thinking) = match session_path
        .as_deref()
        .filter(|p| std::path::Path::new(p).exists())
    {
        None => (None, None),
        Some(path) => {
            let storage: Arc<dyn Storage> = Arc::new(
                SqliteStorage::open(path, &Default::default())
                    .map_err(|e| format!("Error: {e}"))?,
            );
            let probe = print::open_harness(
                storage,
                print::providers(Arc::clone(&registry), &placeholder()),
                print::extension(&[], Vec::new(), tool_settings(&cwd, &settings, &[])),
                CompactionOverrides::default(),
            )
            .map_err(|e| e.to_string())?;
            let root =
                session_tree::active_conversation(&probe, &cancel).map_err(|e| e.to_string())?;
            let agent = root.agent(&cancel).map_err(|e| e.to_string())?;
            let has_messages = !print::transcript(&root, &cancel)
                .map_err(|e| e.to_string())?
                .is_empty();
            let model = agent
                .model
                .filter(|_| has_messages)
                .and_then(|m| registry.get_model(&m.provider, &m.model_id).cloned());
            let thinking = has_messages.then_some(agent.thinking_level);
            print::close(&probe, &cancel).map_err(|e| e.to_string())?;
            (model, thinking)
        }
    };

    let choice = run_config::choose_model(
        &parsed,
        &settings,
        &registry,
        session_model,
        session_thinking,
    );
    report(&choice.diagnostics);
    if choice.diagnostics.iter().any(|d| d.error) {
        return Ok(ExitCode::from(1));
    }

    // RPC mode reads its commands from stdin, as upstream does.
    let stdin = if matches!(mode, Some(Mode::Rpc)) {
        None
    } else {
        piped_stdin()
    };
    // Upstream starts the TUI without -p or --mode when stdin and stdout are terminals.
    let interactive = !parsed.print
        && mode.is_none()
        && stdin.is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal();
    let (file_text, images) = file_arguments(&parsed.file_args, &cwd)?;
    let mut messages = parsed.messages.clone();
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = stdin {
        parts.push(s);
    }
    if !file_text.is_empty() {
        parts.push(file_text);
    }
    if !messages.is_empty() {
        parts.push(messages.remove(0));
    }
    let initial = (!parts.is_empty()).then(|| (parts.concat(), images));

    let Some(model) = choice.model else {
        return Err(list_models::no_models_available());
    };
    let env = vec![
        ("PI_PROVIDER".to_string(), model.provider.clone()),
        ("PI_MODEL".to_string(), model.id.clone()),
        (
            "PI_REASONING_LEVEL".to_string(),
            level_name(choice.thinking),
        ),
    ];
    let mut tool_env = env;
    if let Some(path) = &session_path {
        tool_env.push(("PI_SESSION_FILE".to_string(), path.clone()));
    }
    let compaction_settings =
        settings.compaction_settings(Some((model.provider.as_str(), model.id.as_str())))?;
    let compaction = CompactionOverrides {
        enabled: Some(compaction_settings.enabled),
        reserve_tokens: Some(compaction_settings.reserve_tokens),
        keep_recent_tokens: Some(compaction_settings.keep_recent_tokens),
        background_tokens: None,
    };
    let settings_for_tools = ToolSettings {
        model_takes_images: model.input.iter().any(|i| i == "image"),
        ..tool_settings(&cwd, &settings, &tool_env)
    };
    if let Some((path, source)) = &fork_source {
        record_parent(path, source, &registry, &cwd, &settings)?;
    }
    if interactive {
        let (sender, incoming) = std::sync::mpsc::channel();
        let provider_count = {
            let mut providers: Vec<&str> = registry
                .available()
                .iter()
                .map(|m| m.provider.as_str())
                .collect();
            providers.sort();
            providers.dedup();
            providers.len()
        };
        let opener = interactive::run::Opener {
            source: session_open::Source {
                registry: Arc::clone(&registry),
                model: model.clone(),
                tools: tools.clone(),
                extra_tools: mcp_tools.clone(),
                sections: sections.clone(),
                tool_settings: settings_for_tools.clone(),
                compaction: compaction.clone(),
            },
            sender: sender.clone(),
            reports: Arc::new(std::sync::Mutex::new(sender)),
        };
        let path = interactive::run::run(interactive::run::Session {
            opener,
            session_dir: sessions.clone(),
            persists: !parsed.no_session,
            model,
            thinking: choice.thinking,
            cwd: cwd.clone(),
            settings: std::rc::Rc::new(std::cell::RefCell::new(settings)),
            registry,
            agent_dir: agent_dir.clone(),
            theme_paths: parsed.themes.clone().unwrap_or_default(),
            use_theme: parsed.use_theme.clone(),
            no_themes: parsed.no_themes,
            initial,
            messages,
            provider_count,
            session_path,
            initial_name,
            mcp_problems: mcp_notices,
            extensions,
            templates,
            incoming,
        })?;
        if let Some(path) = path {
            println!("Session: {path}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    for notice in &extensions.notices {
        eprintln!("{notice}");
    }
    let storage: Arc<dyn Storage> = match &session_path {
        Some(path) => {
            std::fs::create_dir_all(pi_coding_agent::paths::dirname(path))
                .map_err(|e| e.to_string())?;
            Arc::new(
                SqliteStorage::open(path, &Default::default())
                    .map_err(|e| format!("Error: {e}"))?,
            )
        }
        None => Arc::new(MemoryStorage::new()),
    };
    let harness = print::open_harness(
        storage,
        print::providers(Arc::clone(&registry), &model),
        print::extension_with(
            &tools,
            sections.clone(),
            settings_for_tools.clone(),
            mcp_tools.clone(),
        ),
        compaction.clone(),
    )
    .map_err(|e| e.to_string())?;
    let root = session_tree::active_conversation(&harness, &cancel).map_err(|e| e.to_string())?;
    if let Some(name) = &initial_name {
        pi_coding_agent::session_info::set_name(&root, &cancel, name).map_err(|e| e.to_string())?;
    }
    let input = RunInput {
        model,
        thinking: choice.thinking,
        cwd: cwd.clone(),
        templates: templates.clone(),
        initial,
        messages,
    };
    if matches!(mode, Some(Mode::Json)) {
        let outcome = modes::json_mode::run(&harness, &root, &input, &cancel);
        print::close(&harness, &cancel).map_err(|e| e.to_string())?;
        outcome?;
        return Ok(ExitCode::SUCCESS);
    }
    if matches!(mode, Some(Mode::Rpc)) {
        let mut commands: Vec<serde_json::Value> = extensions
            .commands
            .iter()
            .map(|command| {
                serde_json::json!({
                    "name": command.name,
                    "description": command.description,
                    "source": "extension",
                    "sourceInfo": { "path": command.path },
                })
            })
            .chain(templates.iter().map(|template| {
                serde_json::json!({
                    "name": template.name,
                    "description": template.description,
                    "source": "prompt",
                    "sourceInfo": { "path": template.file_path },
                })
            }))
            .collect();
        commands.extend(assembled.input.skills.iter().map(|skill| {
            serde_json::json!({
                "name": format!("skill:{}", skill.name),
                "description": skill.description,
                "source": "skill",
                "sourceInfo": { "path": skill.file_path },
            })
        }));
        let options = modes::rpc::Options {
            model: input.model.clone(),
            thinking: input.thinking,
            registry: Arc::clone(&registry),
            session_path: session_path.clone(),
            session_dir: sessions.clone(),
            cwd: input.cwd.clone(),
            settings: std::rc::Rc::new(std::cell::RefCell::new(settings)),
            source: session_open::Source {
                registry: Arc::clone(&registry),
                model: input.model.clone(),
                tools: tools.clone(),
                extra_tools: mcp_tools.clone(),
                sections: sections.clone(),
                tool_settings: settings_for_tools.clone(),
                compaction: compaction.clone(),
            },
            commands,
            templates,
        };
        modes::rpc::run(harness, root, options, &cancel)?;
        return Ok(ExitCode::SUCCESS);
    }
    let outcome = print::run_turns(&harness, &root, &input, &cancel);
    let transcript = print::transcript(&root, &cancel).map_err(|e| e.to_string())?;
    print::close(&harness, &cancel).map_err(|e| e.to_string())?;
    if let Err(e) = outcome {
        return Err(e.to_string());
    }
    let (out, err, code) = print::text_result(&transcript);
    stdout::write(&out)?;
    eprint!("{err}");
    Ok(ExitCode::from(code as u8))
}

fn placeholder() -> pi_durable::ai::types::Model {
    pi_durable::ai::types::Model::new("pi-rs", "none", "none", 0, 0, false)
}

fn tool_settings(cwd: &str, settings: &SettingsManager, env: &[(String, String)]) -> ToolSettings {
    let bin = config::bin_dir();
    let path = std::env::var("PATH").unwrap_or_default();
    let mut env = env.to_vec();
    if !path.split(':').any(|p| p == bin) {
        let joined = if path.is_empty() {
            bin
        } else {
            format!("{bin}:{path}")
        };
        env.push(("PATH".to_string(), joined));
    }
    ToolSettings {
        cwd: cwd.to_string(),
        shell_path: settings.shell_path(),
        command_prefix: settings.shell_command_prefix().map(str::to_string),
        env,
        model_takes_images: true,
    }
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--codemode-vm") {
        return ExitCode::from(pi_codemode::child::run() as u8);
    }
    match run() {
        Ok(code) => code,
        Err(message) => fail(message),
    }
}
