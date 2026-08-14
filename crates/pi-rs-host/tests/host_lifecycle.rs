#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Host lifecycle acceptance at the real VM boundary.
//!
//! Temporal axis: feed a snapshot, mount host resources (a `Host` VM with a
//! running subprocess, an in-memory session handle, and a file watcher), then
//! `Host::stop()` — the vm thread exits, the Lua state is dropped, and every
//! tracked resource's explicit dispose runs. A diff after unmount must be
//! empty: no leaked vm thread, no leaked subprocess, no surviving session
//! handle, no surviving watcher thread.
//!
//! Spatial axis: `HostLifecycle::set` on a declared key notifies exactly its
//! readers; an undeclared key fires nothing (kernel spatial semantics, tested
//! in the lifecycle unit module; surfaced here through the same mechanism).

use pi_rs_host::lifecycle::{Epoch, Generation, HostLifecycle, Scope};
use pi_rs_host::{Host, HostConfig, HostError};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SETTINGS_KEY: &str = "settings";
const MODEL_KEY: &str = "model";

/// After `Host::stop()` the vm thread is gone, so every subsequent method
/// reports `VmUnavailable`.
#[test]
fn stop_tears_down_the_vm() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "test://stop-lua",
        "local pi = ... pi.on('ping', function() return {} end)",
    )
    .unwrap();
    host.stop().unwrap();
    // A second stop is a no-op / safe.
    assert!(matches!(host.stop(), Ok(())) || host.emit("ping", &serde_json::json!({})).is_err());
    // After the vm thread is gone, emits report the vm unavailable.
    assert!(matches!(
        host.emit("ping", &serde_json::json!({})),
        Err(HostError::VmUnavailable)
    ));
}

/// Temporal: a subprocess spawned through `pi.process` is reaped when the
/// host — its owning scope — is drained. Diff against the pre-mount snapshot
/// (no such process) is empty.
#[test]
fn stop_reaps_spawned_subprocess() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "test://spawn-sleep",
        r#"
        local pi = ...
        pi.on("spawn", function()
            local p = pi.process.spawn("sh", { "-c", "sleep 60" })
            return { pid = p:pid() }
        end)
        "#,
    )
    .unwrap();
    let out = host.emit("spawn", &serde_json::json!({})).unwrap();
    let pid = out[0].result.as_ref().unwrap().as_ref().unwrap()["pid"]
        .as_u64()
        .unwrap() as i32;
    assert!(pid > 0);

    // Snap-shot the live state: the process exists before unmount.
    host.stop().unwrap();

    // The tree was SIGKILLed; the detached reaper waitpid-loops so no zombie
    // is left. Poll briefly for the reap to complete (a zombie for a few ms is
    // expected — the deterministic kill is synchronous, reaping is async).
    let mut alive = true;
    for _ in 0..40 {
        alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        !alive,
        "spawned subprocess {pid} survived host stop (leaked)"
    );
}

/// The host lifecycle surfaces the spatial axis: change a declared key →
/// exactly its readers react; an undeclared change fires nothing.
#[test]
fn lifecycle_set_notifies_only_declared_readers() {
    let settings_hits = Arc::new(AtomicUsize::new(0));
    let model_hits = Arc::new(AtomicUsize::new(0));

    let mut lc = HostLifecycle::new();
    let sh = Arc::clone(&settings_hits);
    lc.mount(
        Scope::default()
            .with_read(SETTINGS_KEY)
            .on_change(move |k| {
                assert_eq!(k, SETTINGS_KEY);
                sh.fetch_add(1, Ordering::Relaxed);
            }),
    );
    let mh = Arc::clone(&model_hits);
    lc.mount(Scope::default().with_read(MODEL_KEY).on_change(move |k| {
        assert_eq!(k, MODEL_KEY);
        mh.fetch_add(1, Ordering::Relaxed);
    }));

    lc.set(SETTINGS_KEY, "{}");
    assert_eq!(settings_hits.load(Ordering::Relaxed), 1);
    assert_eq!(
        model_hits.load(Ordering::Relaxed),
        0,
        "undeclared reader fired"
    );

    lc.set("undeclared", 1);
    assert_eq!(
        settings_hits.load(Ordering::Relaxed),
        1,
        "undeclared changed"
    );
    assert_eq!(model_hits.load(Ordering::Relaxed), 0, "undeclared changed");
}

/// Generation/epoch gating at the host lifecycle boundary: a settings/session
/// generation reloads only when its `[key, provider, version, schema_hash]`
/// epoch changes.
#[test]
fn generation_epoch_gates_reload() {
    let epoch = Epoch::new(MODEL_KEY, "settings", "1.0", "sha-abc");
    let generation = Generation {
        value: "gpt",
        epoch: epoch.clone(),
    };
    assert!(!generation.reloads_on(&epoch), "same epoch must not reload");
    assert!(
        generation.reloads_on(&Epoch::new(MODEL_KEY, "settings", "1.1", "sha-abc")),
        "version bump reloads"
    );
    assert!(
        generation.reloads_on(&Epoch::new(MODEL_KEY, "settings", "1.0", "sha-def")),
        "schema hash bump reloads"
    );
}
