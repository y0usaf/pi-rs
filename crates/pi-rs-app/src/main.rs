//! The `pi` binary — WS2.6 bare core (doctrine 06): the substrate with
//! zero packs still boots. Three entry points: `--login`,
//! `--list-models [search]`, and `pi "prompt"` streaming a raw
//! completion (no tool loop — the loop is the WS4 pack). Mode
//! selection, sessions, and the Lua frontends land with WS3+; this file
//! stays a thin dispatcher (spec: `main.ts`).

use std::io::IsTerminal;
use std::process::ExitCode;

use pi_rs_app::cli::args::{Args, help_text, parse_args};
use pi_rs_app::cli::extensions::load_product_extensions;
use pi_rs_app::cli::list_models::render_model_list;
use pi_rs_app::cli::login::run_login;
use pi_rs_app::cli::session_select::{SessionChoice, choose_session, session_header_cwd};
use pi_rs_app::config::VERSION;
use pi_rs_app::core::auth_guidance::format_no_models_available_message;
use pi_rs_app::core::auth_storage::AuthStorage;
use pi_rs_app::core::model_registry::{ModelRegistry, ResolvedRequestAuth};
use pi_rs_app::core::model_resolver::{find_initial_model, resolve_cli_model};
use pi_rs_app::core::settings_manager::{SettingsManager, SettingsManagerCreateOptions};
use pi_rs_host::trust::{ProjectTrustStore, has_project_trust_inputs, project_trust_options};

/// Lua chunk that enables the stdout output guard on the host's `pi` API
/// (spec: output-guard.ts `takeOverStdout`). After this runs, stray Lua stdlib
/// `print`/`io.write` from extension chunks write to stderr instead of stdout,
/// so non-interactive (print/RPC) protocol output stays byte-clean. The product
/// channel `pi.output` writes directly to stdout and is unaffected.
const ENABLE_STDOUT_GUARD: &str =
    "local pi = ...\npi.apply_stdout_guard(true)\n";


