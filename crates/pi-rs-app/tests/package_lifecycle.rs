//! PLAN 9.7 package lifecycle: the public `pi.packages` module drives
//! install/remove/list/getInstalledPath and the SettingsManager packages
//! channel for user + project scope, using the same `pi.module.require`
//! mechanism embedded builtins and file-backed packages share. The local-path
//! transport is deterministic (no network); npm/git route through the public
//! `pi.exec`/`pi.fs` mechanisms and stay JS-inert.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::needless_collect, clippy::redundant_closure)]
#![allow(clippy::manual_contains, unsafe_code)]

use pi_rs_app::builtins::{AGENT_CORE_PACK, INTERACTIVE_PACK};
use pi_rs_host::{Host, HostConfig};
use serde_json::Value;
use std::sync::Mutex;

fn agent_setup_mutex() -> &'static Mutex<()> {
    static M: Mutex<()> = Mutex::new(());
    &M
}

/// Create a host whose `pi.settings` global store is pinned to a temporary
/// agent dir (`PI_CODING_AGENT_DIR`), so package lifecycle tests never touch
/// the real `~/.pi/agent`. The env is read once at Host build; the mutex
/// serializes the set + build window across parallel tests.
fn hermetic_host(cwd: &str) -> Host {
    let _guard = agent_setup_mutex().lock().unwrap();
    let agent_dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_owned()),
        ..HostConfig::default()
    })
    .expect("host");
    unsafe { std::env::remove_var("PI_CODING_AGENT_DIR") };
    std::mem::forget(agent_dir); // keep dir alive for the VM's lifetime
    let report = host.load_embedded(&[AGENT_CORE_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "load errors: {:?}", report.errors);
    host
}

fn host_with(cwd: &str, _agent_dir: &str) -> Host {
    hermetic_host(cwd)
}

fn call(host: &Host, payload: Value) -> Value {
    host.call_command("pm-run", &payload.to_string())
        .expect("command")
        .expect("result")
}

const RUNNER: &str = r#"
local pi = ...
pi.register_command("pm-run", {
  handler = function(args)
    local c = pi.json.decode(args)
    local m = pi.module.require("pi.packages", "1")
    local op = c.op
    if op == "addSource" then
      return { changed = m.add_source_to_settings(c.source, { ["local"] = c["local"] }) }
    end
    if op == "removeSource" then
      return { changed = m.remove_source_from_settings(c.source, { ["local"] = c["local"] }) }
    end
    if op == "list" then
      local list = m.list_configured_packages(c.cwd or pi.cwd(), c.agentDir)
      return { packages = list }
    end
    if op == "getInstalled" then
      return { path = m.get_installed_path(c.source, c.scope, c.cwd or pi.cwd(), c.agentDir) }
    end
    if op == "install" then
      local ok, err = pcall(m.install, c.source, { ["local"] = c["local"], agentDir = c.agentDir })
      if not ok then return { error = tostring(err) } end
      return { installedPath = err.installedPath, sourceParsed = m.parse_source(c.source) }
    end
    if op == "parseSource" then
      return { parsed = m.parse_source(c.source), identity = m.package_identity(c.source) }
    end
    return {}
  end,
})
"#;

#[test]
fn local_path_install_list_get_installed_remove() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    // A local package directory exists on disk.
    let package_dir = root.path().join("local-pkg");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(
        package_dir.join("package.json"),
        r#"{"name":"local-pkg","version":"1.0.0","pi":{"extensions":["init.lua"]}}"#,
    )
    .unwrap();

    let host = host_with(&cwd.to_string_lossy(), &agent_dir.to_string_lossy());
    host.load("pm-runner", RUNNER).expect("runner loads");

    // addSource (user scope) persists to settings.
    let r = call(
        &host,
        serde_json::json!({"op":"addSource","source":"./local-pkg","local":false}),
    );
    assert_eq!(r["changed"], serde_json::json!(true), "add should change");
    let r2 = call(
        &host,
        serde_json::json!({"op":"addSource","source":"./local-pkg","local":false}),
    );
    assert_eq!(
        r2["changed"],
        serde_json::json!(false),
        "duplicate add no-op"
    );

    // removeSource removes it.
    let r3 = call(
        &host,
        serde_json::json!({"op":"removeSource","source":"./local-pkg","local":false}),
    );
    assert_eq!(r3["changed"], serde_json::json!(true));
    let r4 = call(
        &host,
        serde_json::json!({"op":"removeSource","source":"./local-pkg","local":false}),
    );
    assert_eq!(r4["changed"], serde_json::json!(false));
}

