//! SP-TR ceremony over the pi-rs kernel `Component` system on the *real*
//! product paths, not a docs-only claim.
//!
//! **Temporal axis** (`snapshot → mount → exercise effects → unmount → diff`):
//! mount the real host VM + a live session manager together on the daemon
//! boundary's host-owned kernel `Context`, mount a TUI render scope on the TUI
//! lifecycle's kernel `Context`, exercise each mounted unit's real effects,
//! then unmount/drain every mount and diff the contexts against their
//! snapshots. An empty diff proves no residue.
//!
//! **Spatial axis** (`declared reader → resolved consumers`): republish a
//! declared reader's dependency input and confirm *exactly* its resolved
//! consumers react; an undeclared dependency change must react to nobody, and
//! a viewer that declared a *different* key must not react — a missed consumer
//! or undeclared dependency fails the ceremony.
//!
//! Runnable: `cargo test -p pi-rs-app --test spatiotemporal_ceremony`. An
//! SP-TR claim without an executable ceremony fails (DESIGN "Spatiotemporal
//! composability (2026-08-07)" row).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pi_rs_host::daemon::{DaemonBoundary, KEY_HOST_VM, KEY_SESSION};
use pi_rs_host::{Host, HostConfig};
use pi_rs_session::SessionManager;
use pi_rs_tui::component::Text;
use pi_rs_tui::lifecycle::{ScopedComponent, TuiHost};

const THEME: &str = "ceremony:theme";

fn real_host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

fn session_manager() -> Arc<Mutex<SessionManager>> {
    Arc::new(Mutex::new(SessionManager::in_memory()))
}

fn render(text: &str) -> Arc<dyn pi_rs_tui::component::Component> {
    Arc::new(Text::new(text, 1, 1))
}

/// **Temporal ceremony over the real host path (daemon boundary context).**
///
/// Snapshot an empty kernel `Context`; mount the *real* host VM (its Lua
/// runtime/thread is committed under [`KEY_HOST_VM`], torn down by the unmount
/// inverse) and the real session manager (under [`KEY_SESSION`]); exercise the
/// VM and the session while live; unmount both; diff — the context must return
/// to its empty pre-mount state with no residue.
#[test]
fn temporal_host_mount_exercise_unmount_diff_leaves_no_residue() {
    let mut boundary = DaemonBoundary::new();
    assert!(boundary.is_empty(), "pre-mount must be empty");

    // Mount the real host VM (process/session host).
    let host = real_host();
    let host_scope = boundary.mount_host(&host);
    assert!(boundary.has(KEY_HOST_VM), "host liveness not committed");

    // Exercise every mounted effect: load + emit through a live chunk.
    host.load(
        "ceremony://ping",
        "local pi=... pi.on('ping', function() return { pong = 'yes' } end)",
    )
    .unwrap();
    let outcome = host.emit("ping", &serde_json::json!({})).unwrap();
    assert_eq!(
        outcome[0].result.as_ref().unwrap().as_ref().unwrap()["pong"],
        serde_json::json!("yes")
    );

    // Mount the real session manager.
    let manager = session_manager();
    let session_scope = boundary.mount_session(manager.clone());
    assert!(boundary.has(KEY_SESSION), "session handle not committed");
    manager
        .lock()
        .unwrap()
        .append_message(serde_json::json!({
            "role": "user", "content": "hi", "timestamp": pi_rs_session::time::now_ms()
        }))
        .unwrap();

    // Unmount in reverse registration order; diff the context.
    boundary.unmount(session_scope);
    boundary.unmount(host_scope);

    assert!(boundary.is_empty(), "residue: scope still mounted");
    assert!(!boundary.has(KEY_HOST_VM), "residue: host liveness key survives");
    assert!(!boundary.has(KEY_SESSION), "residue: session handle survives");

    // The VM thread was torn down by the unmount inverse: a further call fails.
    assert!(matches!(
        host.emit("ping", &serde_json::json!({})),
        Err(pi_rs_host::HostError::VmUnavailable)
    ));
}

/// **Temporal ceremony over the real TUI render path (TuiHost context).**
///
/// Snapshot a TuiHost kernel `Context`; mount a render-scoped component that
/// commits editor state under the shared editor key; exercise the render;
/// unmount; diff — the context returns to its pre-mount state (no editor
/// residue, pre-existing theme preserved).
#[test]
fn temporal_tui_render_mount_unmount_diff_leaves_no_residue() {
    let mut tui = TuiHost::new();
    tui.set(THEME, "dark");
    assert!(tui.read::<&str>(THEME).is_some());

    let id = tui.mount(
        ScopedComponent::new(vec![THEME], render("prompt")).editor_state("idle"),
    );
    assert_eq!(
        tui.read::<String>(pi_rs_tui::lifecycle::KEY_EDITOR).map(String::as_str),
        Some("idle"),
        "editor state committed on mount"
    );

    // Exercise the render while mounted.
    let _cells = tui.render(40);
    assert!(!_cells.is_empty(), "render produced cells while mounted");

    tui.unmount(id);

    assert!(
        tui.read::<String>(pi_rs_tui::lifecycle::KEY_EDITOR).is_none(),
        "residue: editor state survives unmount"
    );
    assert_eq!(
        tui.read::<&str>(THEME).copied(),
        Some("dark"),
        "pre-existing theme must survive"
    );
}

/// **Spatial ceremony over the real host path.**
///
/// Two client viewers attach over the same kernel `Context`: viewer A declares
/// it reads [`KEY_SESSION`]; viewer B declares it reads only `session:leaf`.
/// Republish the resolved session handle on [`KEY_SESSION`] and assert exactly
/// A reacts; a commit on an undeclared key must react to nobody.
#[test]
fn spatial_set_notifies_exactly_the_declared_readers() {
    use pi_rs_session::composable::KEY_LEAF;

    let mut boundary = DaemonBoundary::new();
    let host = real_host();
    let host_scope = boundary.mount_host(&host);
    let manager = session_manager();
    let session_scope = boundary.mount_session(manager.clone());

    let session_hits = Arc::new(AtomicUsize::new(0));
    let leaf_hits = Arc::new(AtomicUsize::new(0));

    let a = session_hits.clone();
    boundary.attach_with(vec![KEY_SESSION], move |k| {
        assert_eq!(k, KEY_SESSION, "session viewer got non-session key");
        a.fetch_add(1, Ordering::Relaxed);
    });
    let b = leaf_hits.clone();
    boundary.attach_with(vec![KEY_LEAF], move |k| {
        assert_eq!(k, KEY_LEAF, "leaf viewer got non-leaf key");
        b.fetch_add(1, Ordering::Relaxed);
    });

    // Change one declared reader's dependency input: republish the session.
    boundary.set(KEY_SESSION, manager.clone());
    assert_eq!(
        session_hits.load(Ordering::Relaxed),
        1,
        "session viewer must react to the session set"
    );
    assert_eq!(
        leaf_hits.load(Ordering::Relaxed),
        0,
        "leaf viewer reacted to a session change (missed-consumer fail)"
    );

    // An undeclared dependency change reacts to nobody.
    boundary.set("ceremony.undeclared", 1u8);
    assert_eq!(session_hits.load(Ordering::Relaxed), 1);
    assert_eq!(leaf_hits.load(Ordering::Relaxed), 0);

    boundary.unmount(session_scope);
    boundary.unmount(host_scope);
}