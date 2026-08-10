//! P3 RLM loop end-to-end through the public loader (scripted provider).
//!
//! Drives the `prime-rlm` role the way the `.#prime` flake app does: load
//! `prime/rlm.lua` as an ordinary file-backed package on the public loader,
//! dispatch to the `prime-rlm` role it registers, and let the loop run
//! `pi.ai.stream_simple` against a scripted API provider registered through
//! the public `pi-rs-ai::registry` (the same registration path real providers
//! use — no dedicated test hook). The loop must reach a prose stop and return
//! the scripted assistant message.
//!
//! The test requires the real kernel python env (pi.repl spawns an IPython
//! child in create_session). In the bare `cargo test`/`cargo clippy`
//! environment the kernel is skipped by detecting the absence of IPython, so
//! this file stays green offline; the flake `prime-rlm` check runs it under
//! the Nix kernel env where the kernel is real.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pi_rs_ai::registry::register_api_provider;
use pi_rs_ai::transport::create_assistant_message_event_stream;
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantRole, StopReason, TextContent, Usage, now_ms,
};
use pi_rs_host::{Host, HostConfig};

fn kernel_envurable() -> bool {
    if let Ok(py) = std::env::var("PI_RS_REPL_PYTHON") {
        return !py.is_empty();
    }
    // Fallback probe: is IPython importable on the default python3?
    std::process::Command::new("python3")
        .arg("-c")
        .arg("import IPython")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn scripted_message(text: &str) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContent::Text(TextContent::new(text))],
        api: "prime-rlm-scripted".to_string(),
        provider: "scripted".to_string(),
        model: "prime-scripted-1".to_string(),
        response_model: None,
        response_id: None,
        diagnostics: None,
        usage: Usage::default(),
        stop_reason: StopReason::Stop,
        error_message: None,
        timestamp: now_ms(),
    }
}

/// Register a scripted provider (api `prime-rlm-scripted`) that emits a
/// single prose-only assistant turn and completes.
fn scripted_provider(text: String) {
    let text_simple = text.clone();
    register_api_provider(
        pi_rs_ai::registry::ApiProvider {
            api: "prime-rlm-scripted".to_string(),
            stream: {
                let text = text.clone();
                Arc::new(move |_, _, _| {
                    let stream = create_assistant_message_event_stream();
                    let message = scripted_message(&text);
                    stream.push(pi_rs_ai_types::AssistantMessageEvent::Start {
                        partial: message.clone(),
                    });
                    stream.push(pi_rs_ai_types::AssistantMessageEvent::Done {
                        reason: StopReason::Stop,
                        message: message.clone(),
                    });
                    stream.end();
                    Ok(stream)
                })
            },
            stream_simple: Arc::new(move |_, _, _| {
                let stream = create_assistant_message_event_stream();
                let message = scripted_message(&text_simple);
                stream.push(pi_rs_ai_types::AssistantMessageEvent::Start {
                    partial: message.clone(),
                });
                stream.push(pi_rs_ai_types::AssistantMessageEvent::Done {
                    reason: StopReason::Stop,
                    message: message.clone(),
                });
                stream.end();
                Ok(stream)
            }),
        },
        Some("prime-rlm-loop-test"),
    );
}

#[test]
fn prime_rlm_loop_reaches_prose_stop_through_public_loader() {
    if !kernel_envurable() {
        // The bare offline environment has no IPython kernel; the flake
        // `prime-rlm` check runs this under the Nix kernel env.
        eprintln!("skipped: no IPython kernel env (PI_RS_REPL_PYTHON unset / IPython absent)");
        return;
    }
    // Clean up any prior registration of the same source id.
    pi_rs_ai::registry::unregister_api_providers("prime-rlm-loop-test");
    scripted_provider("scripted prose answer".to_string());

    let host = Host::new(HostConfig {
        dispatch_timeout_ms: 90_000,
        cwd: None,
        project_trusted: true,
    })
    .expect("host");
    host.load("prime/rlm.lua", include_str!("../../../prime/rlm.lua"))
        .expect("prime RLM package loads");

    let request = serde_json::json!({
        "sessionId": "prime-loop-test",
        "sessionDir": std::env::temp_dir().join("prime-rlm-test").to_string_lossy(),
        "model": { "id": "prime-scripted-1", "provider": "scripted",
                   "api": "prime-rlm-scripted", "name": "Scripted",
                   "baseUrl": "http://127.0.0.1:1", "reasoning": false,
                   "input": ["text"],
                   "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
                   "contextWindow": 100000, "maxTokens": 4096 },
        "prompt": "hello",
    });

    let result = host
        .call_role("prime-rlm", &request.to_string())
        .expect("role dispatch")
        .expect("role returned a result");

    let message = result.get("result").expect("result field");
    assert_eq!(message["stopReason"], "stop", "{result}");
    let content = message["content"].as_array().expect("content array");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text", "{result}");
    assert_eq!(content[0]["text"], "scripted prose answer", "{result}");

    pi_rs_ai::registry::unregister_api_providers("prime-rlm-loop-test");
}