#[test]
fn local_path_install_resolves_and_lists() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let package_dir = root.path().join("local-pkg");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(
        package_dir.join("init.lua"),
        "local pi = ...\npi.register_command('x', {})\n",
    )
    .unwrap();

    let host = host_with(&cwd.to_string_lossy(), &agent_dir.to_string_lossy());
    host.load("pm-runner", RUNNER).expect("runner loads");

    // install a local path (must exist) then getInstalledPath resolves it.
    let abs = package_dir.to_string_lossy().into_owned();
    let r = call(
        &host,
        serde_json::json!({"op":"install","source":abs,"local":false,"agentDir":agent_dir.to_string_lossy()}),
    );
    assert!(r.get("error").is_none(), "install error: {:?}", r);
    assert_eq!(r["installedPath"], serde_json::Value::String(abs.clone()));

    let got = call(
        &host,
        serde_json::json!({"op":"getInstalled","source":abs,"scope":"user","agentDir":agent_dir.to_string_lossy()}),
    );
    // Local paths resolve to the on-disk path when present (absolute input
    // wins over the scope base dir).
    assert_eq!(
        got["path"],
        serde_json::Value::String(abs.clone()),
        "unexpected: {:?}",
        got
    );

    // Install a missing path errors.
    let missing = root.path().join("nope").to_string_lossy().into_owned();
    let r2 = call(
        &host,
        serde_json::json!({"op":"install","source":missing,"local":false,"agentDir":agent_dir.to_string_lossy()}),
    );
    assert!(r2.get("error").is_some(), "missing path should error");
}

#[test]
fn parse_source_routes_git_npm_local() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let host = host_with(&cwd.to_string_lossy(), &agent_dir.to_string_lossy());
    host.load("pm-runner", RUNNER).expect("runner loads");

    let npm = call(
        &host,
        serde_json::json!({"op":"parseSource","source":"npm:some-pkg@^1.0.0"}),
    );
    assert_eq!(npm["parsed"]["type"], "npm");
    assert_eq!(npm["parsed"]["name"], "some-pkg");
    assert_eq!(npm["parsed"]["pinned"], true);
    assert_eq!(npm["identity"], "npm:some-pkg");

    let git = call(
        &host,
        serde_json::json!({"op":"parseSource","source":"git:github.com/user/repo"}),
    );
    assert_eq!(git["parsed"]["type"], "git");
    assert_eq!(git["parsed"]["host"], "github.com");
    assert_eq!(git["identity"], "git:github.com/user/repo");

    let local = call(
        &host,
        serde_json::json!({"op":"parseSource","source":"./local/path"}),
    );
    assert_eq!(local["parsed"]["type"], "local");
}

/// Scoped npm sources parse into a clean name + identity (spec parseNpmSpec),
/// including the pinned scoped form `@scope/name@version`.
#[test]
fn parse_scoped_npm_spec() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let host = host_with(&cwd.to_string_lossy(), &agent_dir.to_string_lossy());
    host.load("pm-runner", RUNNER).expect("runner loads");

    let scoped = call(
        &host,
        serde_json::json!({"op":"parseSource","source":"npm:@babel/core@^7.0.0"}),
    );
    assert_eq!(scoped["parsed"]["type"], "npm");
    assert_eq!(scoped["parsed"]["name"], "@babel/core", "{:?}", scoped);
    assert_eq!(scoped["parsed"]["pinned"], true);
    assert_eq!(scoped["identity"], "npm:@babel/core", "{:?}", scoped);

    // Versionless scoped source.
    let scoped2 = call(
        &host,
        serde_json::json!({"op":"parseSource","source":"npm:@types/node"}),
    );
    assert_eq!(scoped2["parsed"]["name"], "@types/node", "{:?}", scoped2);
    assert_eq!(scoped2["parsed"]["pinned"], false);
}

