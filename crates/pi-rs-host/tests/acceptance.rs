//! WS1 acceptance run (DESIGN.md): one end-to-end scenario through the
//! public API only —
//! - an untrusted `.pi/` project resolves to `Ask` (the substrate
//!   exposes the decision; prompting is the frontend's, headless = no)
//!   and discovery skips the project-local extensions
//! - embedded packs and discovered extensions load through the same path
//! - a handler awaits a host future and returns
//! - a hung handler is killed by the watchdog without stopping the rest

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use pi_rs_host::trust::{
    ProjectTrustStore, ResolveProjectTrust, TrustResolution, resolve_project_trusted,
};
use pi_rs_host::{EmbeddedPack, Host, HostConfig, discover};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pi-rs-acceptance-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn ws1_acceptance_run() {
    let root = temp_dir("ws1");
    let proj = root.join("proj");
    let agent = root.join("agent");

    // A project that would register a tool if trusted…
    write(
        &proj.join(".pi").join("extensions").join("project.lua"),
        r#"
            local pi = ...
            pi.register_tool({
                name = "project_tool",
                description = "must not load untrusted",
                parameters = { type = "object", properties = {} },
                execute = function() return { content = {} } end,
            })
        "#,
    );
    // …and a global extension: an awaiting handler, a hung handler, and
    // a survivor after the hang.
    write(
        &agent.join("extensions").join("global.lua"),
        r#"
            local pi = ...
            pi.on("work", function(event)
                pi.sleep(10)
                return { awaited = true, got = event.n }
            end)
            pi.on("work", function() while true do end end)
            pi.on("work", function() return "survivor" end)
        "#,
    );

    // 1. Trust: `.pi/` exists, nothing stored, no override, no
    //    extension answer → the substrate says Ask; headless maps Ask to
    //    untrusted (spec: `!hasUI → false`).
    let store = ProjectTrustStore::new(&s(&agent));
    let resolution = resolve_project_trusted(&ResolveProjectTrust {
        cwd: &s(&proj),
        store: &store,
        trust_override: None,
        default_project_trust: None,
        extension_result: None,
    })
    .expect("resolve");
    assert_eq!(resolution, TrustResolution::Ask);
    let project_trusted = false;

    // 2. Discovery skips the untrusted project dir.
    let paths = discover::discover_extension_paths(&[], &s(&proj), &s(&agent), project_trusted);
    assert_eq!(paths, vec![s(&agent.join("extensions").join("global.lua"))]);

    // 3. Embedded pack + discovered extensions through the same load path.
    let host = Host::new(HostConfig {
        dispatch_timeout_ms: 200,
        cwd: Some(s(&proj)),
        project_trusted,
    })
    .expect("host starts");
    let embedded = host.load_embedded(&[EmbeddedPack {
        name: "pack:hello",
        source: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/extensions/hello.lua"
        )),
    }]);
    assert!(embedded.errors.is_empty());
    let report = host.load_extensions(&paths);
    assert!(report.errors.is_empty());
    assert_eq!(report.loaded, paths);

    // The untrusted project tool never registered; the embedded one did.
    let tool_names: Vec<String> = host
        .tools()
        .expect("tools")
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(tool_names, vec!["hello".to_owned()]);

    // 4. Emit: await works, the hang is killed, the survivor still runs.
    let outcomes = host
        .emit("work", &serde_json::json!({ "n": 7 }))
        .expect("emit");
    assert_eq!(outcomes.len(), 3);
    assert_eq!(
        outcomes[0].result.as_ref().expect("awaited ok").as_ref(),
        Some(&serde_json::json!({ "awaited": true, "got": 7 }))
    );
    let err = outcomes[1].result.as_ref().expect_err("watchdog kill");
    assert!(err.contains("watchdog"), "watchdog error, got: {err}");
    assert_eq!(
        outcomes[2].result.as_ref().expect("survivor ok").as_ref(),
        Some(&serde_json::json!({ "message": "survivor" }))
    );
}
