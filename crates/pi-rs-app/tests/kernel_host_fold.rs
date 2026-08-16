//! Stage 1 of docs/pi-kernel-surface.md — the "exactly one context" fold.
//!
//! The daemon is the host-side holder of a [`KernelBridge`] (the live host VM +
//! the active session-manager handle); the always-loaded `agent-core` pack's
//! Lua fragment (`utils/kernel-host.lua`) exposes the *composition* — a
//! `pi.kernel.mount` declaring `daemon:host:vm` and `session:active` on the ONE
//! VM-resident kernel Context — as `pi.agent.kernel-host.mount_host_lifecycle`.
//! The product run invokes it once the daemon is ready, replacing the Rust
//! `DaemonBoundary::mount_host` / `mount_session`.
//!
//! This proves, through the public path (`host.load` + `host.call_command`):
//!   - both product states mount from Lua and are present on the single kernel
//!     Context (`pi.kernel.has("daemon:host:vm")` is true);
//!   - `pi.kernel.unmount` replays the effect inverse in reverse, so both keys
//!     are gone (residue-empty);
//!   - the real host VM is torn down by the daemon fold (host-side inverse run
//!     off the VM thread where a blocking `Host::stop` is legal), so a further
//!     `host.emit` fails with `VmUnavailable`.
//!
//! Contrast: `cargo test -p pi-rs-app --test spatiotemporal_ceremony` still
//! runs the raw `DaemonBoundary` ceremony unchanged (green). A plain host that
//! never invokes the composition mounts nothing (byte-identical, residue-free).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::daemon::DaemonBoundary;
use pi_rs_host::{Host, HostConfig, HostError};

/// The mount + probe commands the test drives through the public command path.
const FOLD_DRIVER: &str = "\
local pi = ...
local kernel_host = pi.module.require('pi.agent.kernel-host', '1')
local mount_id = kernel_host.mount_host_lifecycle()
pi.register_command('kernel-host-probe', {
  description = 'Stage 1 fold report',
  handler = function()
    return {
      mountId = mount_id,
      hasHost = pi.kernel.has('daemon:host:vm'),
      hasSession = pi.kernel.has('session:active'),
    }
  end,
})
pi.register_command('kernel-host-unmount', {
  description = 'Stage 1 fold unmount',
  handler = function()
    pi.kernel.unmount(mount_id)
  end,
})
";

#[test]
fn host_and_session_mount_from_lua_and_teardown_stops_the_vm() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(dir.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap();

    // Boot the composition as agent-core (always loaded; carries the fold).
    let report = host.load_embedded(&[pi_rs_app::builtins::AGENT_CORE_PACK]);
    assert!(
        report.errors.is_empty(),
        "agent-core composition failed to load: {:?}",
        report.errors
    );

    // Product run: invoke the agent-core composition the moment the daemon is
    // ready (main.rs does the same). The two authoritative states mount FROM
    // Lua through `pi.kernel.mount` onto the single kernel Context.
    host.load("product://fold", FOLD_DRIVER)
        .expect("the agent-core fold driver loads through the public path");

    let rep = host
        .call_command("kernel-host-probe", "")
        .expect("fold reports through the public command path")
        .expect("fold report value");
    assert_eq!(rep["hasHost"], true, "daemon:host:vm not mounted from Lua");
    assert_eq!(rep["hasSession"], true, "session:active not mounted from Lua");

    // Residue-empty unmount: the inverse replays, removing both keys.
    host.call_command("kernel-host-unmount", "")
        .expect("unmount runs through the public path");
    let gone = host
        .call_command("kernel-host-probe", "")
        .expect("post-unmount report")
        .expect("post-unmount value");
    assert_eq!(gone["hasHost"], false, "residue: daemon:host:vm survives");
    assert_eq!(
        gone["hasSession"], false,
        "residue: session:active survives"
    );

    // Host teardown: the daemon fold keeps the real host host-side and stops it
    // off the VM thread (a blocking Host::stop on the VM write path would
    // deadlock). After `daemon.drain()` the product VM is really gone.
    let mut daemon = DaemonBoundary::new();
    daemon.retain_host(&host);
    daemon.drain();
    assert!(matches!(
        host.emit("ping", &serde_json::json!({})),
        Err(HostError::VmUnavailable)
    ));
}

/// A plain host never invokes the composition: the agent-core fragment is pure
/// (no load-time side effect), so it mounts nothing and leaves no residue —
/// default product byte-identical.
#[test]
fn plain_host_composes_nothing_no_residue() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(dir.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[pi_rs_app::builtins::AGENT_CORE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    // The composition is never invoked, so no state is mounted onto the
    // (empty) kernel context and no fold command exists.
    assert!(
        host.call_command("kernel-host-probe", "").is_err(),
        "fold command must not be registered on a plain host"
    );
    host.load(
        "probe://plain",
        "local pi = ... assert(not pi.kernel.has('daemon:host:vm'), 'residue on plain host')",
    )
    .unwrap();
    host.stop().unwrap();
}