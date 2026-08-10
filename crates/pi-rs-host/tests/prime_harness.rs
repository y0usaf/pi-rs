//! P4 continual harness — Lua policy over the P2 record layer
//! (docs/prime-agent-plan.md). Loads `prime/harness.lua` through the public
//! loader exactly the way `prime_rlm.rs` loads `prime/rlm.lua` and drives it
//! through the host command/role dispatch path — the same seam a normal
//! file-backed package uses (no privileged/builtin path).
//!
//! Proven here:
//!   * the package registers the `/refine` command and its fixture roles;
//!   * `/refine` does validated CRUD with rollback and record_refinement;
//!   * the prompt projection is a pure function — same input always yields
//!     the same output, and local entries win over global (precedence);
//!   * harness records written to a fresh session survive a host restart and
//!     all three of memory/skill/prompt appear in the injected overview
//!     (the P4 fixture);
//!   * with the harness package absent, the RLM package still boots with an
//!     empty harness (no `/refine` command, no harness roles).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicU64, Ordering};

use pi_rs_host::{Host, HostConfig};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn host() -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 90_000,
        cwd: None,
        project_trusted: true,
    })
    .expect("host")
}

fn load_harness(host: &Host) {
    host.load("prime/harness.lua", include_str!("../../../prime/harness.lua"))
        .expect("prime harness package loads through the public loader");
}

#[allow(clippy::all)]
fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir()
        .join(format!("prime-harness-{}-{}-{}", tag, std::process::id(), seq));
    std::fs::create_dir_all(&dir).expect("temp session dir");
    dir
}

#[test]
fn harness_package_registers_refine_and_roles_through_public_loader() {
    let host = host();
    load_harness(&host);

    let commands = host.commands().expect("commands mirror");
    assert!(
        commands.iter().any(|c| c.name == "refine"),
        "harness must register the /refine command: {commands:?}"
    );

    let roles = host.roles().expect("roles");
    let role_names: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();
    for expected in [
        "prime-harness-project-pure",
        "prime-harness-crud",
        "prime-harness-overview",
    ] {
        assert!(role_names.contains(&expected), "missing harness role {expected}: {role_names:?}");
    }
}

#[test]
fn refine_validated_crud_with_rollback_and_record_refinement() {
    let host = host();
    load_harness(&host);
    let session_dir = fresh_dir("refine");
    let sd = session_dir.to_string_lossy().to_string();

    // Invalid collection must be rejected.
    let bad = host
        .call_command("refine", &serde_json::json!({
            "sessionDir": sd,
            "op": "put",
            "collection": "bogus",
            "key": "k",
            "value": { "text": "x" },
        }).to_string()).expect("refine").expect("result");
    assert_eq!(bad["ok"], false, "{bad}");
    assert!(bad["error"].as_str().unwrap().contains("collection"));

    // Silent invalid args loud.
    let bad_args = host
        .call_command("refine", "not-json")
        .expect("refine").expect("result");
    assert_eq!(bad_args["ok"], false, "{bad_args}");

    // begin -> put -> get visible -> rollback -> gone (restores prior nil).
    host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "op": "begin"
    }).to_string()).expect("begin").expect("begin result");

    let put = host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "op": "put", "scope": "local",
        "collection": "memory", "key": "alpha",
        "value": { "text": "remember alpha" },
    }).to_string()).expect("put").expect("put result");
    assert_eq!(put["ok"], true, "{put}");

    let get = host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "op": "get", "scope": "local",
        "collection": "memory", "key": "alpha",
    }).to_string()).expect("get").expect("get result");
    assert_eq!(get["ok"], true, "{get}");
    assert_eq!(get["result"]["value"]["text"], "remember alpha", "{get}");

    let rb = host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "op": "rollback"
    }).to_string()).expect("rollback").expect("rollback result");
    assert_eq!(rb["ok"], true, "{rb}");
    assert_eq!(rb["result"]["txn"], "rolled_back", "{rb}");

    let after = host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "op": "get", "scope": "local",
        "collection": "memory", "key": "alpha",
    }).to_string()).expect("get after").expect("get after result");
    assert!(after["result"]["value"].is_null(), "rollback must restore prior absence: {after}");

    // record_refinement writes a durable refinement entry.
    let rec = host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "scope": "local",
        "op": "record_refinement", "text": "remember beta",
    }).to_string()).expect("record_refinement").expect("record ref result");
    assert_eq!(rec["ok"], true, "{rec}");
    assert_eq!(rec["result"]["op"], "record_refinement", "{rec}");

    // find the refinement via list
    let list = host.call_command("refine", &serde_json::json!({
        "sessionDir": sd, "op": "list", "scope": "local", "collection": "refinement"
    }).to_string()).expect("list").expect("list result");
    assert_eq!(list["ok"], true, "{list}");
    assert!(list["result"]["collections"]["refinement"].as_array().unwrap().len() >= 1, "{list}");
}

