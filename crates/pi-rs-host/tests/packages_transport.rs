#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Deterministic fixture tests for the three package transports and the
//! pinned package-manager.ts outcomes (PLAN 9.7): source grammar, install
//! roots, identity/dedupe, offline cache, install/remove/list/update/config
//! persistence. npm and git run through PATH shims so the tests never touch
//! the network; package JavaScript is never evaluated — only the pi manifest
//! and .lua/.md/.json resources are read.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pi_rs_host::packages::{
    PackageManager, Scope, is_local_path, parse_git_url, parse_source, package_identity,
};
use pi_rs_host::settings_manager::{SettingsManager, SettingsManagerCreateOptions};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn manager(cwd: &Path, agent_dir: &Path) -> PackageManager {
    let settings = Arc::new(Mutex::new(SettingsManager::create(
        cwd,
        Some(agent_dir.to_path_buf()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    )));
    PackageManager::new(&cwd.to_string_lossy(), &agent_dir.to_string_lossy(), settings)
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn fixture_package(root: &Path, name: &str, version: &str) -> PathBuf {
    let dir = root.join("packages").join(name);
    write(
        &dir.join("package.json"),
        &format!(
            r#"{{"name":"{name}","version":"{version}","pi":{{"extensions":["init.lua"],"themes":["themes/{name}.json"]}}}}"#
        ),
    );
    write(&dir.join("init.lua"), "-- pure Lua entry\n");
    write(&dir.join("themes").join(format!("{name}.json")), r##"{"accent":"#000000"}"##);
    // JavaScript must stay inert: a JS sibling of the Lua entry is never loaded.
    write(&dir.join("index.js"), "process.exit(99); // never evaluated\n");
    dir
}

fn npm_shim(root: &Path, registry: &Path) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("npm");
    let script = format!(
        r#"#!/bin/sh
# Deterministic fake npm: copies the fixture package into --prefix's node_modules.
mode="$1"
if [ "$mode" = "install" ]; then
  shift
  spec="$1"
  while [ $# -gt 0 ]; do
    case "$1" in
      --prefix) shift; prefix="$1" ;;
    esac
    shift
  done
  name="${{spec%%@*}}"
  mkdir -p "$prefix/node_modules/$name"
  cp -r "{0}"/"$name"/* "$prefix/node_modules/$name/"
  exit 0
fi
if [ "$mode" = "uninstall" ]; then
  shift
  name="$1"
  while [ $# -gt 0 ]; do
    case "$1" in
      --prefix) shift; prefix="$1" ;;
    esac
    shift
  done
  rm -rf "$prefix/node_modules/$name"
  exit 0
fi
if [ "$mode" = "root" ]; then
  echo "{0}"
  exit 0
fi
exit 0
"#,
        registry.display()
    );
    std::fs::write(&shim, script).unwrap();
    let mut perms = std::fs::metadata(&shim).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&shim, perms).unwrap();
    shim
}

fn git_shim(root: &Path, registry: &Path) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("git");
    let script = format!(
        r#"#!/bin/sh
# Deterministic fake git: clone copies a fixture repo; ref ops record markers.
case "$1" in
  clone)
    repo="$2"; dir="$3"
    name="${{repo##*/}}"
    name="${{name%%.git}}"
    rm -rf "$dir"
    cp -r "{0}/$name" "$dir"
    echo "cloned $name"
    ;;
  checkout)
    echo "$2" > "$PWD/.pi-fake-ref"
    ;;
  "rev-parse")
    echo "abcdef1234567890abcdef1234567890abcdef12"
    ;;
  fetch)
    ;;
  reset)
    ;;
  clean)
    ;;
esac
exit 0
"#,
        registry.display()
    );
    std::fs::write(&shim, script).unwrap();
    let mut perms = std::fs::metadata(&shim).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&shim, perms).unwrap();
    shim
}

fn git_fixture(root: &Path, name: &str) -> PathBuf {
    let dir = root.join("repos").join(name);
    write(
        &dir.join("package.json"),
        r#"{"name":"git-pkg","version":"1.0.0","pi":{"skills":["skills/SKILL.md"]}}"#,
    );
    write(&dir.join("skills").join("SKILL.md"), "# Fixture skill\n");
    dir
}

