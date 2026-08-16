#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_app::builtins::manifest::{DEFAULT_MANIFEST, ManifestError};
use pi_rs_host::{Host, HostConfig};

fn host(cwd: &std::path::Path) -> Host {
    Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap()
}

fn replacement_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/assembly-replacement.lua"
    )
}

#[test]
fn manifest_is_fail_closed_and_each_package_is_independently_suppressible() {
    let root = tempfile::tempdir().unwrap();
    let package_ids = DEFAULT_MANIFEST
        .packages
        .iter()
        .map(|package| package.id)
        .collect::<Vec<_>>();
    assert_eq!(
        package_ids,
        vec![
            "agent-core",
            "agent-policy",
            "coding-tools",
            "print-application",
            "interactive-frontend"
        ]
    );
    assert!(
        DEFAULT_MANIFEST
            .packages
            .iter()
            .all(|package| package.enabled_by_default)
    );
    // The only core substrate pack is agent-core: it is always loaded and
    // cannot be suppressed (suppressing it would break the policy packs that
    // require its pi.agent.* modules).
    assert_eq!(
        DEFAULT_MANIFEST
            .packages
            .iter()
            .filter(|package| package.core)
            .map(|package| package.id)
            .collect::<Vec<_>>(),
        vec!["agent-core"]
    );
    assert_eq!(
        DEFAULT_MANIFEST
            .load(&host(root.path()), &["agent-core"])
            .unwrap_err(),
        ManifestError::UnknownPackage("core substrate 'agent-core' cannot be suppressed".to_owned())
    );

    for package in DEFAULT_MANIFEST.packages {
        if package.core {
            continue; // core substrate is not a suppressible policy unit
        }
        let host = host(root.path());
        let report = DEFAULT_MANIFEST.load(&host, &[package.id]).unwrap();
        assert!(
            report.errors.is_empty(),
            "{}: {:?}",
            package.id,
            report.errors
        );
        // Suppressing a policy pack keeps the core substrate loaded.
        assert_eq!(report.loaded.len(), DEFAULT_MANIFEST.packages.len() - 1);
        assert!(!report.loaded.contains(&package.pack.source_key()));
        assert!(host.roles().is_ok());
        assert!(host.tools().is_ok());
    }

    let host = host(root.path());
    assert_eq!(
        DEFAULT_MANIFEST.load(&host, &["missing"]).unwrap_err(),
        ManifestError::UnknownPackage("missing".to_owned())
    );
    assert_eq!(
        DEFAULT_MANIFEST
            .load(&host, &["coding-tools", "coding-tools"])
            .unwrap_err(),
        ManifestError::DuplicateSuppression("coding-tools".to_owned())
    );
}

#[test]
fn zero_pack_host_accepts_the_same_file_backed_public_declarations() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let report = DEFAULT_MANIFEST.load_zero(&host);
    assert!(report.loaded.is_empty());
    assert!(report.errors.is_empty());
    assert!(host.roles().unwrap().is_empty());
    assert!(host.tools().unwrap().is_empty());

    host.load_file(replacement_path()).unwrap();
    let role = host
        .call_role("print", r#"{"prompt":"bare"}"#)
        .unwrap()
        .unwrap();
    assert_eq!(role["text"], "file-role:bare");
    assert_eq!(role["cwd"], root.path().to_string_lossy().as_ref());
    let tool = host
        .call_tool("read", "call-1", &serde_json::json!({}))
        .unwrap();
    assert_eq!(tool["content"][0]["text"], "file-tool");
    assert_eq!(
        host.call_command("assembly-policy", "").unwrap().unwrap()["message"],
        "file-policy"
    );
}

