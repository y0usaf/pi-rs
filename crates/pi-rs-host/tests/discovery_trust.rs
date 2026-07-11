//! WS1.4 acceptance: discovery + trust gate.
//! - discovery matches loader.ts rules mapped to Lua: direct `.lua`
//!   files, subdir `init.lua`, one level deep, project → global →
//!   configured order, dedup, trust gate on the project dir
//! - `Host::load_extensions` collects per-path errors without aborting
//! - trust store matches trust-manager.ts: nearest-ancestor lookup,
//!   `set_many` with deletion, sorted `trust.json`
//! - trust options and prompt text match the spec's shapes
//! - `resolve_project_trusted` decision order matches project-trust.ts
//! - the `project_trust` event exerciser answers through the public API

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use pi_rs_host::trust::{
    ProjectTrustStore, ResolveProjectTrust, TrustResolution, TrustUpdate, has_project_trust_inputs,
    project_trust_options, resolve_project_trusted, save_trust_option, trust_event_result,
};
use pi_rs_host::{Host, HostConfig, discover};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pi-rs-trust-test-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Layout: project `.pi/extensions` (file + init.lua subdir + ignored
/// entries), global `agent/extensions`, a configured directory without
/// entry points, and a configured single file.
fn discovery_fixture() -> (PathBuf, PathBuf, PathBuf) {
    let root = temp_dir("discover");
    let proj = root.join("proj");
    let agent = root.join("agent");

    let ext = proj.join(".pi").join("extensions");
    write(&ext.join("alpha.lua"), "local pi = ...");
    write(&ext.join("pack").join("init.lua"), "local pi = ...");
    write(&ext.join("note.txt"), "not an extension");
    // Subdir without init.lua: skipped (no recursion, no entry point).
    write(&ext.join("no-entry").join("inner.lua"), "local pi = ...");

    write(
        &agent.join("extensions").join("global.lua"),
        "local pi = ...",
    );

    (root, proj, agent)
}

#[test]
fn discovery_orders_project_then_global_then_configured() {
    let (root, proj, agent) = discovery_fixture();
    let extra = root.join("extra");
    write(&extra.join("x.lua"), "local pi = ...");
    let single = root.join("single.lua");
    write(&single, "local pi = ...");

    let ext = proj.join(".pi").join("extensions");
    let found =
        discover::discover_extension_paths(&[s(&extra), s(&single)], &s(&proj), &s(&agent), true);
    assert_eq!(
        found,
        vec![
            s(&ext.join("alpha.lua")),
            s(&ext.join("pack").join("init.lua")),
            s(&agent.join("extensions").join("global.lua")),
            s(&extra.join("x.lua")),
            s(&single),
        ]
    );
}

#[test]
fn discovery_untrusted_skips_project_local() {
    let (_root, proj, agent) = discovery_fixture();
    let found = discover::discover_extension_paths(&[], &s(&proj), &s(&agent), false);
    assert_eq!(found, vec![s(&agent.join("extensions").join("global.lua"))]);
}

#[test]
fn discovery_dedupes_on_resolved_path() {
    let (_root, proj, agent) = discovery_fixture();
    let alpha = proj.join(".pi").join("extensions").join("alpha.lua");
    // Configuring an already-discovered path keeps the first occurrence.
    let found = discover::discover_extension_paths(&[s(&alpha)], &s(&proj), &s(&agent), true);
    assert_eq!(found.iter().filter(|p| **p == s(&alpha)).count(), 1);
}

#[test]
fn discovery_configured_dir_prefers_entry_points() {
    let (root, proj, agent) = discovery_fixture();
    // A configured directory with init.lua loads only the entry point.
    let packdir = root.join("packdir");
    write(&packdir.join("init.lua"), "local pi = ...");
    write(&packdir.join("other.lua"), "local pi = ...");

    let found = discover::discover_extension_paths(&[s(&packdir)], &s(&proj), &s(&agent), false);
    assert_eq!(
        found,
        vec![
            s(&agent.join("extensions").join("global.lua")),
            s(&packdir.join("init.lua")),
        ]
    );
}

#[test]
fn discovery_missing_dirs_yield_nothing() {
    let root = temp_dir("discover-empty");
    let found = discover::discover_extension_paths(
        &[],
        &s(&root.join("nope")),
        &s(&root.join("agent-nope")),
        true,
    );
    assert!(found.is_empty());
}

#[test]
fn load_extensions_collects_errors_without_aborting() {
    let root = temp_dir("load");
    let good = root.join("good.lua");
    write(&good, "local pi = ...\npi.on(\"x\", function() end)");
    let bad = root.join("bad.lua");
    write(&bad, "this is not lua ((");
    let missing = root.join("missing.lua");

    let host = Host::new(HostConfig::default()).expect("host starts");
    let report = host.load_extensions(&[s(&good), s(&bad), s(&missing)]);
    assert_eq!(report.loaded, vec![s(&good)]);
    assert_eq!(report.errors.len(), 2);
    assert_eq!(report.errors[0].path, s(&bad));
    assert!(
        report.errors[0]
            .error
            .starts_with("Failed to load extension:")
    );
    assert_eq!(report.errors[1].path, s(&missing));
}

// ---------------------------------------------------------------------------
// Trust store
// ---------------------------------------------------------------------------

#[test]
fn trust_store_nearest_ancestor_lookup() {
    let root = temp_dir("store");
    let agent = root.join("agent");
    let proj = root.join("work").join("proj");
    let sub = proj.join("deep").join("er");
    std::fs::create_dir_all(&sub).unwrap();

    let store = ProjectTrustStore::new(&s(&agent));
    assert_eq!(store.get(&s(&sub)).unwrap(), None);

    store.set(&s(&proj), Some(true)).unwrap();
    // Decision at proj is found from a descendant, attributed to proj.
    let entry = store.get_entry(&s(&sub)).unwrap().unwrap();
    assert!(entry.decision);
    assert_eq!(entry.path, s(&proj.canonicalize().unwrap()));

    // A closer decision shadows the ancestor.
    store.set(&s(&sub), Some(false)).unwrap();
    assert_eq!(store.get(&s(&sub)).unwrap(), Some(false));
    assert_eq!(store.get(&s(&proj)).unwrap(), Some(true));

    // Deletion falls back to the ancestor.
    store.set(&s(&sub), None).unwrap();
    assert_eq!(store.get(&s(&sub)).unwrap(), Some(true));
}

#[test]
fn trust_store_writes_sorted_json() {
    let root = temp_dir("store-json");
    let agent = root.join("agent");
    let b = root.join("b");
    let a = root.join("a");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();

    let store = ProjectTrustStore::new(&s(&agent));
    store
        .set_many(&[
            TrustUpdate {
                path: s(&b),
                decision: Some(true),
            },
            TrustUpdate {
                path: s(&a),
                decision: Some(false),
            },
        ])
        .unwrap();

    let content = std::fs::read_to_string(agent.join("trust.json")).unwrap();
    assert!(content.ends_with('\n'));
    let a_pos = content.find(&s(&a.canonicalize().unwrap())).unwrap();
    let b_pos = content.find(&s(&b.canonicalize().unwrap())).unwrap();
    assert!(a_pos < b_pos, "keys must be sorted");
}

#[test]
fn trust_store_rejects_invalid_file() {
    let root = temp_dir("store-invalid");
    let agent = root.join("agent");
    write(&agent.join("trust.json"), "{\"x\": \"yes\"}");
    let store = ProjectTrustStore::new(&s(&agent));
    assert!(store.get(&s(&root)).is_err());
}

// ---------------------------------------------------------------------------
// Options + inputs
// ---------------------------------------------------------------------------

#[test]
fn trust_options_match_spec_shape() {
    let root = temp_dir("options");
    let proj = root.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let canonical = s(&proj.canonicalize().unwrap());
    let parent = s(&proj.parent().unwrap().canonicalize().unwrap());

    let options = pi_rs_host::trust::project_trust_options(&s(&proj), true);
    let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "Trust",
            format!("Trust parent folder ({parent})").as_str(),
            "Trust (this session only)",
            "Do not trust",
            "Do not trust (this session only)",
        ]
    );
    // Parent option trusts the parent and clears the child entry.
    assert_eq!(
        options[1].updates,
        vec![
            TrustUpdate {
                path: parent.clone(),
                decision: Some(true)
            },
            TrustUpdate {
                path: canonical.clone(),
                decision: None
            },
        ]
    );
    // Session-only options carry no updates.
    assert!(options[2].updates.is_empty());
    assert!(options[4].updates.is_empty());
    assert!(!options[3].trusted);

    // Without session-only: Trust / Trust parent / Do not trust.
    assert_eq!(project_trust_options(&s(&proj), false).len(), 3);
}

