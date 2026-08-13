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