#[test]
fn source_grammar_matches_pi() {
    let npm = parse_source("npm:@scope/pkg@1.2.3");
    match npm {
        pi_rs_host::packages::ParsedSource::Npm(source) => {
            assert_eq!(source.name, "@scope/pkg");
            assert!(source.pinned);
        }
        _ => panic!("expected npm source"),
    }
    let npm = parse_source("npm:pkg");
    match npm {
        pi_rs_host::packages::ParsedSource::Npm(source) => {
            assert_eq!(source.name, "pkg");
            assert!(!source.pinned);
        }
        _ => panic!("expected npm source"),
    }
    let git = parse_source("https://github.com/acme/pi-demo.git@main").unwrap_git();
    assert_eq!(git.host, "github.com");
    assert_eq!(git.path, "acme/pi-demo");
    assert_eq!(git.r#ref.as_deref(), Some("main"));
    assert!(git.pinned);

    let git = parse_git_url("git:git@github.com:acme/pi-demo.git").unwrap();
    assert_eq!(git.host, "github.com");
    assert_eq!(git.path, "acme/pi-demo");
    assert!(!git.pinned);

    let git = parse_git_url("git:github.com/acme/pi-demo#v2").unwrap();
    assert_eq!(git.r#ref.as_deref(), Some("v2"));

    assert!(is_local_path("./packages/demo"));
    assert!(!is_local_path("npm:demo"));
    assert!(!is_local_path("https://example.com/a"));
    match parse_source("./local/dir") {
        pi_rs_host::packages::ParsedSource::Local(source) => assert_eq!(source.path, "./local/dir"),
        _ => panic!("expected local source"),
    }
}

trait UnwrapGit {
    fn unwrap_git(self) -> pi_rs_host::packages::GitSource;
}
impl UnwrapGit for pi_rs_host::packages::ParsedSource {
    fn unwrap_git(self) -> pi_rs_host::packages::GitSource {
        match self {
            pi_rs_host::packages::ParsedSource::Git(source) => source,
            _ => panic!("expected git source"),
        }
    }
}

#[test]
fn install_roots_identity_and_dedupe_match_pi() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    let manager = manager(&cwd, &agent);

    let npm = match parse_source("npm:demo") {
        pi_rs_host::packages::ParsedSource::Npm(source) => source,
        _ => panic!(),
    };
    assert_eq!(
        manager.get_npm_install_path(&npm, Scope::User),
        agent.join("npm/node_modules/demo")
    );
    assert_eq!(
        manager.get_npm_install_path(&npm, Scope::Project),
        cwd.join(".pi/npm/node_modules/demo")
    );

    let git = match parse_source("https://github.com/acme/pi-demo.git") {
        pi_rs_host::packages::ParsedSource::Git(source) => source,
        _ => panic!(),
    };
    assert_eq!(
        manager.get_git_install_path(&git, Scope::User).unwrap(),
        agent.join("git/github.com/acme/pi-demo")
    );
    assert_eq!(
        manager.get_git_install_path(&git, Scope::Project).unwrap(),
        cwd.join(".pi/git/github.com/acme/pi-demo")
    );

    // SSH (ssh:// protocol) and HTTPS forms of the same repo share one identity.
    let https = "https://github.com/acme/pi-demo.git";
    let ssh = "ssh://git@github.com/acme/pi-demo.git";
    assert_eq!(
        manager.get_package_identity(https, Some(Scope::User)),
        manager.get_package_identity(ssh, Some(Scope::User))
    );

    // npm identity ignores the version.
    assert_eq!(
        manager.get_package_identity("npm:demo@1", Some(Scope::User)),
        manager.get_package_identity("npm:demo@2", Some(Scope::User))
    );

    // Dedupe: project wins over user; same scope keeps the first.
    let deduped = manager.dedupe_packages(&[
        ("npm:demo".to_owned(), Scope::User),
        ("npm:demo@2".to_owned(), Scope::Project),
    ]);
    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].1, Scope::Project);
    assert_eq!(deduped[0].0, "npm:demo@2");

    let identity = package_identity(&parse_source("./pkg"), Scope::Project, &cwd, &agent);
    assert!(identity.contains(".pi"));
}