#[test]
fn per_tool_suppression_ablates_a_builtin_tool_and_allows_file_backed_replacement() {
    let root = tempfile::tempdir().unwrap();

    // Suppress `bash` while the rest of the tools pack stays loaded.
    let host = host(root.path());
    let report = DEFAULT_MANIFEST
        .load_with_suppressed_tools(&host, &[], &["bash"])
        .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let names: Vec<String> = host
        .tools()
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(names.contains(&"read".to_owned()), "read stays: {names:?}");
    assert!(!names.contains(&"bash".to_owned()), "bash ablated: {names:?}");

    // A file-backed replacement claims the name first-wins.
    let replacement_dir = tempfile::tempdir().unwrap();
    let replacement = replacement_dir.path().join("bash.lua");
    std::fs::write(
        &replacement,
        "local pi = ...\npi.register_tool({\n  name = \"bash\", active_by_default = true,\n  description = \"file-backed bash\",\n  parameters = { type = \"object\", properties = {} },\n  execute = function() return { content = { { type = \"text\", text = \"file-role:bash\" } }, details = {} } end,\n})\n",
    )
    .unwrap();
    host.load_file(replacement.to_str().unwrap()).unwrap();
    let names_after: Vec<String> = host
        .tools()
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(names_after.contains(&"bash".to_owned()), "{names_after:?}");
    let result = host
        .call_tool("bash", "call-1", &serde_json::json!({}))
        .unwrap();
    assert_eq!(result["content"][0]["text"], "file-role:bash");
}

#[test]
fn file_backed_role_tool_and_command_policy_replace_manifest_units() {
    let root = tempfile::tempdir().unwrap();

    let role_host = host(root.path());
    let report = DEFAULT_MANIFEST.load(&role_host, &[]).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    role_host.load_file(replacement_path()).unwrap();
    let role = role_host
        .call_role("print", r#"{"prompt":"replacement"}"#)
        .unwrap()
        .unwrap();
    assert_eq!(role["text"], "file-role:replacement");
    assert_eq!(
        role["capabilityCwd"],
        root.path().to_string_lossy().as_ref()
    );

    let tool_host = host(root.path());
    let report = DEFAULT_MANIFEST
        .load(&tool_host, &["coding-tools"])
        .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    tool_host.load_file(replacement_path()).unwrap();
    let tool = tool_host
        .call_tool("read", "call-1", &serde_json::json!({}))
        .unwrap();
    assert_eq!(tool["content"][0]["text"], "file-tool");

    let marker = root.path().join("policy-marker.txt");
    let route = role_host
        .call_command(
            "interactive-submit-route",
            &serde_json::json!({
                "texts": [format!("/assembly-policy {}", marker.display())],
                "cwd": root.path(),
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(route["trace"][0]["action"], "extension_command");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "file-policy");
}

#[test]
fn file_backed_package_imports_the_shared_agent_core_modules() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let report = DEFAULT_MANIFEST.load(&host, &[]).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let example = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/agent-core-module-demo.lua"
    );
    host.load_file(example).expect("agent-core module demo loads");
    let result = host
        .call_command("agent-core-module-demo", "")
        .expect("agent-core module demo runs")
        .expect("agent-core module demo result");

    // messages.convert_to_llm maps bashExecution -> user text block.
    assert_eq!(result["toLlmRole"], "user");
    // branch-summary.estimate_tokens is shared with compaction.
    assert_eq!(result["estimated"], 1);
    // compaction.is_context_overflow detects the pinned prompt-too-long text.
    assert_eq!(result["overflow"], true);
    // system-prompt.build_system_prompt includes the Guidelines section.
    assert_eq!(result["promptHasGuidelines"], true);
    // session-runtime exposes persist/session_startup/construct closures.
    assert_eq!(result["session"]["construct"], true);
    assert_eq!(result["session"]["startup"], true);
    assert_eq!(result["session"]["persist"], true);
    // bash-executor.get_shell_config resolves a shell path.
    assert!(result["shell"].is_string());
}

#[test]
fn file_backed_package_imports_the_public_syntax_highlight_module() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let report = DEFAULT_MANIFEST.load(&host, &[]).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    // An ordinary file-backed package (loaded through the same `Host::load`
    // path as user code) resolves the exact-version public `pi.highlight`
    // module that the builtin tools and interactive packs define, and
    // re-uses its closures — no private chunk-local tier. PLAN 9.7
    // module.syntax-highlight / modules.chunk-local-helpers. Theme arguments
    // flow in as tables, so the closures only need `pi` to resolve; the
    // exercise asserts the same closures resolve through the public module.
    let source = r#"local pi = ...
local h = pi.module.require("pi.highlight", "1")
local listed = {}
for _, entry in ipairs(pi.module.list()) do
  listed[entry.name .. "@" .. entry.version] = true
end
pi.register_command("highlight-module-demo", {
  description = "file-backed pi.highlight consumer",
  handler = function()
    return {
      hasThemeHighlight = type(h.theme_highlight_code) == "function",
      hasMarkdownHighlight = type(h.markdown_highlight_code) == "function",
      listed = listed["pi.highlight@1"] or false,
      differentClosures = h.theme_highlight_code ~= h.markdown_highlight_code,
    }
  end,
})
"#;
    host.load("file/highlight-module-demo.lua", source).expect("file-backed highlight consumer loads");
    let result = host
        .call_command("highlight-module-demo", "")
        .expect("highlight-module-demo runs")
        .expect("highlight-module-demo result");
    assert_eq!(result["listed"], true, "pi.highlight@1 is a public module");
    assert_eq!(result["hasThemeHighlight"], true);
    assert_eq!(result["hasMarkdownHighlight"], true);
    assert_eq!(result["differentClosures"], true);
}