/// Minimal chalk analogue: color only when the stream is a terminal.
fn stderr_paint(code: &str, text: &str) -> String {
    if std::io::stderr().is_terminal() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn error_line(text: &str) {
    eprintln!("{}", stderr_paint("31", text));
}

fn warning_line(text: &str) {
    eprintln!("{}", stderr_paint("33", text));
}

/// Minimal chalk analogue for stdout (`console.log(chalk.…)`).
fn stdout_paint(code: &str, text: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

/// Spec: `promptConfirm(message)` — readline `"{message} [y/N] "`.
fn prompt_confirm(message: &str) -> bool {
    print!("{message} [y/N] ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim().to_lowercase();
    answer == "y" || answer == "yes"
}

/// Create a Lua host at `cwd` with the product packs loaded (shared by
/// the pre-runtime selectors and the interactive frontend).
fn load_host_with_trust(cwd: &str, project_trusted: bool) -> Result<pi_rs_host::Host, String> {
    let host = pi_rs_host::Host::new(pi_rs_host::HostConfig {
        cwd: Some(cwd.to_owned()),
        project_trusted,
        ..Default::default()
    })
    .map_err(|error| format!("Error starting Lua host: {error}"))?;
    let report = pi_rs_app::builtins::manifest::DEFAULT_MANIFEST
        .load(&host, &[])
        .map_err(|error| format!("Error selecting builtin packages: {error}"))?;
    if let Some(error) = report.errors.first() {
        return Err(format!("Error loading {}: {}", error.path, error.error));
    }
    Ok(host)
}

fn load_host(cwd: &str) -> Result<pi_rs_host::Host, String> {
    load_host_with_trust(cwd, true)
}

/// Plan 10 (modes/rpc/rpc-mode.ts): run the headless RPC protocol.
/// Commands are read from stdin and responses/events written to stdout as
/// JSONL by the `rpc` Lua role. This runs before model resolution and auth, so
/// no network occurs at startup; the role constructs its own session. Returns
/// Test-only differential seed for the RPC role (Plan 10 RPC streaming).
///
/// Gated by `debug_assertions` (compiled OUT of release binaries) AND a magic
/// one-time token prefix: `PI_RPC_SCRIPTED_SEED` must be exactly
/// `PARITY_SEED_PREFIX` concatenated with the JSON seed. Without the token the
/// function returns `None` and the seam is inert, so an arbitrary/accidental
/// env value can never activate it. Only the sibling `rpc_streaming_parity`
/// test (which spawns `pi`) sets this exact value, passing the seed JSON
/// through to `request.scriptedRpc` so the streaming + seeded-read commands
/// reproduce Pi's `runRpcMode` oracle.
#[cfg(debug_assertions)]
fn scripted_rpc_seed() -> Option<serde_json::Value> {
    const PARITY_SEED_PREFIX: &str = "parity-seed:";
    let value = std::env::var("PI_RPC_SCRIPTED_SEED").ok()?;
    // Only an exact magic-token match activates the seam: strip the prefix and
    // parse the remainder as JSON, else treat it as absent. This both gates
    // spoofing (the token is not a user-visible setting) and rejects any
    // malformed/unrelated env value, keeping the seam inert outside the
    // harness that sets the matching token.
    match value.strip_prefix(PARITY_SEED_PREFIX) {
        Some(seed) => serde_json::from_str(seed).ok(),
        None => None,
    }
}

/// Non-debug builds never read the seed: the seam is compiled out entirely.
#[cfg(not(debug_assertions))]
fn scripted_rpc_seed() -> Option<serde_json::Value> {
    None
}

/// RPC mode: dispatch to the faithful `rpc` role. Returns the process exit as
/// the role's exit code.
fn run_rpc_mode(
    cwd: &str,
    agent_dir: &str,
    session_dir: Option<&str>,
    project_trusted: bool,
    settings_manager: &SettingsManager,
    cli_extensions: &[String],
    no_extensions: bool,
) -> ExitCode {
    let host = match load_host_with_trust(cwd, project_trusted) {
        Ok(host) => host,
        Err(message) => {
            error_line(&message);
            return ExitCode::FAILURE;
        }
    };
    // Output guard (spec: rpc-mode.ts takeOverStdout): RPC stdout must carry
    // only JSONL protocol records. Enable before extension loading so stray
    // Lua stdlib `print`/`io.write` from extension chunks go to stderr.
    if let Err(message) = host.load("stdout-guard", ENABLE_STDOUT_GUARD) {
        error_line(&message.to_string());
        return ExitCode::FAILURE;
    }
    // Load product extensions (explicit CLI sources first, then trusted
    // project/global/configured), so file-backed extension commands/providers
    // are available inside the RPC session.
    let extension_report = load_product_extensions(
        &host,
        &settings_manager.get_extension_paths(),
        cli_extensions,
        cwd,
        agent_dir,
        project_trusted,
        no_extensions,
    );
    if !extension_report.errors.is_empty() {
        for diagnostic in extension_report.errors {
            error_line(&format!(
                "Error: Failed to load extension \"{}\": {}",
                diagnostic.path, diagnostic.error
            ));
        }
        return ExitCode::FAILURE;
    }
    let request = serde_json::json!({
        "cwd": cwd,
        "agentDir": agent_dir,
        "sessionDir": session_dir,
        "mode": "rpc",
        "projectTrusted": project_trusted,
        // Differential parity seam (Plan 10 RPC streaming): when the test
        // harness sets PI_RPC_SCRIPTED_SEED (with the internal magic token —
        // see scripted_rpc_seed), its JSON is passed through to the RPC role so
        // the streaming commands (prompt/bash/compact/fork/clone/
        // new_session/switch_session/get_fork_messages) reproduce Pi's
        // `runRpcMode` oracle (generated from a scripted session stub).
        // SECURITY: this env var is a test-only backdoor — it can spoof RPC
        // responses (bash results, export paths, fork text). It is compiled OUT
        // of release binaries (`debug_assertions`) AND requires the magic token
        // value before `scripted_rpc_seed` returns anything, and it is set only
        // by the sibling rpc_streaming_parity test that spawns `pi`. In normal
        // runs the var is unset (or lacks the token) and the seam is inert
        // (scriptedRpc = null), so the real (unseeded) code paths run unchanged.
        "scriptedRpc": scripted_rpc_seed(),
    });
    match host.call_role("rpc", &request.to_string()) {
        Ok(Some(result)) => {
            let exit_code = result
                .get("exitCode")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if exit_code == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Ok(None) => {
            error_line("Error: rpc role returned no result");
            ExitCode::FAILURE
        }
        Err(error) => {
            error_line(&format!("Error: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args = parse_args(std::env::args().skip(1));

    // Spec: report parse diagnostics; errors are fatal.
    let mut had_error = false;
    for diagnostic in &args.diagnostics {
        if diagnostic.is_error {
            had_error = true;
            error_line(&format!("Error: {}", diagnostic.message));
        } else {
            warning_line(&format!("Warning: {}", diagnostic.message));
        }
    }
    if had_error {
        return ExitCode::FAILURE;
    }

    if args.version {
        println!("{VERSION}");
        return ExitCode::SUCCESS;
    }

    if args.help {
        print!("{}", help_text());
        return ExitCode::SUCCESS;
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            error_line(&format!("Error: {error}"));
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(args))
}

async fn run(mut args: Args) -> ExitCode {
    let mut auth_storage = AuthStorage::create(None);
    for error in auth_storage.drain_errors() {
        warning_line(&format!("Warning: {error}"));
    }

    if let Some(provider) = &args.login {
        return match run_login(&mut auth_storage, provider.as_deref()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error_line(&format!("Error: {error}"));
                ExitCode::FAILURE
            }
        };
    }

    let model_registry = ModelRegistry::new(&auth_storage);

    if let Some(search) = &args.list_models {
        if let Some(error) = model_registry.get_error() {
            warning_line(&format!(
                "Warning: errors loading config.lua models:\n{error}"
            ));
        }
        println!(
            "{}",
            render_model_list(&model_registry, &auth_storage, search.as_deref())
        );
        return ExitCode::SUCCESS;
    }

    let interactive = args.messages.is_empty()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !args.print
        && args.mode == pi_rs_app::cli::args::Mode::Text;
    if args.messages.is_empty() && !interactive {
        // Preserve the existing headless/no-stdin behavior. Interactive mode
        // is selected only when both sides of the terminal are live.
        // Spec (main.ts): plain `--mode json` or `--mode rpc` without a
        // message should invoke the mode with no initial prompt (stdin-only
        // is read below; an empty prompt is handled by the mode itself).
        if args.mode != pi_rs_app::cli::args::Mode::Json
            && args.mode != pi_rs_app::cli::args::Mode::Rpc
        {
            print!("{}", help_text());
            return ExitCode::SUCCESS;
        }
    }

    // Spec (main.ts): RPC mode uses stdin for the JSON-RPC protocol, not
    // for piped input. @file arguments are rejected in RPC mode.
    if args.mode == pi_rs_app::cli::args::Mode::Rpc && !args.file_args.is_empty() {
        error_line("Error: @file arguments are not supported in RPC mode");
        return ExitCode::FAILURE;
    }

    // Spec (main.ts readPipedStdin): read piped stdin content (skip for RPC).
    let stdin_content = if args.mode != pi_rs_app::cli::args::Mode::Rpc {
        pi_rs_app::cli::file_processor::read_piped_stdin()
    } else {
        None
    };

    // Spec (`main.ts`): the startup settings manager is created with
    // the default trust (the trust-prompt wiring is WS7 glue).
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut settings_manager =
        SettingsManager::create(&cwd, None, SettingsManagerCreateOptions::default());
    for error in settings_manager.drain_errors() {
        warning_line(&format!(
            "Warning: error loading {} settings: {}",
            error.scope, error.error
        ));
    }

    // Spec (`main.ts`): the effective session dir is `--session-dir` (not
    // landed) ?? env ?? settings; `createSessionManager` resolves the
    // `--session`/`--continue` selection before any cwd-bound state.
    let agent_dir_path = pi_rs_app::config::get_agent_dir();
    let agent_dir = agent_dir_path.to_string_lossy().into_owned();
    let cwd_string = cwd.to_string_lossy().into_owned();
    let session_dir = std::env::var(pi_rs_app::config::ENV_SESSION_DIR)
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| {
            pi_rs_app::core::settings_manager::expand_tilde_path(&value)
                .to_string_lossy()
                .into_owned()
        })
        .or_else(|| {
            settings_manager
                .get_session_dir()
                .map(|dir| dir.to_string_lossy().into_owned())
        });
    // Spec (`createSessionManager`): `--session` wins, then `--resume`'s
    // selector, then `--continue`. The picker is the Lua-authored
    // SessionSelectorComponent in a standalone TUI (cli/session-picker.ts).
    let resume_picked = if args.resume && args.session.is_none() {
        let host = match load_host(&cwd_string) {
            Ok(host) => host,
            Err(message) => {
                error_line(&message);
                return ExitCode::FAILURE;
            }
        };
        let request = serde_json::json!({
            "cwd": cwd_string,
            "sessionDir": session_dir,
            "agentDir": agent_dir,
            "home": std::env::var("HOME").ok(),
            "theme": settings_manager.get_theme(),
        });
        let picked = match host.call_role("resume-picker", &request.to_string()) {
            Ok(Some(result)) => result
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            Ok(None) => None,
            Err(error) => {
                error_line(&format!("Error: {error}"));
                return ExitCode::FAILURE;
            }
        };
        let Some(path) = picked else {
            // Spec: `console.log(chalk.dim("No session selected"))`, exit 0
            // (the picker's onExit quits silently with the same status).
            println!("{}", stdout_paint("2", "No session selected"));
            return ExitCode::SUCCESS;
        };
        Some(path)
    } else {
        None
    };

    let session_file = if let Some(path) = resume_picked {
        Some(path)
    } else {
        match choose_session(
            args.continue_recent,
            args.session.as_deref(),
            &cwd_string,
            session_dir.as_deref(),
            &agent_dir,
        ) {
            SessionChoice::Open { path } => Some(path),
            SessionChoice::Create => None,
            SessionChoice::NotFound { arg } => {
                error_line(&format!("No session found matching '{arg}'"));
                return ExitCode::FAILURE;
            }
            SessionChoice::ConfirmFork {
                path,
                cwd: session_cwd,
            } => {
                // Spec: a `--session` match from another project forks into
                // the current directory after confirmation.
                println!(
                    "{}",
                    stdout_paint(
                        "33",
                        &format!("Session found in different project: {session_cwd}")
                    )
                );
                if !prompt_confirm("Fork this session into current directory?") {
                    println!("{}", stdout_paint("2", "Aborted."));
                    return ExitCode::SUCCESS;
                }
                let forked = match pi_rs_session::SessionManager::fork_from(
                    &path,
                    &cwd_string,
                    session_dir.as_deref(),
                    &agent_dir,
                    None,
                ) {
                    Ok(forked) => forked,
                    Err(error) => {
                        error_line(&format!("Error: {error}"));
                        return ExitCode::FAILURE;
                    }
                };
                forked.get_session_file().map(str::to_owned)
            }
        }
    };
    // Spec (`main.ts`): a selected session whose stored cwd no longer
    // exists prompts before the runtime cwd is chosen — interactive mode
    // offers Continue (use the process cwd as an override) / Cancel
    // (exit 0); headless mode fails with the MissingSessionCwdError text.
    let mut cwd_override: Option<String> = None;
    if let Some(file) = &session_file {
        let header_cwd = session_header_cwd(file).unwrap_or_default();
        if !header_cwd.is_empty() && !std::path::Path::new(&header_cwd).exists() {
            if !interactive {
                error_line(&format!(
                    "Stored session working directory does not exist: {header_cwd}\nSession file: {file}\nCurrent working directory: {cwd_string}"
                ));
                return ExitCode::FAILURE;
            }
            let host = match load_host(&cwd_string) {
                Ok(host) => host,
                Err(message) => {
                    error_line(&message);
                    return ExitCode::FAILURE;
                }
            };
            let request = serde_json::json!({
                "title": format!(
                    "cwd from session file does not exist\n{header_cwd}\n\ncontinue in current cwd\n{cwd_string}"
                ),
                "options": ["Continue", "Cancel"],
                "theme": settings_manager.get_theme(),
            });
            let selected = match host.call_role("startup-selector", &request.to_string()) {
                Ok(Some(result)) => result
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                Ok(None) => None,
                Err(error) => {
                    error_line(&format!("Error: {error}"));
                    return ExitCode::FAILURE;
                }
            };
            if selected.as_deref() != Some("Continue") {
                return ExitCode::SUCCESS;
            }
            cwd_override = Some(cwd_string.clone());
        }
    }
    // Spec: the session's header cwd is the effective runtime cwd —
    // `--session`/`--continue`/`--resume` may re-home the process to the
    // session's project (`sessionManager.getCwd()` feeds every cwd-bound
    // service); a missing-cwd override keeps the process cwd instead.
    let cwd = if cwd_override.is_some() {
        cwd
    } else {
        session_file
            .as_deref()
            .and_then(session_header_cwd)
            .map(std::path::PathBuf::from)
            .unwrap_or(cwd)
    };
    let cwd_string = cwd.to_string_lossy().into_owned();

    // project-trust.ts startup decision order. Prompt presentation is the
    // embedded Lua startup selector; Rust owns only trust-store persistence.
    let trust_store = ProjectTrustStore::new(&agent_dir);
    let has_trust_inputs = has_project_trust_inputs(&cwd_string);
    let mut project_trusted = args.project_trust_override.unwrap_or(!has_trust_inputs);
    if args.project_trust_override.is_none() && has_trust_inputs {
        project_trusted = match trust_store.get(&cwd_string) {
            Ok(Some(decision)) => decision,
            Ok(None) => match settings_manager.get_default_project_trust() {
                "always" => true,
                "never" => false,
                _ if interactive => {
                    let host = match load_host(&cwd_string) {
                        Ok(host) => host,
                        Err(message) => {
                            error_line(&message);
                            return ExitCode::FAILURE;
                        }
                    };
                    let options = project_trust_options(&cwd_string, true);
                    let request = serde_json::json!({
                        "title": pi_rs_host::trust::format_project_trust_prompt(&cwd_string),
                        "options": options.iter().map(|option| option.label.clone()).collect::<Vec<_>>(),
                        "theme": settings_manager.get_theme(),
                    });
                    let selected = match host.call_role("startup-selector", &request.to_string()) {
                        Ok(Some(result)) => result
                            .get("value")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        Ok(None) => None,
                        Err(error) => {
                            error_line(&format!("Error: {error}"));
                            return ExitCode::FAILURE;
                        }
                    };
                    let selected = selected
                        .and_then(|label| options.into_iter().find(|option| option.label == label));
                    if let Some(option) = selected {
                        if let Err(error) = trust_store.set_many(&option.updates) {
                            error_line(&format!("Error: {error}"));
                            return ExitCode::FAILURE;
                        }
                        option.trusted
                    } else {
                        false
                    }
                }
                _ => false,
            },
            Err(error) => {
                error_line(&format!("Error: {error}"));
                return ExitCode::FAILURE;
            }
        };
    }
    settings_manager = SettingsManager::create(
        &cwd,
        None,
        SettingsManagerCreateOptions {
            project_trusted: Some(project_trusted),
        },
    );

    // Plan 10 (modes/rpc/rpc-mode.ts): `--mode rpc` is a headless JSONL
    // protocol on stdin/stdout. It does not resolve a model or fetch auth at
    // startup (commands drive the session); dispatch to the RPC role before
    // model resolution so no network/OAuth occurs.
    if args.mode == pi_rs_app::cli::args::Mode::Rpc {
        return run_rpc_mode(
            &cwd_string,
            &agent_dir,
            session_dir.as_deref(),
            project_trusted,
            &settings_manager,
            &args.extensions,
            args.no_extensions,
        );    }

    // Spec (`buildSessionOptions` / `findInitialModel`): CLI model wins
    // (a `:<thinking>` suffix applies unless --thinking is explicit),
    // else the settings default, else the first available model
    // preferring provider defaults.
    let model = if args.model.is_some() {
        let resolved = resolve_cli_model(
            args.provider.as_deref(),
            args.model.as_deref(),
            &model_registry,
        );
        if let Some(warning) = &resolved.warning {
            warning_line(&format!("Warning: {warning}"));
        }
        if let Some(error) = &resolved.error {
            error_line(&format!("Error: {error}"));
            return ExitCode::FAILURE;
        }
        resolved.model
    } else {
        find_initial_model(
            &model_registry,
            &auth_storage,
            settings_manager.get_default_provider().as_deref(),
            settings_manager.get_default_model().as_deref(),
            settings_manager.get_default_thinking_level(),
        )
        .model
    };

    let Some(model) = model else {
        error_line(&format_no_models_available_message());
        return ExitCode::FAILURE;
    };

    // Spec: `--api-key` requires a model selected via --model.
    if let Some(api_key) = &args.api_key {
        if args.model.is_none() {
            error_line(
                "Error: --api-key requires a model to be specified via --model, --provider/--model, or --models",
            );
            return ExitCode::FAILURE;
        }
        auth_storage.set_runtime_api_key(&model.provider, api_key);
    }

    let ResolvedRequestAuth::Ok {
        api_key,
        headers: _,
    } = model_registry
        .get_api_key_and_headers(&mut auth_storage, &model)
        .await
    else {
        error_line(&format_no_models_available_message());
        return ExitCode::FAILURE;
    };

    // Plan 10 (main.ts prepareInitialMessage + cli/file-processor.ts +
    // cli/initial-message.ts): combine piped stdin, `@file` content (text
    // + image attachments), and the first CLI message into the single
    // initial prompt. The differential oracle in
    // tests/file-processor-parity/oracle.json pins the text path
    // byte-for-byte against Pi's real processFileArguments/buildInitialMessage.
    let mut file_images: Vec<pi_rs_app::cli::file_processor::ImageContent> = Vec::new();
    let mut file_text = String::new();
    if !args.file_args.is_empty() {
        let processed = pi_rs_app::cli::file_processor::process_file_arguments(
            &args.file_args,
            &cwd,
            settings_manager.get_image_auto_resize(),
        )
        .await;
        match processed {
            Ok(files) => {
                file_text = files.text;
                file_images = files.images;
            }
            Err(e) => {
                // Spec (file-processor.ts): missing/unreadable file prints an
                // error to stderr and exits 1.
                match e {
                    pi_rs_app::cli::file_processor::FileProcessorError::NotFound(p) => {
                        error_line(&format!("Error: File not found: {}", p.display()));
                        return ExitCode::FAILURE;
                    }
                    pi_rs_app::cli::file_processor::FileProcessorError::ReadFailed {
                        path,
                        message,
                    } => {
                        error_line(&format!(
                            "Error: Could not read file {}: {}",
                            path.display(),
                            message
                        ));
                        return ExitCode::FAILURE;
                    }
                    pi_rs_app::cli::file_processor::FileProcessorError::Other(message) => {
                        error_line(&format!("Error: {message}"));
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
    }
    let initial = pi_rs_app::cli::file_processor::build_initial_message(
        &mut args.messages,
        &file_text,
        file_images,
        stdin_content,
    );
    let prompt = initial.initial_message.unwrap_or_default();
    let initial_images = initial.initial_images.unwrap_or_default();
    // Spec (main.ts runPrintMode): `buildInitialMessage` shifted the first CLI
    // message into the initial prompt; any remaining positional messages are
    // sent as sequential follow-up prompts after the initial message.
    let follow_up_messages = args.messages.clone();
    // The CLI is a thin consumer of public Lua packs. Rust resolves startup
    // resources and supplies mechanism values; frontend/loop policy is Lua.
    let host = match load_host_with_trust(&cwd_string, project_trusted) {
        Ok(host) => host,
        Err(message) => {
            error_line(&message);
            return ExitCode::FAILURE;
        }
    };
    // Resource-loader extension precedence: explicit CLI sources first, then
    // trusted project, global, and configured sources. --no-extensions keeps
    // CLI sources. Every file loads in isolation through the same VM/API as
    // the embedded packs; report all diagnostics only after the full batch.
    // Non-interactive modes (print/text/json) keep stdout protocol-clean:
    // enable the output guard before extension loading (spec: output-guard.ts
    // takeOverStdout). Interactive mode leaves it off (TUI owns the terminal).
    if !interactive
        && let Err(message) = host.load("stdout-guard", ENABLE_STDOUT_GUARD)
    {
        error_line(&message.to_string());
        return ExitCode::FAILURE;
    }
    let extension_report = load_product_extensions(
        &host,
        &settings_manager.get_extension_paths(),
        &args.extensions,
        &cwd_string,
        &agent_dir,
        project_trusted,
        args.no_extensions,
    );
    if !extension_report.errors.is_empty() {
        for diagnostic in extension_report.errors {
            error_line(&format!(
                "Error: Failed to load extension \"{}\": {}",
                diagnostic.path, diagnostic.error
            ));
        }
        return ExitCode::FAILURE;
    }
    let request = serde_json::json!({

        "model": model, "apiKey": api_key, "prompt": prompt,
        // Plan 10 (main.ts runPrintMode messages[]): follow-up prompts sent
        // sequentially after the initial message, from remaining positional
        // CLI messages.
        "followUpMessages": follow_up_messages,
        // Plan 10: `@file` image attachments for the initial message (spec
        // print-mode.ts initialImages → session.prompt(initialMessage, {images})).
        "initialImages": initial_images,
        // The raw --api-key override: the interactive frontend mirrors it
        // into the VM's auth storage (spec: `setRuntimeApiKey`) so the
        // per-request `getApiKey` seam resolves it for the model's provider.
        "runtimeApiKey": args.api_key,
        // Session construction is pack policy (`pi.session.open`/`create`
        // per sdk.ts createAgentSession); the CLI resolves the
        // `--continue`/`--session` selection to a file (main.ts
        // createSessionManager) and supplies cwd/agentDir.
        "sessionFile": session_file,
        "sessionDir": session_dir,
        "cwdOverride": cwd_override,
        "cwd": cwd_string,
        "home": std::env::var("HOME").ok(),
        "appName": pi_rs_app::config::APP_NAME, "version": VERSION,
        // System-prompt inputs (spec: buildSystemPrompt reads config.ts
        // paths and the agent dir; policy composes them in Lua).
        "agentDir": agent_dir,
        "docsPath": pi_rs_app::config::get_docs_path().to_string_lossy(),
        "readmePath": pi_rs_app::config::get_readme_path().to_string_lossy(),
        "examplesPath": pi_rs_app::config::get_examples_path().to_string_lossy(),
        "thinkingLevel": args.thinking.map(|level| serde_json::to_value(level).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_else(|| "off".into())).unwrap_or_else(|| "off".into()),
        // sdk.ts restore inputs: CLI-sourced model/thinking win; a
        // restored session's saved values apply only when the flag was
        // not given (utils/agent-session.lua session_startup, which reads
        // the settings default from the VM's own `pi.settings` store).
        "modelFromCli": args.model.is_some(),
        "thinkingFromCli": args.thinking.is_some(),
        "mode": args.mode.to_string(),
        "projectTrusted": project_trusted,
    });
    let role = if interactive { "interactive" } else { "print" };
    match host.call_role(role, &request.to_string()) {
        Ok(Some(result)) => {
            if interactive {
                // handleFatalRuntimeError → process.exit(1).
                let exit_code = result
                    .get("exitCode")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                return if exit_code == 0 {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                };
            }
            // JSON mode: output is already JSONL lines from the handler.
            if args.mode == pi_rs_app::cli::args::Mode::Json {
                return ExitCode::SUCCESS;
            }
            // Plan 10 (modes/print-mode.ts): text mode output was already
            // written by the handler (final text blocks + `\n`); the run's
            // `exitCode`/`stopReason` carry the exit status, and an
            // error/aborted result exits 1 with the message on stderr.
            let exit_code = result
                .get("exitCode")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if exit_code != 0 {
                // Plan 10 (modes/print-mode.ts): on an `error`/`aborted` stop
                // reason Pi writes exactly `assistantMsg.errorMessage ||
                // \`Request ${stopReason}\`` to stderr and exits 1.
                let message = result
                    .get("errorMessage")
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty());
                let line = match message {
                    Some(message) => message.to_owned(),
                    None => format!(
                        "Request {}",
                        result
                            .get("stopReason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("error")
                    ),
                };
                error_line(&line);
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Ok(None) => {
            error_line("Error: agent returned no result");
            ExitCode::FAILURE
        }
        Err(error) => {
            error_line(&format!("Error: {error}"));
            ExitCode::FAILURE
        }
    }
}
