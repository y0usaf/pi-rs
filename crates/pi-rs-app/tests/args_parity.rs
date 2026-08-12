#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Differential for Plan 10 CLI args + help. The oracle in
// tests/args-parity/oracle.json is generated from Pi's real `parseArgs`
// (cli/args.ts) and `printHelp` by scripts/args-oracle. This test replays the
// same argv corpus through pi-rs's Rust `parse_args` and compares the parsed
// result against Pi's, and compares the landing help text byte-for-byte
// against Pi's `printHelp` (with `APP_NAME`/CONFIG-dir placeholders resolved).
use pi_rs_app::cli::args::{Mode, help_text, parse_args};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/args-parity/oracle.json")).unwrap()
}

/// Map a Rust Args into the same shape the generator emits for Pi, so a JSON
/// equality comparison is meaningful. Fields Pi leaves unset are omitted.
fn normalize(args: &pi_rs_app::cli::args::Args) -> Value {
    let mut out = serde_json::Map::new();
    out.insert(
        "messages".into(),
        Value::Array(
            args.messages
                .iter()
                .map(|m| Value::String(m.clone()))
                .collect(),
        ),
    );
    out.insert(
        "fileArgs".into(),
        Value::Array(
            args.file_args
                .iter()
                .map(|m| Value::String(m.clone()))
                .collect(),
        ),
    );
    macro_rules! set_if {
        ($k:literal, $e:expr) => {
            if let Some(v) = $e {
                out.insert($k.into(), v);
            }
        };
    }
    if args.mode_explicit {
        out.insert(
            "mode".into(),
            Value::String(
                match args.mode {
                    Mode::Text => "text",
                    Mode::Json => "json",
                    Mode::Rpc => "rpc",
                }
                .into(),
            ),
        );
    }
    if args.print {
        out.insert("print".into(), Value::Bool(true));
    }
    set_if!(
        "provider",
        args.provider.as_ref().map(|s| Value::String(s.clone()))
    );
    set_if!(
        "model",
        args.model.as_ref().map(|s| Value::String(s.clone()))
    );
    set_if!(
        "apiKey",
        args.api_key.as_ref().map(|s| Value::String(s.clone()))
    );
    set_if!(
        "systemPrompt",
        args.system_prompt
            .as_ref()
            .map(|s| Value::String(s.clone()))
    );
    if !args.append_system_prompt.is_empty() {
        out.insert(
            "appendSystemPrompt".into(),
            Value::Array(
                args.append_system_prompt
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    set_if!("thinking", args.thinking.as_ref().map(|t| { json!(t) }));
    if args.help {
        out.insert("help".into(), Value::Bool(true));
    }
    if args.version {
        out.insert("version".into(), Value::Bool(true));
    }
    set_if!(
        "listModels",
        args.list_models.as_ref().map(|lm| match lm {
            Some(p) => Value::String(p.clone()),
            None => Value::Bool(true),
        })
    );
    if args.continue_recent {
        out.insert("continue".into(), Value::Bool(true));
    }
    if args.resume {
        out.insert("resume".into(), Value::Bool(true));
    }
    set_if!(
        "session",
        args.session.as_ref().map(|s| Value::String(s.clone()))
    );
    set_if!(
        "sessionId",
        args.session_id.as_ref().map(|s| Value::String(s.clone()))
    );
    set_if!("fork", args.fork.as_ref().map(|s| Value::String(s.clone())));
    set_if!(
        "sessionDir",
        args.session_dir.as_ref().map(|s| Value::String(s.clone()))
    );
    if args.no_session {
        out.insert("noSession".into(), Value::Bool(true));
    }
    set_if!("name", args.name.as_ref().map(|s| Value::String(s.clone())));
    if !args.models.is_empty() {
        out.insert(
            "models".into(),
            Value::Array(
                args.models
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if !args.tools.is_empty() {
        out.insert(
            "tools".into(),
            Value::Array(
                args.tools
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if !args.exclude_tools.is_empty() {
        out.insert(
            "excludeTools".into(),
            Value::Array(
                args.exclude_tools
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if args.no_tools {
        out.insert("noTools".into(), Value::Bool(true));
    }
    if args.no_builtin_tools {
        out.insert("noBuiltinTools".into(), Value::Bool(true));
    }
    set_if!(
        "export",
        args.export.as_ref().map(|s| Value::String(s.clone()))
    );
    if !args.skills.is_empty() {
        out.insert(
            "skills".into(),
            Value::Array(
                args.skills
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if args.no_skills {
        out.insert("noSkills".into(), Value::Bool(true));
    }
    if !args.prompt_templates.is_empty() {
        out.insert(
            "promptTemplates".into(),
            Value::Array(
                args.prompt_templates
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if args.no_prompt_templates {
        out.insert("noPromptTemplates".into(), Value::Bool(true));
    }
    if !args.themes.is_empty() {
        out.insert(
            "themes".into(),
            Value::Array(
                args.themes
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if args.no_themes {
        out.insert("noThemes".into(), Value::Bool(true));
    }
    if args.no_context_files {
        out.insert("noContextFiles".into(), Value::Bool(true));
    }
    if args.verbose {
        out.insert("verbose".into(), Value::Bool(true));
    }
    if args.offline {
        out.insert("offline".into(), Value::Bool(true));
    }
    if !args.extensions.is_empty() {
        out.insert(
            "extensions".into(),
            Value::Array(
                args.extensions
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if args.no_extensions {
        out.insert("noExtensions".into(), Value::Bool(true));
    }
    set_if!(
        "projectTrustOverride",
        args.project_trust_override.map(Value::Bool)
    );
    if !args.diagnostics.is_empty() {
        out.insert(
            "diagnostics".into(),
            Value::Array(
                args.diagnostics
                    .iter()
                    .map(|d| {
                        json!({
                            "type": if d.is_error { "error" } else { "warning" },
                            "message": d.message,
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(out)
}

#[test]
fn parse_args_matches_pi_oracle() {
    let oracle = fixture();
    for case in oracle["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let argv: Vec<String> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_owned())
            .collect();
        let parsed = parse_args(argv);
        let want = normalize(&parsed);
        // The generator omits `login` (pi-rs surface): ensure parse didn't set it.
        assert_eq!(parsed.login, None, "case {name}: login must be unset");
        assert_eq!(
            want, case["args"],
            "case {name}: parse_args result differs from Pi"
        );
    }
}

#[test]
fn help_text_matches_pi_printhelp() {
    let oracle = fixture();
    // Only one help text oracle entry lacks extension flags (the landing path).
    let help = oracle["help"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["name"] == "no-extension-flags")
        .expect("no-extension-flags help entry");
    // APP_NAME is `pi` in both; the oracle was generated from the same pinned
    // source so the placeholders are already resolved identically.
    assert_eq!(
        help_text(),
        help["text"].as_str().unwrap(),
        "help text differs from Pi's printHelp"
    );
}
