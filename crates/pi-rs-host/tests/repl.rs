//! P1 pi.repl tier-2 binding acceptance: the file-backed exerciser
//! (examples/extensions/repl-demo.lua) proves the kernel bridge works from
//! ordinary Lua policy: spawn, host_request pump, execute with persistence,
//! stream capture, exceptions, snapshot/restore, and VM-drop disposal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use std::time::Duration;

fn host() -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 90_000,
        cwd: None,
        project_trusted: true,
    })
    .expect("host starts")
}

fn load_demo(host: &Host) {
    host.load(
        "repl-demo.lua",
        include_str!("../../../examples/extensions/repl-demo.lua"),
    )
    .expect("exerciser loads");
}

fn call(host: &Host, command: &str, args: &str) -> serde_json::Value {
    host.call_command(command, args)
        .expect("command runs")
        .expect("command returns a value")
}

/// Count live kernel-shim python processes (cmdline contains kernel-shim).
fn shim_count() -> usize {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
                .filter(|e| {
                    std::fs::read_to_string(e.path().join("cmdline"))
                        .map(|c| c.contains("kernel-shim"))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn repl_basic_kernel_lifecycle() {
    let host = host();
    load_demo(&host);
    // Kernel startup (IPython import in the Nix store) can take ~20s on a
    // cold cache; the exerciser is the gate, so give it room.
    let out = call(&host, "repl-basic", "");
    eprintln!("repl-basic out: {}", out);
    assert_eq!(out["persistence"], "2");
    assert_eq!(out["host_request_ok"], true);
    assert!(out["stdout"].as_str().unwrap().contains("out"));
    assert!(out["stderr"].as_str().unwrap().contains("err"));
    assert_eq!(out["exception"], "ZeroDivisionError");
}

#[test]
fn repl_snapshot_restore_round_trip() {
    let host = host();
    load_demo(&host);
    let out = call(&host, "repl-snapshot", "");
    assert_eq!(out["snapshot_saved_x"], true);
    assert_eq!(out["restore_restored_x"], true);
    assert_eq!(out["value_after_restore"], "142");
}

#[test]
fn repl_kernel_is_disposed_at_vm_drop() {
    let baseline = shim_count();
    let host = host();
    load_demo(&host);
    let out = call(&host, "repl-leak", "");
    assert_eq!(out["value"], "2");
    // The kernel was deliberately not shut down; dropping the host must
    // dispose it (resources::dispose_all runs the kernel shutdown).
    drop(host);
    // Give the async shutdown task a moment to SIGKILL the shim.
    std::thread::sleep(Duration::from_millis(1500));
    let after = shim_count();
    assert_eq!(after, baseline, "kernel shim leaked past VM drop");
}