#[test]
fn local_install_persists_lists_and_config_outcome() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    let package = fixture_package(root.path(), "local-pkg", "1.0.0");
    let manager = manager(&cwd, &agent);

    manager.install_and_persist(&package.to_string_lossy(), Scope::User).unwrap();
    assert!(manager.get_installed_path(&package.to_string_lossy(), Scope::User).is_some());

    // install_and_persist wrote the package into the canonical config.lua.
    let config = std::fs::read_to_string(agent.join("config.lua")).unwrap();
    assert!(config.contains("packages"), "{config}");
    assert!(config.contains("local-pkg"), "{config}");

    let listed = manager.list();
    assert_eq!(listed.len(), 1);
    // Pi stores local sources relative to the scope base.
    assert_eq!(listed[0].source, "../packages/local-pkg");
    assert_eq!(listed[0].scope, Scope::User);
    assert!(!listed[0].filtered);

    // Re-adding the same source is a no-op (returns false, no duplicate).
    assert!(!manager.add_source_to_settings(&package.to_string_lossy(), Scope::User));

    // Removing the local source removes the settings entry; local remove is a no-op.
    assert!(manager.remove_and_persist(&package.to_string_lossy(), Scope::User).unwrap());
    assert!(manager.list().is_empty());
    let config = std::fs::read_to_string(agent.join("config.lua")).unwrap();
    assert!(!config.contains("local-pkg"), "{config}");
}

#[test]
fn npm_transport_installs_archive_without_evaluating_js() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    fixture_package(root.path(), "npm-pkg", "2.0.0");
    let shim = npm_shim(root.path(), &root.path().join("packages"));

    let settings = Arc::new(Mutex::new(SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    )));
    settings
        .lock()
        .unwrap()
        .set_npm_command(Some(&[shim.to_string_lossy().into_owned()]));
    let manager = PackageManager::new(&cwd.to_string_lossy(), &agent.to_string_lossy(), settings);

    manager.install_and_persist("npm:npm-pkg@2.0.0", Scope::User).unwrap();

    let installed = agent.join("npm/node_modules/npm-pkg");
    assert!(installed.join("package.json").exists(), "npm archive materialized");
    assert!(installed.join("init.lua").exists());
    assert!(installed.join("index.js").exists(), "JS is present as inert data");
    assert_eq!(
        manager.get_installed_path("npm:npm-pkg", Scope::User),
        Some(installed.to_string_lossy().into_owned())
    );

    // update() on a pinned npm version does not reinstall (spec: pinned fixed).
    manager.update(Some("npm:npm-pkg")).unwrap();

    // remove deletes the managed install and the persisted entry.
    assert!(manager.remove_and_persist("npm:npm-pkg", Scope::User).unwrap());
    assert!(!installed.exists());
    assert!(manager.list().is_empty());
}

#[test]
fn git_transport_clones_ref_and_removes() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    git_fixture(root.path(), "pi-demo");
    let shim = git_shim(root.path(), &root.path().join("repos"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", shim.parent().unwrap().display(), old_path);
    unsafe { std::env::set_var("PATH", &new_path) };

    let manager = manager(&cwd, &agent);
    manager
        .install_and_persist("https://github.com/acme/pi-demo.git@v1", Scope::User)
        .unwrap();

    let installed = agent.join("git/github.com/acme/pi-demo");
    assert!(installed.join("package.json").exists(), "git clone materialized");
    assert!(installed.join(".pi-fake-ref").exists(), "ref checkout ran");

    // Pinned git refs reconcile on update.
    manager.update(Some("https://github.com/acme/pi-demo.git")).unwrap();

    assert!(manager.remove_and_persist("https://github.com/acme/pi-demo.git", Scope::User).unwrap());
    assert!(!installed.exists());
    assert!(!agent.join("git/github.com").exists(), "empty git parents pruned");
    unsafe { std::env::set_var("PATH", &old_path) };
}

#[test]
fn project_scope_refuses_untrusted_and_offline_skips_updates() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    let package = fixture_package(root.path(), "trusted-pkg", "1.0.0");

    let settings = Arc::new(Mutex::new(SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(false),
        },
    )));
    let manager = PackageManager::new(&cwd.to_string_lossy(), &agent.to_string_lossy(), settings);

    // Untrusted project scope refuses to touch project package storage.
    let error = manager
        .install(&package.to_string_lossy(), Scope::Project)
        .unwrap_err();
    assert!(error.contains("not trusted"), "{error}");

    // Offline mode: update() returns without touching the network or failing.
    unsafe { std::env::set_var("PI_OFFLINE", "1") };
    manager.update(None).unwrap();
    manager.update(Some("npm:nothing")).unwrap();
    unsafe { std::env::remove_var("PI_OFFLINE") };
}
