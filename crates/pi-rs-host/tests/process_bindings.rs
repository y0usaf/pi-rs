#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_raw_strings
)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

fn run(host: &Host, body: &str) -> serde_json::Value {
    host.load(
        "test://run",
        &format!(
            r#"
            local pi = ...
            pi.register_command("run", {{
                handler = function()
                    {body}
                end,
            }})
            "#
        ),
    )
    .unwrap();
    host.call_command("run", "").unwrap().unwrap()
}

#[test]
fn process_spawn_drives_pipes_and_waits() {
    let host = host();
    host.load(
        "process-demo.lua",
        include_str!("../../../examples/extensions/process-demo.lua"),
    )
    .unwrap();
    let out = host.call_command("process-demo", "").unwrap().unwrap();
    assert_eq!(out["pid_positive"], true);
    assert_eq!(out["out"], "got:ping\n");
    assert_eq!(out["err"], "errline\n");
    assert_eq!(out["code"], 3);
    assert_eq!(out["running_after_wait"], false);
}

#[test]
fn process_signal_aborts_tree() {
    let host = host();
    host.load(
        "process-demo.lua",
        include_str!("../../../examples/extensions/process-demo.lua"),
    )
    .unwrap();
    let out = host.call_command("process-abort", "").unwrap().unwrap();
    assert!(out["pid"].as_u64().unwrap() > 0);
    // signal-driven kill leaves no exit code (nil => null).
    assert_eq!(out["signal_killed"], true);
    assert_eq!(out["code"], serde_json::Value::Null);
}

#[test]
fn process_handle_disposal_reaps_tree() {
    let host = host();
    host.load(
        "process-demo.lua",
        include_str!("../../../examples/extensions/process-demo.lua"),
    )
    .unwrap();
    let out = host.call_command("process-disposal", "").unwrap().unwrap();
    assert_eq!(out["spawned"], true);
    assert_eq!(out["terminated"], true);
}

#[test]
fn process_spawn_failure_throws() {
    let host = host();
    let out = run(
        &host,
        r#"
            local ok, err = pcall(pi.process.spawn, "definitely-not-a-real-binary-xyz")
            return { ok = ok, mentions_error = tostring(err):find("process.spawn") ~= nil }
        "#,
    );
    assert_eq!(out["ok"], false);
    assert_eq!(out["mentions_error"], true);
}

#[test]
fn process_kill_accepts_pid() {
    let host = host();
    let out = run(
        &host,
        r#"
            local p = pi.process.spawn("sh", { "-c", "sleep 60" })
            local pid = p:pid()
            pi.process.kill(pid) -- SIGTERM the tree
            local code = p:wait()
            return { pid = pid, killed = code == nil }
        "#,
    );
    assert!(out["pid"].as_u64().unwrap() > 0);
    assert_eq!(out["killed"], true);
}

#[test]
fn process_kill_zero_is_ignored() {
    let host = host();
    // pid 0 signals the *caller's* whole process group; the guard must ignore
    // it rather than SIGTERM every process the test shares the group with.
    let out = run(
        &host,
        r#"
            pi.process.kill(0)
            return { ok = true }
        "#,
    );
    assert_eq!(out["ok"], true);
}
