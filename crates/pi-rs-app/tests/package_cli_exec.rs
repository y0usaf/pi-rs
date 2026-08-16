#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// PLAN 9.7 packages CLI *execution* legs. The parse/help/early-error surface
// is pinned by package_cli_parity.rs; this suite covers the deterministic
// legs that reach the package manager (spec `handlePackageCommand`'s
// install/remove/list clauses) by running the real `pi` binary against a
// hermetic agent dir + cwd, asserting exact stdout/stderr/exit.
//
// The deterministic legs are:
//   - `pi list` (reads settings, prints user/project packages + installed
//     paths; "No packages installed." when empty)
//   - `pi install <localPath>` / `pi remove <localPath>` (filesystem + settings)
//
// Network-modulated legs (`update`/self-update, npm/git install/update
// against live registries) are out of the deterministic fixture scope and are
// documented as such in PLAN 9.7.
use std::io::Read;
use std::process::{Command, Stdio};

fn run_pi(
    argv: &[&str],
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
) -> (i32, String, String) {
    let exe = std::env::var("PI_TEST_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_pi").to_owned());
    let mut child = Command::new(&exe)
        .args(argv)
        .current_dir(cwd)
        .env("PI_CODING_AGENT_DIR", agent_dir)
        .env("PI_OFFLINE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pi");
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    (status.code().unwrap_or(-1), stdout, stderr)
}

fn write(p: &std::path::Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// `pi list` with a configured user package prints Pi's exact format, and a
/// configured-but-uninstalled package shows no installed path.
#[test]
fn list_shows_configured_user_packages() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    write(
        &agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'npm:pkg-a' } })\n",
    );
    // Installed npm path exists -> installedPath shows.
    std::fs::create_dir_all(agent.join("npm/node_modules/pkg-a")).unwrap();

    let (code, stdout, _) = run_pi(&["list"], &cwd, &agent);
    assert_eq!(code, 0, "exit");
    assert!(
        stdout.starts_with("User packages:\n  npm:pkg-a\n    "),
        "unexpected stdout: {stdout}"
    );
    assert!(
        stdout.trim_end().ends_with("npm/node_modules/pkg-a"),
        "expected installed path suffix: {stdout}"
    );
    // No project section (no project packages).
    assert!(!stdout.contains("Project packages:"), "{stdout}");
}

/// Empty settings -> `pi list` prints Pi's dim "No packages installed." line.
#[test]
fn list_empty_shows_no_packages() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    let (code, stdout, _) = run_pi(&["list"], &cwd, &agent);
    assert_eq!(code, 0);
    assert_eq!(stdout, "No packages installed.\n");
}

/// `pi list` shows both User and Project sections, separated by a blank line,
/// with installed-path lines for present installs.
#[test]
fn list_shows_user_and_project_sections() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    write(
        &agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'npm:user-pkg' } })\n",
    );
    std::fs::create_dir_all(agent.join("npm/node_modules/user-pkg")).unwrap();
    // Project scope: a .pi/config.lua with a local package.
    write(
        &cwd.join(".pi/config.lua"),
        "local pi = ...\npi.config.settings({ packages = { './proj-pkg' } })\n",
    );
    std::fs::create_dir_all(cwd.join(".pi/proj-pkg")).unwrap();

    let (code, stdout, _) = run_pi(&["list"], &cwd, &agent);
    assert_eq!(code, 0);
    assert!(stdout.contains("User packages:\n  npm:user-pkg"));
    assert!(stdout.contains("\n\nProject packages:\n  ./proj-pkg"));
}

/// Local-path install resolves against cwd (spec install(): resolvePath against
/// this.cwd) and is persisted to settings.
#[test]
fn local_install_resolves_against_cwd_and_persists() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    // A local package dir under cwd (relative path, tilde-free).
    std::fs::create_dir_all(cwd.join("local-pkg")).unwrap();

    let (code, stdout, stderr) = run_pi(&["install", "./local-pkg"], &cwd, &agent);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Installed ./local-pkg\n");

    // The package is now persisted in user settings and appears in list.
    let (code2, out2, _) = run_pi(&["list"], &cwd, &agent);
    assert_eq!(code2, 0);
    assert!(out2.contains("./local-pkg"), "after install: {out2}");
}

/// Installing a missing local path errors with Pi's message and a nonzero exit.
#[test]
fn local_install_missing_path_errors() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    let (code, stdout, stderr) = run_pi(&["install", "./missing-path-xyz"], &cwd, &agent);
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("Error: Path does not exist:"),
        "stderr: {stderr}"
    );
}

/// Removing a configured package removes it from settings and prints Removed.
#[test]
fn local_remove_removes_from_settings() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    write(
        &agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'npm:pkg-remove-me' } })\n",
    );

    let (code, stdout, stderr) = run_pi(&["remove", "npm:pkg-remove-me"], &cwd, &agent);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Removed npm:pkg-remove-me\n");

    // Config now empty -> list shows No packages installed.
    let (code2, out2, _) = run_pi(&["list"], &cwd, &agent);
    assert_eq!(code2, 0);
    assert_eq!(out2, "No packages installed.\n");
}