/// PLAN 9.10 module.extension-composition: an ordinary file-backed package
/// imports the same public exact-version `pi.extension.composition@1` module
/// the builtin coding-agent/interactive packs define — the active-tool,
/// tool-call-fold, and extension-command policy table shared via the VM-wide
/// module registry. There is no pack-private chunk-local composition tier a
/// file-backed application cannot reach.
#[test]
fn file_backed_package_imports_the_public_extension_composition_module() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let report = DEFAULT_MANIFEST.load(&host, &[]).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    host.load_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/extension-composition-demo.lua"
    ))
    .expect("extension-composition-demo loads");
    let result = host
        .call_command("extension-composition-demo", "")
        .expect("extension-composition-demo runs")
        .expect("extension-composition-demo result");

    // pi.extension.composition@1 is a real registered public module.
    assert_eq!(result["registered"], true);
    // The full composition surface resolves from the file-backed consumer.
    assert_eq!(result["hasActiveTools"], true);
    assert_eq!(result["hasEmitToolCall"], true);
    assert_eq!(result["hasEmitToolResult"], true);
    assert_eq!(result["hasEmitGeneric"], true);
    assert_eq!(result["hasExecuteCommand"], true);
    assert_eq!(result["hasTryExecute"], true);
    assert_eq!(result["hasBindPiActions"], true);
    // active_tools reflects the builtin/tool-pack registered tools through
    // the shared table (the product and file-backed apps reuse one registry).
    assert!(
        result["toolCount"].as_u64().unwrap() > 0,
        "active_tools returned none: {result}"
    );
    assert_eq!(result["hasBashTool"], true);
    // emit_generic over an open channel with no listeners is a no-op fold.
    assert_eq!(result["noEventHandlers"], true);
    // emit_message_end with no handlers returns nil (no replacement).
    assert_eq!(result["replacedNil"], true);
}

