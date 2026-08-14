//! App-declance composability tests (axioms 05/06, fleet contract gates 2–4):
//! temporal (mount/unmount residue-free), spatial (declared readers only),
//! one-declaration-path (axiom 05), generation/epoch-lifetime dependent reload,
//! and atomic reload. Exercises the [`pi_rs_app::decl`] registry backed by the
//! pi-rs kernel.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_app::decl::{Definition, Epoch, Kind, Phase, Provider, Registry};

const APP_KEY: &str = "app.frontend";
const CMD_KEY: &str = "command.slash";
const TOOL_KEY: &str = "tool.shell";
const RES_KEY: &str = "resource.theme";
const TARGET_KEY: &str = "target.session";

fn p(key: &'static str, version: u64) -> Provider {
    Provider {
        key,
        provider_id: "declared-provider",
        version,
        schema_hash: "schema-v1",
    }
}

fn epoch(key: &'static str, version: u64) -> Epoch {
    Epoch {
        key,
        provider_id: "declared-provider",
        version,
        schema_hash: "schema-v1",
    }
}

/// A declaration for each kind through the *single* public path.
fn declare_all(reg: &mut Registry) {
    for def in [
        Definition::new(Kind::App, "app-unit", p(APP_KEY, 1)).reads(vec![TARGET_KEY]),
        Definition::new(Kind::Command, "cmd-unit", p(CMD_KEY, 1)),
        Definition::new(Kind::Tool, "tool-unit", p(TOOL_KEY, 1)),
        Definition::new(Kind::Resource, "res-unit", p(RES_KEY, 1)),
    ] {
        reg.declare(def).unwrap();
    }
}

/// Acceptance 2 — temporal axis: register all kinds, activate every unit,
/// then drain every unit. The kernel context must return to its pre-mount state
/// (no residue): no provider key remains.
#[test]
fn temporal_mount_unmount_leaves_no_residue() {
    let mut reg = Registry::new();
    declare_all(&mut reg);

    // No provider published before mount.
    for key in [APP_KEY, CMD_KEY, TOOL_KEY, RES_KEY] {
        assert!(!reg.has(key), "pre-mount residue for {key}");
    }

    for (name, key) in [
        ("app-unit", APP_KEY),
        ("cmd-unit", CMD_KEY),
        ("tool-unit", TOOL_KEY),
        ("res-unit", RES_KEY),
    ] {
        reg.activate(name).unwrap();
        assert_eq!(reg.phase(name), Some(Phase::Active));
        assert!(reg.has(key), "provider {key} not published on mount");
    }

    // Unmount in reverse registration order; each inverse replays residue-free.
    for name in ["res-unit", "tool-unit", "cmd-unit", "app-unit"] {
        reg.drain(name).unwrap();
        assert_eq!(reg.phase(name), Some(Phase::Inactive));
    }

    // Diff is empty: no provider value survives.
    for key in [APP_KEY, CMD_KEY, TOOL_KEY, RES_KEY] {
        assert!(!reg.has(key), "residue after unmount for {key}");
    }
}

/// Acceptance 3 — spatial axis: only the unit that declared a read key reloads
/// when that key changes; undeclared key changes react to nobody.
#[test]
fn spatial_notifies_only_declared_readers() {
    let mut reg = Registry::new();
    declare_all(&mut reg);
    // Only `app-unit` reads TARGET_KEY.
    for name in ["app-unit", "cmd-unit", "tool-unit", "res-unit"] {
        reg.activate(name).unwrap();
    }

    let app_start = reg.generation("app-unit");
    let cmd_start = reg.generation("cmd-unit");
    let tool_start = reg.generation("tool-unit");
    let res_start = reg.generation("res-unit");

    // Change the declared key (TARGET_KEY). Only app-unit reloads.
    reg.set(TARGET_KEY, epoch(TARGET_KEY, 2));
    assert_eq!(
        reg.generation("app-unit"),
        app_start + 1,
        "app-unit should reload"
    );
    assert_eq!(
        reg.generation("cmd-unit"),
        cmd_start,
        "cmd-unit must not reload"
    );
    assert_eq!(
        reg.generation("tool-unit"),
        tool_start,
        "tool-unit must not reload"
    );
    assert_eq!(
        reg.generation("res-unit"),
        res_start,
        "res-unit must not reload"
    );

    // An undeclared key: nobody reacts.
    let app_before = reg.generation("app-unit");
    reg.set(RES_KEY, epoch(RES_KEY, 9));
    assert_eq!(
        reg.generation("app-unit"),
        app_before,
        "undeclared key reloaded a reader"
    );
    assert_eq!(reg.generation("cmd-unit"), cmd_start);
    assert_eq!(reg.generation("tool-unit"), tool_start);
    assert_eq!(reg.generation("res-unit"), res_start);
}

/// Axiom 05 — one declaration path. Every unit of a kind (app, command, tool,
/// resource) registers through `Registry::declare`; the shipped frontends are
/// returned as declared data consumed uniformly, and there is no per-kind
/// authoring/launcher branch. Declaring duplicate names fails, and all kinds
/// travel the same declare() call.
#[test]
fn one_declaration_path_for_all_kinds() {
    let mut reg = Registry::new();
    declare_all(&mut reg);

    // All four kinds registered through one public path, none rejected.
    let units = reg.units();
    assert!(units.contains(&("app-unit", Kind::App)));
    assert!(units.contains(&("cmd-unit", Kind::Command)));
    assert!(units.contains(&("tool-unit", Kind::Tool)));
    assert!(units.contains(&("res-unit", Kind::Resource)));

    // Duplicate name through the same path is rejected (single registry).
    let dup = Definition::new(Kind::Tool, "tool-unit", p(TOOL_KEY, 1));
    assert!(reg.declare(dup).is_err());

    // The shipped frontends are declared apps through the same single path:
    // `declare_shipped` goes through `Registry::declare` (no separate
    // hardcoded launcher table), and selection reads declared metadata — the
    // launcher's `interactive ? ... : ...` branch is gone.
    let mut shipped = Registry::new();
    let names = shipped.declare_shipped();
    assert_eq!(names, vec!["print-application", "interactive-frontend"]);
    assert_eq!(shipped.select_frontend(false), "print");
    assert_eq!(shipped.select_frontend(true), "interactive");
    // The shipped apps are Kind::App units registered through the registry.
    assert!(shipped.units().contains(&("print-application", Kind::App)));
    assert!(
        shipped
            .units()
            .contains(&("interactive-frontend", Kind::App))
    );
}