/// Removing a package not present prints Pi's "No matching package found" on
/// stderr and exits 1.
#[test]
fn local_remove_nonexistent_errors() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    let (code, stdout, stderr) = run_pi(&["remove", "npm:nope"], &cwd, &agent);
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("No matching package found for npm:nope"),
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Network-modulated update/self-update legs (PLAN 9.7). run_pi sets
// PI_OFFLINE=1, so the extensions updateConfiguredSources leg short-circuits
// with no network (deterministic offline-skip); the self leg is deterministic
// for the native pi-rs install (no package reinstall command -> Pi's
// cannot-self-update error, DESIGN platform boundary). npm/git *install* and
// *update* themselves route through the public pi.exec/pi.fs mechanisms (the
// network-modulated half is exercised in package_lifecycle with a stubbed
// exec through the shared pi.packages module, never evaluating package JS).
// ---------------------------------------------------------------------------

/// `pi update --extensions` with no configured packages offline-skip: Pi's
/// updateConfiguredSources returns when sources are empty, so stdout still
/// reports "Updated packages" with exit 0 and no network.
#[test]
fn update_extensions_offline_skip_empty() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    let (code, stdout, stderr) = run_pi(&["update", "--extensions"], &cwd, &agent);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Updated packages\n");
}

/// `pi update --extensions` with a configured (even uninstalled) package
/// offline-skip: updateConfiguredSources returns when offline so a genuinely
/// uninstalled package is not installed/updated and no network runs.
#[test]
fn update_extensions_offline_skip_configured_uninstalled() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    write(
        &agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'npm:pkg-offline' } })\n",
    );
    // No install exists — the package is uninstalled but offline must be skipped.

    let (code, stdout, stderr) = run_pi(&["update", "--extensions"], &cwd, &agent);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Updated packages\n");
    // Nothing was installed (no network, no filesystem writes).
    assert!(!agent.join("npm").exists(), "offline must not create npm root");
}

/// `pi update <configured-source>` offline-skip still reports "Updated" with
/// exit 0 for the extensions leg.
#[test]
fn update_source_offline_skip_configured() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    write(
        &agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'git:github.com/acme/widgets' } })\n",
    );

    let (code, stdout, stderr) = run_pi(&["update", "git:github.com/acme/widgets"], &cwd, &agent);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Updated git:github.com/acme/widgets\n");
}

/// `pi update <unconfigured-source>` errors with Pi's no-matching-package
/// message even when offline (spec update() throws before the offline
/// updateConfiguredSources short-circuit).
#[test]
fn update_unconfigured_source_errors_offline() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    let (code, stdout, stderr) = run_pi(&["update", "npm:not-configured"], &cwd, &agent);
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("No matching package found for npm:not-configured"),
        "stderr: {stderr}"
    );
}

/// `pi update self` (and the self half of `pi update`) prints Pi's
/// cannot-self-update error for the native pi-rs install and exits nonzero
/// (DESIGN platform boundary).
#[test]
fn update_self_unavailable_for_native_install() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    let (code, stdout, stderr) = run_pi(&["update", "self"], &cwd, &agent);
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("pi cannot self-update this installation"),
        "stderr: {stderr}"
    );
}

/// `pi update` (default all): extensions leg offline-skips, then the self leg
/// reports cannot-self-update, so the process exits 1 with "Updated packages"
/// on stdout and the unavailable note on stderr (faithful Pi ordering).
#[test]
fn update_all_offline_skip_then_self_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    let (code, stdout, stderr) = run_pi(&["update"], &cwd, &agent);
    assert_eq!(code, 1);
    assert_eq!(stdout, "Updated packages\n");
    assert!(
        stderr.contains("pi cannot self-update this installation"),
        "stderr: {stderr}"
    );
}

/// `pi config` (TUI config command) routes to the `config-exec` Lua role and
/// emits the resolved resource listing deterministically (headless CLI path),
/// exiting 0. Mirrors package-manager-cli.ts handleConfigCommand.
#[test]
fn config_command_runs_resource_preamble() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    write(
        &agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'npm:pkg-config' } })\n",
    );

    let (code, stdout, stderr) = run_pi(&["config"], &cwd, &agent);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("Configured resources for"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("npm:pkg-config"),
        "resolved packages should be listed: {stdout}"
    );
}

/// A "--approve" local remove/install requires trust; without it, the project
/// write is refused. (Trust default with no project-trust inputs is true, so
/// this exercises the trusted side; the untrusted refusal is covered by the
/// unit-level package_lifecycle guards.)
#[test]
fn local_install_with_approve_succeeds() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("cwd");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    // Local install resolves the source against cwd (Pi: resolvePath against
    // process.cwd), regardless of scope.
    std::fs::create_dir_all(cwd.join("local-pkg")).unwrap();

    let (code, stdout, stderr) = run_pi(
        &["install", "./local-pkg", "--local", "--approve"],
        &cwd,
        &agent,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Installed ./local-pkg\n");
    // --local persists to project settings.
    let (_, out2, _) = run_pi(&["list"], &cwd, &agent);
    assert!(out2.contains("Project packages:"), "{out2}");
}
