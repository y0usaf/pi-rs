//! End-to-end `/model` behavior through the product wiring (PLAN 3a.3):
//! the `interactive-model-flow` exerciser routes `/model` through
//! `handle_submit`, resolves exact references over the real `pi.ai`
//! registry bridge, and runs prompts through a scripted stream function
//! so the test can assert which model and API key the next provider
//! request used.
//!
//! This file is its own test binary: it owns the process-global
//! `PI_CODING_AGENT_DIR`.

#![allow(clippy::unwrap_used)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

#[test]
fn model_command_switches_the_next_provider_request() {
    let agent_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        agent_dir.path().join("auth.json"),
        serde_json::json!({
            "anthropic": { "type": "api_key", "key": "sk-a" },
            "openai": { "type": "api_key", "key": "sk-o" },
        })
        .to_string(),
    )
    .unwrap();
    // The VM's auth storage resolves PI_CODING_AGENT_DIR at Host::new.
    // SAFETY: this test binary has no other threads reading the env yet.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };

    let request = serde_json::json!({
        "model": {
            "id": "claude-opus-4-8", "provider": "anthropic",
            "api": "anthropic-messages", "name": "Claude Opus 4.8",
            "reasoning": true, "contextWindow": 200000,
        },
        "steps": [
            // Baseline request on the startup model.
            { "submit": "hello" },
            // Exact provider/id reference switches without a selector.
            { "submit": "/model openai/gpt-5.4" },
            { "submit": "hi again" },
            // Ambiguous search opens the selector; escape cancels it.
            { "submit": "/model claude" },
            { "input": ["\u{1b}"] }
        ]
    });

    let result = host()
        .call_command("interactive-model-flow", &request.to_string())
        .unwrap()
        .unwrap();

    // The next provider request after /model used the selected model and
    // that provider's stored API key (per-request getApiKey resolution).
    let requests = result["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 2, "{result}");
    assert_eq!(requests[0]["provider"], "anthropic");
    assert_eq!(requests[0]["model"], "claude-opus-4-8");
    assert_eq!(requests[0]["apiKey"], "sk-a");
    assert_eq!(requests[1]["provider"], "openai");
    assert_eq!(requests[1]["model"], "gpt-5.4");
    assert_eq!(requests[1]["apiKey"], "sk-o");

    // Frontend and agent state both track the selection.
    assert_eq!(result["model"]["provider"], "openai");
    assert_eq!(result["model"]["id"], "gpt-5.4");
    assert_eq!(result["agent_model"], "gpt-5.4");

    // Status row exactly as pi's handleModelCommand shows it.
    let rows = result["transcript"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "status" && row["text"] == "Model: gpt-5.4"),
        "{result}"
    );

    // The ambiguous search mounted the selector and escape restored the
    // editor with focus.
    assert_eq!(result["overlay"], false, "{result}");
    assert_eq!(result["editor_focused"], true);

    // Both configured providers count toward the footer's provider count
    // (ambient env keys may add more on a developer machine).
    assert!(result["provider_count"].as_u64().unwrap() >= 2, "{result}");
}