/// Generation / epoch lifecycle. Only an epoch change forces a dependent
/// reload: an identical re-set of the same epoch is a no-op, a new epoch
/// reloads exactly once.
///
/// The dependent unit's read key (`RES_KEY`) is republished by the provider
/// unit itself. Ensure the dependent's recorded baseline only lets an *actual
/// epoch change* through.
#[test]
fn dependent_reloads_only_on_epoch_change() {
    let mut reg = Registry::new();
    reg.declare(Definition::new(Kind::Resource, "provider-unit", p(RES_KEY, 1)).reads(vec![]))
        .unwrap();
    reg.declare(Definition::new(Kind::App, "dependent-unit", p(APP_KEY, 1)).reads(vec![RES_KEY]))
        .unwrap();

    reg.activate("provider-unit").unwrap();
    reg.activate("dependent-unit").unwrap();

    let dep_start = reg.generation("dependent-unit");

    // Re-assert the SAME epoch → no reload (only an epoch change reloads).
    reg.set(RES_KEY, epoch(RES_KEY, 1));
    assert_eq!(
        reg.generation("dependent-unit"),
        dep_start,
        "same epoch reloaded"
    );

    // New epoch → exactly one reload.
    reg.set(RES_KEY, epoch(RES_KEY, 2));
    assert_eq!(
        reg.generation("dependent-unit"),
        dep_start + 1,
        "new epoch did not reload"
    );

    // The provider unit itself never reads RES_KEY, so it never reloads.
    let prov_start = reg.generation("provider-unit");
    reg.set(RES_KEY, epoch(RES_KEY, 3));
    assert_eq!(
        reg.generation("provider-unit"),
        prov_start,
        "non-reader reloaded"
    );

    // Lifecycle phases walk Inactive → Preparing → Active → Draining → Inactive.
    assert_eq!(reg.phase("dependent-unit"), Some(Phase::Active));
    reg.drain("dependent-unit").unwrap();
    assert_eq!(reg.phase("dependent-unit"), Some(Phase::Inactive));

    // Deprecated placeholder kept to keep this test self-contained:
    // drain leaves no residue.
    assert!(!reg.has(APP_KEY));
}

/// Atomic reload — a failing preflight (build) leaves the published runtime
/// untouched: the unit keeps its prior generation and provider value.
#[test]
fn atomic_reload_build_failure_leaves_old_runtime() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let fail_build = std::sync::Arc::new(AtomicBool::new(false));

    let mut reg = Registry::new();
    let fail = fail_build.clone();
    reg.declare(
        Definition::new(Kind::App, "atomic-unit", p(APP_KEY, 1))
            .preflight(move || {
                if fail.load(Ordering::SeqCst) {
                    Err("preflight refused".to_owned())
                } else {
                    Ok(())
                }
            })
            .reads(vec![TARGET_KEY]),
    )
    .unwrap();
    reg.activate("atomic-unit").unwrap();
    let start = reg.generation("atomic-unit");

    // Fail the build, then trigger a reload via a read-key change.
    fail_build.store(true, Ordering::SeqCst);
    reg.set(TARGET_KEY, epoch(TARGET_KEY, 2));
    fail_build.store(false, Ordering::SeqCst);

    // The published generation is untouched: same generation, provider value
    // unchanged, unit still active.
    assert_eq!(
        reg.generation("atomic-unit"),
        start,
        "failed build mutated published runtime"
    );
    assert!(reg.has(APP_KEY), "failed build dropped the provider value");
    assert_eq!(reg.phase("atomic-unit"), Some(Phase::Active));

    // Once the build is fixed, the next change reloads atomically (build →
    // activate → publish → drain) with one generation bump.
    reg.set(TARGET_KEY, epoch(TARGET_KEY, 3));
    assert_eq!(
        reg.generation("atomic-unit"),
        start + 1,
        "reload after fix failed"
    );
    assert!(reg.has(APP_KEY));
}

/// Drain defers while a [`Lease`] (in-flight work) is held, then completes
/// residue-free once released.
#[test]
fn drain_defers_until_in_flight_leases_release() {
    let mut reg = Registry::new();
    reg.declare(Definition::new(Kind::App, "lease-unit", p(APP_KEY, 1)))
        .unwrap();
    reg.activate("lease-unit").unwrap();
    assert!(reg.has(APP_KEY));

    let lease = reg.acquire("lease-unit").expect("active unit leases");
    assert!(
        reg.drain("lease-unit").is_err(),
        "drain succeeded with in-flight work"
    );
    assert_eq!(reg.phase("lease-unit"), Some(Phase::Active));
    assert!(reg.has(APP_KEY));

    drop(lease);
    reg.drain("lease-unit").unwrap();
    assert_eq!(reg.phase("lease-unit"), Some(Phase::Inactive));
    assert!(!reg.has(APP_KEY), "residue after drain");
}