#[test]
fn trust_inputs_detected_from_config_dir_and_agents_skills() {
    let root = temp_dir("inputs");
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(!has_project_trust_inputs(&s(&plain)));

    let with_config = root.join("with-config");
    std::fs::create_dir_all(with_config.join(".pi")).unwrap();
    assert!(has_project_trust_inputs(&s(&with_config)));

    // .agents/skills in an ancestor counts.
    let skills_root = root.join("skills-root");
    std::fs::create_dir_all(skills_root.join(".agents").join("skills")).unwrap();
    let nested = skills_root.join("deep").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(has_project_trust_inputs(&s(&nested)));
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

fn resolve_fixture() -> (PathBuf, ProjectTrustStore, PathBuf) {
    let root = temp_dir("resolve");
    let agent = root.join("agent");
    let proj = root.join("proj");
    std::fs::create_dir_all(proj.join(".pi")).unwrap();
    (root, ProjectTrustStore::new(&s(&agent)), proj)
}

fn resolve(
    store: &ProjectTrustStore,
    cwd: &str,
    trust_override: Option<bool>,
    default: Option<&str>,
    extension_result: Option<pi_rs_host::trust::TrustEventResult>,
) -> TrustResolution {
    resolve_project_trusted(&ResolveProjectTrust {
        cwd,
        store,
        trust_override,
        default_project_trust: default,
        extension_result,
    })
    .unwrap()
}

#[test]
fn resolution_order_matches_spec() {
    let (root, store, proj) = resolve_fixture();
    let proj_s = s(&proj);

    // Override wins over everything.
    assert_eq!(
        resolve(&store, &proj_s, Some(false), Some("always"), None),
        TrustResolution::Decided(false)
    );

    // No trust inputs → trivially trusted.
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(
        resolve(&store, &s(&plain), None, Some("never"), None),
        TrustResolution::Decided(true)
    );

    // Extension answer wins over store and default; remember persists.
    assert_eq!(
        resolve(
            &store,
            &proj_s,
            None,
            Some("never"),
            Some(pi_rs_host::trust::TrustEventResult {
                trusted: true,
                remember: true,
            }),
        ),
        TrustResolution::Decided(true)
    );
    assert_eq!(store.get(&proj_s).unwrap(), Some(true));

    // Stored decision wins over default.
    assert_eq!(
        resolve(&store, &proj_s, None, Some("never"), None),
        TrustResolution::Decided(true)
    );

    // Default applies when nothing is stored.
    store.set(&proj_s, None).unwrap();
    assert_eq!(
        resolve(&store, &proj_s, None, Some("always"), None),
        TrustResolution::Decided(true)
    );
    assert_eq!(
        resolve(&store, &proj_s, None, Some("never"), None),
        TrustResolution::Decided(false)
    );

    // "ask" (and None) fall through to the frontend.
    assert_eq!(
        resolve(&store, &proj_s, None, Some("ask"), None),
        TrustResolution::Ask
    );
    assert_eq!(
        resolve(&store, &proj_s, None, None, None),
        TrustResolution::Ask
    );
}

#[test]
fn saving_a_prompted_option_persists_updates() {
    let (_root, store, proj) = resolve_fixture();
    let options = project_trust_options(&s(&proj), true);

    // Session-only writes nothing.
    save_trust_option(&store, &options[2]).unwrap();
    assert_eq!(store.get(&s(&proj)).unwrap(), None);

    // "Do not trust" persists false.
    save_trust_option(&store, &options[3]).unwrap();
    assert_eq!(store.get(&s(&proj)).unwrap(), Some(false));
}

// ---------------------------------------------------------------------------
// project_trust event through the public API
// ---------------------------------------------------------------------------

#[test]
fn project_trust_exerciser_answers_through_emit() {
    let root = temp_dir("event");
    let marked = root.join("marked");
    write(&marked.join(".pi-trust"), "");
    let unmarked = root.join("unmarked");
    std::fs::create_dir_all(&unmarked).unwrap();

    let host = Host::new(HostConfig::default()).expect("host starts");
    host.load_file("../../examples/extensions/project-trust.lua")
        .expect("exerciser loads");

    // Marked project → first handler answers yes.
    let outcomes = host
        .emit(
            "project_trust",
            &serde_json::json!({ "type": "project_trust", "cwd": s(&marked) }),
        )
        .unwrap();
    let (result, errors) = trust_event_result(&outcomes);
    assert!(errors.is_empty());
    let result = result.unwrap();
    assert!(result.trusted);
    assert!(!result.remember);

    // Unmarked project → undecided falls through.
    let outcomes = host
        .emit(
            "project_trust",
            &serde_json::json!({ "type": "project_trust", "cwd": s(&unmarked) }),
        )
        .unwrap();
    let (result, errors) = trust_event_result(&outcomes);
    assert!(errors.is_empty());
    assert!(result.is_none());
}

#[test]
fn trust_event_result_maps_answers_like_the_spec() {
    let ok = |v: serde_json::Value| pi_rs_host::Outcome {
        source: "ext".to_owned(),
        result: Ok(Some(v)),
    };

    // "no" answers decide false.
    let (result, _) = trust_event_result(&[ok(serde_json::json!({ "trusted": "no" }))]);
    assert!(!result.unwrap().trusted);

    // A non-"undecided", non-"yes" value decides false (spec:
    // `result.trusted === "yes"`).
    let (result, _) = trust_event_result(&[ok(serde_json::json!({ "trusted": "maybe" }))]);
    assert!(!result.unwrap().trusted);

    // Failing handlers are collected and fall through.
    let failing = pi_rs_host::Outcome {
        source: "bad-ext".to_owned(),
        result: Err("boom".to_owned()),
    };
    let (result, errors) = trust_event_result(&[
        failing,
        ok(serde_json::json!({ "trusted": "yes", "remember": true })),
    ]);
    assert_eq!(errors, vec![("bad-ext".to_owned(), "boom".to_owned())]);
    let result = result.unwrap();
    assert!(result.trusted);
    assert!(result.remember);
}