#[test]
fn pure_projection_is_deterministic_and_local_overrides_global() {
    let host = host();
    load_harness(&host);

    let input = serde_json::json!({
        "local":  { "memory": [ { "key": "k1", "value": { "text": "local one" } } ] },
        "global": {
            "prompt": [ { "key": "pk", "value": { "text": "prompt g" } } ],
            "skill":  [ { "key": "sk", "value": { "text": "skill g" } } ],
            "memory": [ { "key": "k1", "value": { "text": "global one" } },
                        { "key": "k2", "value": { "text": "global two" } } ],
        },
    }).to_string();

    let a = host.call_role("prime-harness-project-pure", &input)
        .expect("pure role").expect("result");
    let b = host.call_role("prime-harness-project-pure", &input)
        .expect("pure role").expect("result");
    let block_a = a["block"].as_str().expect("block string");
    let block_b = b["block"].as_str().expect("block string");
    assert_eq!(block_a, block_b, "projection must be a pure function (same input -> same output)");
    assert!(!block_a.is_empty());

    // Local wins over global for the same (kind, key).
    assert!(block_a.contains("memory k1: local one"), "local must override global: {block_a}");
    assert!(!block_a.contains("global one"), "global shadowed value must not appear: {block_a}");
    // Global-only entries still surface.
    assert!(block_a.contains("memory k2: global two"), "{block_a}");
    assert!(block_a.contains("prompt pk: prompt g"), "{block_a}");
    // Markdown skill surfaces.
    assert!(block_a.contains("skill sk: skill g"), "{block_a}");
}

#[test]
fn crud_round_trip_survives_restart_and_overview_injects_all_three() {
    // Fresh session: write memory + skill + prompt through one host, drop it,
    // then a brand-new host (a process restart in miniature) must project all
    // three into the injected overview from the durable record store.
    let session_dir = fresh_dir("restart");

    {
        let host = host();
        load_harness(&host);
        let sd = session_dir.to_string_lossy().to_string();
        for (collection, key, text) in [
            ("memory", "m1", "remember fact"),
            ("skill", "s1", "a reusable markdown skill"),
            ("prompt", "p1", "always do X"),
        ] {
            let r = host.call_role("prime-harness-crud", &serde_json::json!({
                "sessionDir": sd, "scope": "local", "op": "put",
                "collection": collection, "key": key,
                "value": { "text": text },
            }).to_string()).expect("crud").expect("crud result");
            assert_eq!(r["ok"], true, "put {collection}/{key}: {r}");
        }
        // host dropped here -> teardown.
    }

    let host = host();
    load_harness(&host);
    let sd = session_dir.to_string_lossy().to_string();
    let overview = host.call_role("prime-harness-overview", &serde_json::json!({
        "sessionDir": sd,
    }).to_string()).expect("overview").expect("overview result");
    let block = overview["block"].as_str().expect("overview block");
    assert!(block.contains("memory m1: remember fact"), "overview lost memory: {block}");
    assert!(block.contains("skill s1: a reusable markdown skill"), "overview lost skill: {block}");
    assert!(block.contains("prompt p1: always do X"), "overview lost prompt: {block}");
}

#[test]
fn rlm_boots_with_empty_harness_when_harness_package_absent() {
    // The P4 "harness removed -> boots" requirement: load ONLY prime/rlm.lua.
    // The RLM package must load, register its role, and expose no harness
    // surface (empty harness). The full loop run without harness is covered
    // by the existing prime_rlm / prime_rlm_loop host tests, which load only
    // the RLM package too.
    let host = host();
    host.load("prime/rlm.lua", include_str!("../../../prime/rlm.lua"))
        .expect("prime RLM package loads without the harness package");

    let roles = host.roles().expect("roles");
    assert!(roles.iter().any(|r| r.role == "prime-rlm"), "RLM role must still register");

    let commands = host.commands().expect("commands mirror");
    assert!(
        !commands.iter().any(|c| c.name == "refine"),
        "with harness removed there must be no /refine command: {commands:?}"
    );
    // Empty harness also means no harness fixture roles.
    assert!(!roles.iter().any(|r| r.id == "prime-harness-overview"));
}
