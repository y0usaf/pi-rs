//! File-backed scope cancellation and effect-resource cleanup.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, HostError, PackageSource};

const PACKAGE: &str = r#"
local pi = ...
local roots = pi.roots.v1
local effects = pi.effects.v1

roots.register({
  kind="application", id="cleanup-effects", active=true, priority=0,
  dispatch=function(snapshot)
    if snapshot.event.kind == "long_process" then
      local script = "sh -c \"trap '' TERM; while :; do sleep 1; done\" & echo $! > "
        .. snapshot.context.pid_file .. "; wait"
      local result = effects.process.run("sh", {"-c", script}, {
        timeout_ms=30000, max_output_bytes=1024,
      })
      roots.action("process_finished", result)
    elseif snapshot.event.kind == "long_timer" then
      effects.timer.sleep(10000)
      roots.action("timer_finished", {})
    elseif snapshot.event.kind == "quick" then
      effects.timer.sleep(1)
      roots.action("quick", {completed=true})
    end
  end,
})
"#;

fn package_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cleanup-effects.lua");
    std::fs::write(&path, PACKAGE).unwrap();
    (directory, path)
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn process_is_gone(pid: i32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    stat.is_empty() || stat.split_whitespace().nth(2) == Some("Z")
}

fn loaded_host(path: &std::path::Path) -> (Host, pi_rs_host::kernel::PackageHandle) {
    let host = Host::new(HostConfig::default()).unwrap();
    let handle = host.load_package(PackageSource::File { path }).unwrap();
    (host, handle)
}

fn request(kind: &str, context: serde_json::Value) -> DispatchRequest {
    DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({"kind":kind}),
        context,
    )
}

#[cfg(unix)]
#[test]
fn scope_disposal_kills_process_tree_and_settles_the_effect_queue() {
    let (directory, path) = package_file();
    let pid_file = directory.path().join("descendant.pid");
    let (host, handle) = loaded_host(&path);
    let caller = host.clone();
    let pid_path = pid_file.to_string_lossy().into_owned();
    let call = std::thread::spawn(move || {
        caller.dispatch(request(
            "long_process",
            serde_json::json!({"pid_file":pid_path}),
        ))
    });
    wait_until(|| pid_file.exists());
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(host.effect_stats().active > 0);

    host.dispose_package(&handle).unwrap();
    match call.join().unwrap() {
        Ok(batch) => assert_eq!(batch.actions[0].payload["killed"], true),
        Err(error) => assert!(matches!(error, HostError::Cancelled)),
    }
    wait_until(|| process_is_gone(pid));
    wait_until(|| host.effect_stats().active == 0);
    let stats = host.scope_stats(&handle).unwrap();
    assert!(stats.cancelled && stats.disposed);
    assert_eq!(stats.resources, 0);
}

#[test]
fn timer_is_cancelled_on_disposal_and_file_package_can_reload_cleanly() {
    let (_directory, path) = package_file();
    let (host, handle) = loaded_host(&path);
    let caller = host.clone();
    let started = Instant::now();
    let timer =
        std::thread::spawn(move || caller.dispatch(request("long_timer", serde_json::json!({}))));
    wait_until(|| host.effect_stats().active > 0);
    host.dispose_package(&handle).unwrap();
    assert!(matches!(timer.join().unwrap(), Err(HostError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(2));
    wait_until(|| host.effect_stats().active == 0);

    let reloaded = host
        .load_package(PackageSource::File { path: &path })
        .unwrap();
    let batch = host
        .dispatch(request("quick", serde_json::json!({})))
        .unwrap();
    assert_eq!(batch.actions[0].kind, "quick");
    assert_eq!(batch.actions[0].payload["completed"], true);
    host.dispose_package(&reloaded).unwrap();
    assert_eq!(host.effect_stats().active, 0);
}