/// PLAN 9.10 command-routing per-command suppression + first-wins replacement:
/// an ordinary file-backed package suppresses ONLY the builtin `/model` route
/// (via the public `pi.commands` registry) and registers a replacement, which
/// the real forward-facing dispatch resolves. The rest of the frontend stays
/// active: `/settings` and `/export` still route to their builtin actions.
#[test]
fn command_routing_suppresses_and_replaces_one_builtin_command() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let report = DEFAULT_MANIFEST.load(&host, &[]).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    host.load_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/command-routing-demo.lua"
    ))
    .expect("command-routing-demo loads");

    // The demo suppressed the builtin `/model` and registered a replacement.
    let demo = host
        .call_command(
            "command-routing-demo",
            &serde_json::json!({"texts":["/model", "/settings", "/model openai"]}).to_string(),
        )
        .expect("demo runs")
        .expect("demo result");
    assert_eq!(demo["disabled"], serde_json::json!(true));
    assert_eq!(demo["modelReplaced"], serde_json::json!(true));

    // The file-backed replacement handles /model with its replacement arg.
    let trace = demo["trace"].as_array().unwrap();
    let model_actions: Vec<&str> = trace
        .iter()
        .filter(|e| e["action"] == "model_command")
        .map(|e| e["value"].as_str().unwrap())
        .collect();
    assert_eq!(model_actions, vec!["replacement-default", "replacement:openai"]);

    // The builtin /settings route is untouched and still dispatches.
    let settings_handled = trace.iter().any(|e| e["action"] == "set_text" && e["value"] == "");
    assert!(settings_handled, "builtin /settings still routes: {trace:?}");

    // Real handle_submit path: a replacement command does not leak into prompt.
    let route = host
        .call_command(
            "interactive-submit-route",
            &serde_json::json!({ "texts": ["/model"], "cwd": root.path() }).to_string(),
        )
        .unwrap()
        .unwrap();
    let route_trace = route["trace"].as_array().unwrap();
    assert!(
        route_trace.iter().any(|e| e["action"] == "model_command"),
        "interactive-submit-route should route /model through replacement: {route_trace:?}"
    );
    assert!(
        !route_trace.iter().any(|e| e["action"] == "prompt"),
        "replaced /model must not fall through to prompt: {route_trace:?}"
    );
}

/// PLAN 9.10 bash-tool factory: an ordinary file-backed package resolves the
/// public `pi.tools.bash` exact-version module (the Pi `createBashTool`
/// equivalent) and builds a bash tool definition with a spawnHook + custom
/// operations — the same surface Pi exposes as `createBashTool(cwd, options)`.
/// This is the additive app-side policy export (no new host mechanism); the
/// default bash tool is built through the identical factory, so the shipped
/// default and file-backed replacements share one definition.
#[test]
fn file_backed_package_uses_the_public_bash_tool_factory() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let report = DEFAULT_MANIFEST.load(&host, &[]).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    // A file-backed package requires `pi.tools.bash`, builds a bash tool with
    // a spawnHook, and registers a command that reflects the factory shape.
    let source = r#"local pi = ...
local bash_mod = pi.module.require("pi.tools.bash", "1")
local listed = {}
for _, entry in ipairs(pi.module.list()) do
  listed[entry.name .. "@" .. entry.version] = true
end

-- createBashTool(cwd, { spawnHook }): the hook prepends a marker to the
-- command; the returned definition is a registerable bash tool. We keep the
-- ops from the public createLocalBashOperations re-export.
local tool = bash_mod.create_bash_tool(pi.cwd(), {
  spawnHook = function(ctx)
    return { command = "echo pfx; " .. ctx.command, cwd = ctx.cwd }
  end,
})
local ops = bash_mod.create_local_bash_operations({ shellPath = "" })

pi.register_command("bash-factory-demo", {
  handler = function()
    return {
      listed = listed["pi.tools.bash@1"] or false,
      name = tool.name,
      hasExecute = type(tool.execute) == "function",
      hasRenderCall = type(tool.renderCall) == "function",
      hasRenderResult = type(tool.renderResult) == "function",
      paramsCommand = tool.parameters.required[1] == "command",
      opsExec = type(ops.exec) == "function",
    }
  end,
})
"#;
    host.load("file/bash-factory-demo.lua", source)
        .expect("file-backed bash factory consumer loads");
    let result = host
        .call_command("bash-factory-demo", "")
        .expect("bash-factory-demo runs")
        .expect("bash-factory-demo result");
    assert_eq!(result["listed"], true, "pi.tools.bash@1 is a public module");
    assert_eq!(result["name"], "bash");
    assert_eq!(result["hasExecute"], true);
    assert_eq!(result["hasRenderCall"], true);
    assert_eq!(result["hasRenderResult"], true);
    assert_eq!(result["paramsCommand"], true);
    assert_eq!(result["opsExec"], true);
}
