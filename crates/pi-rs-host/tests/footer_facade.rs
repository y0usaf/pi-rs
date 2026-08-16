//! `pi.footer` — the footer data-provider facade dogfood UI extensions
//! (minimal-editor) read for live status: `get_git_branch`, `extension_statuses`
//! (+ per-extension `set_extension_status`/`clear_extension_status`),
//! `available_provider_count` (composed with `pi.ai.available_models`), and
//! `on_branch_change`/`notify_branch_change` pub/sub. Exercised unprivileged
//! from file-backed extensions through the public Lua surface.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

// True when the `git` binary is actually usable on PATH (supported `init -b`).
// The Nix test sandbox may not provide git, so the real-git test skips there
// rather than panicking on `Command::new("git")` failing to spawn.
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn git(args: &[&str], cwd: &Path) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

fn write(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).unwrap();
}

#[test]
fn footer_status_provider_count_and_branch_subscription() {
    let host = host();
    host.load(
        "<status>",
        r#"
            local pi = ...
            pi.footer.set_extension_status("janitor", "cleaning 3 hunks")
            pi.register_command("footer-status", {
                handler = function()
                    return { statuses = pi.footer.extension_statuses() }
                end,
            })
        "#,
    )
    .unwrap();
    host.load(
        "<sub>",
        r#"
            local pi = ...
            local seen = {}
            local dispose = pi.footer.on_branch_change(function(branch)
                seen[#seen + 1] = branch
            end)
            pi.register_command("footer-sub", {
                handler = function()
                    pi.footer.notify_branch_change("main")
                    pi.footer.notify_branch_change("feature/x")
                    return { seen = seen }
                end,
            })
            pi.register_command("footer-clear-sub", {
                handler = function()
                    dispose()
                    pi.footer.notify_branch_change("after-dispose")
                    return { seen = seen }
                end,
            })
        "#,
    )
    .unwrap();

    // `available_provider_count` is async and composes `pi.ai.available_models`.
    host.load(
        "<count>",
        r#"
            local pi = ...
            -- Seed a demo API key so at least one model is available
            -- (hermetic in both sandbox and developer machines), then
            -- publish it to the model registry.
            pi.auth.set("moonshotai", { type = "api_key", key = "sk-demo" })
            pi.ai.registry_refresh()
            pi.register_command("footer-count", {
                handler = function()
                    return { count = pi.footer.available_provider_count(),
                             models = #pi.ai.available_models() }
                end,
            })
        "#,
    )
    .unwrap();
    let count = host
        .call_command("footer-count", "")
        .expect("footer-count")
        .unwrap();
    // provider count tracks the available-model count from the ai registry.
    assert_eq!(count["count"], count["models"], "{count}");
    // A demo key was seeded for a known provider, so at least one model is
    // available in both the hermetic sandbox and a developer machine.
    assert!(count["count"].as_u64().unwrap() >= 1, "{count}");

    // statuses reflect the extension's status key in order.
    let statuses = host
        .call_command("footer-status", "")
        .expect("footer-status")
        .unwrap();
    assert_eq!(statuses["statuses"][0], "cleaning 3 hunks", "{statuses}");

    // branch-change subscribers fire in subscription order (including async).
    let sub = host
        .call_command("footer-sub", "")
        .expect("footer-sub")
        .unwrap();
    assert_eq!(sub["seen"], serde_json::json!(["main", "feature/x"]), "{sub}");

    // dispose removes the subscription.
    let cleared = host
        .call_command("footer-clear-sub", "")
        .expect("footer-clear-sub")
        .unwrap();
    assert_eq!(
        cleared["seen"],
        serde_json::json!(["main", "feature/x"]),
        "no new notifications after dispose: {cleared}"
    );
}

#[test]
fn footer_get_git_branch_through_a_real_repo() {
    if !git_available() {
        // Git is not on PATH in the Nix test sandbox; skip rather than fail.
        eprintln!("skipping: git binary not available on PATH");
        return;
    }
    let host = host();
    host.load(
        "<branch>",
        r#"
            local pi = ...
            pi.register_command("footer-branch", {
                handler = function(args)
                    local cwd = pi.json.decode(args).cwd
                    return { branch = pi.footer.get_git_branch(cwd) }
                end,
            })
        "#,
    )
    .unwrap();

    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&["init", "-q", "-b", "zeta"], &repo);
    write(&repo, "a.txt", "a");
    git(&["add", "a.txt"], &repo);
    git(&["commit", "-q", "-m", "init"], &repo);

    let got = host
        .call_command(
            "footer-branch",
            &serde_json::json!({ "cwd": repo.to_string_lossy() }).to_string(),
        )
        .expect("footer-branch")
        .unwrap();
    assert_eq!(got["branch"], "zeta", "{got}");
}