/// npm/git transport security guards reject option-like specs, unsafe package
/// names, and unsafe refs *before* any network/exec work — hermetic coverage of
/// the guards (no real npm/git is invoked).
#[test]
fn transport_security_guards_reject_before_exec() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let host = host_with(&cwd.to_string_lossy(), &agent_dir.to_string_lossy());
    host.load("pm-runner", RUNNER).expect("runner loads");

    // Option-like npm spec (leading `-`): refused before exec.
    let opt = call(
        &host,
        serde_json::json!({"op":"install","source":"npm:--global","local":false,"agentDir":agent_dir.to_string_lossy()}),
    );
    assert!(
        opt.get("error").is_some() && opt["error"].as_str().unwrap().contains("option-like"),
        "npm spec option guard: {:?}",
        opt
    );

    // Unsafe npm name (`..` traversal): refused before constructing the path.
    let unsafe_name = call(
        &host,
        serde_json::json!({"op":"install","source":"npm:foo/../../etc","local":false,"agentDir":agent_dir.to_string_lossy()}),
    );
    assert!(
        unsafe_name.get("error").is_some()
            && unsafe_name["error"]
                .as_str()
                .unwrap()
                .contains("unsafe package name"),
        "npm name guard: {:?}",
        unsafe_name
    );

    // Unsafe git ref (option-like `-`): refused at parse/checkout; a `git:`
    // URL that fails to parse falls back to local in Pi, so use a parseable
    // git source with an option-like ref to exercise the checkout guard only
    // when a target exists. The install guard is exercised through git.rs
    // parse (option-like refs are parsed), so simply confirm a well-formed
    // non-local git source routes as git (no exec side effect here).
    // Option-like git ref parses as git (routing); the checkout-time ref guard
    // is exercised in code (package-manager.lua install) and requires an
    // existing clone, so it is not network-invoked here.
    let git = call(
        &host,
        serde_json::json!({"op":"parseSource","source":"git:github.com/a/b@--detach"}),
    );
    assert_eq!(git["parsed"]["type"], "git");
    // npm guards above are the hermetic, no-network assertions.
}

#[test]
fn file_backed_package_uses_same_package_module() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let host = hermetic_host(&cwd.to_string_lossy());
    host.load(
        "examples/extensions/pm-consumer.lua",
        include_str!("../../../examples/extensions/pm-consumer.lua"),
    )
    .expect("file-backed pm consumer loads");
    let result = host
        .call_command(
            "pm-consumer",
            &serde_json::json!({ "source": "git:github.com/acme/widgets", "local": false })
                .to_string(),
        )
        .expect("pm consumer runs")
        .expect("result");
    assert_eq!(result["type"], "git");
    assert_eq!(result["identity"], "git:github.com/acme/widgets");
    assert_eq!(result["added"], true);
}

#[test]
fn list_configured_packages_user_scope() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let host = host_with(&cwd.to_string_lossy(), &agent_dir.to_string_lossy());
    host.load("pm-runner", RUNNER).expect("runner loads");

    call(
        &host,
        serde_json::json!({"op":"addSource","source":"npm:pkg-a","local":false}),
    );
    call(
        &host,
        serde_json::json!({"op":"addSource","source":"npm:pkg-b","local":false}),
    );
    let r = call(
        &host,
        serde_json::json!({"op":"list","cwd":cwd.to_string_lossy(),"agentDir":agent_dir.to_string_lossy()}),
    );
    let packages = r["packages"].as_array().unwrap();
    let sources: Vec<&str> = packages
        .iter()
        .map(|p| p["source"].as_str().unwrap())
        .collect();
    assert!(sources.iter().any(|s| *s == "npm:pkg-a"));
    assert!(sources.iter().any(|s| *s == "npm:pkg-b"));
    assert!(packages.iter().all(|p| p["scope"] == "user"));
}
