//! `pi.session.*` — the session persistence mechanism (PLAN 6.1).
//!
//! Entry shapes, tree bookkeeping, and persist timing are pinned to pi by
//! the pi-rs-session unit suite and tests/session-parity; these tests cover
//! the Lua binding surface itself via the examples/ exerciser.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn session_demo_walks_create_open_and_in_memory_handles() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .expect("host");
    let path = format!(
        "{}/../../examples/extensions/session-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let request = serde_json::json!({
        "cwd": cwd.to_string_lossy(),
        "sessionDir": temp.path().join("sessions").to_string_lossy(),
        "agentDir": temp.path().to_string_lossy(),
    });
    let result = host
        .call_command("session-demo", &request.to_string())
        .expect("command")
        .expect("result");

    // Spec _persist: nothing hits disk before the first assistant message.
    assert_eq!(result["deferredUntilAssistant"], true);

    let session_file = result["sessionFile"].as_str().expect("session file path");
    let content = std::fs::read_to_string(session_file).unwrap();
    let entries: Vec<serde_json::Value> = content
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let types: Vec<&str> = entries
        .iter()
        .map(|entry| entry["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "session",
            "model_change",
            "thinking_level_change",
            "message",
            "message",
            "session_info"
        ]
    );
    assert_eq!(entries[0]["id"], result["sessionId"]);
    assert_eq!(entries[0]["cwd"].as_str().unwrap(), cwd.to_string_lossy());

    // Read side: leaf chases the last append; context excludes non-messages.
    assert_eq!(result["leafId"], entries[5]["id"]);
    assert_eq!(result["name"], "demo session");
    assert_eq!(result["entryCount"], 5); // getEntries excludes the header
    assert_eq!(result["branchCount"], 5);
    assert_eq!(result["contextMessages"], 2);
    assert_eq!(result["contextModel"]["provider"], "demo-provider");
    assert_eq!(result["contextModel"]["modelId"], "demo-model");

    // open() reconstructs id and name from the file.
    assert_eq!(result["reopenedId"], result["sessionId"]);
    assert_eq!(result["reopenedName"], "demo session");

    // inMemory never persists.
    assert_eq!(result["inMemoryPersisted"], false);
    assert!(result["inMemoryFile"].is_null());

    // Listing (PLAN 6.3): the persisted session shows up with the spec's
    // SessionInfo fields in both list and listAll (custom flat dir).
    assert_eq!(result["listedCount"], 1);
    assert_eq!(result["listedAllCount"], 1);
    assert_eq!(result["listedName"], "demo session");
    assert_eq!(result["listedFirstMessage"], "hello");
    assert_eq!(result["listedMessageCount"], 2);
    // The demo passes an explicit sessionDir, so it is not the default.
    assert_eq!(result["usesDefaultSessionDir"], false);

    // Branching (PLAN 6.4): the tree resolves labels onto nodes, branch()
    // forks at the user message, branch_with_summary moves the leaf to
    // the summary entry, and in-memory create_branched_session returns no
    // path (the file variants are pinned by pi-rs-session's unit suite and
    // pi-rs-app's interactive_tree tests).
    assert_eq!(result["treeRoots"], 1);
    assert_eq!(result["branchChildren"], 2);
    assert_eq!(result["labeledEntryIsUser"], true);
    assert_eq!(result["labeledLabel"], "important");
    assert_eq!(result["summaryLeaf"], true);
    assert_eq!(result["summarySummary"], "Explored a second take.");
    assert_eq!(result["summaryFromId"], result["labeledEntry"]);
    assert!(result["branchedFile"].is_null());

    // messages.ts timestamp semantics: JS Date.parse of the ISO string.
    assert_eq!(result["isoMs"], 1_782_900_000_000_i64);

    // Compaction (PLAN 6.5): append_compaction cuts the rebuilt context
    // over to the summary message + kept entries, and the standalone
    // pi.session.build_context computes the same result over raw entries.
    assert_eq!(result["compactedMessages"], 2);
    assert_eq!(result["compactedFirstRole"], "compactionSummary");
    assert_eq!(result["compactedSummary"], "What came before.");
    assert_eq!(result["standaloneMessages"], 2);
}
