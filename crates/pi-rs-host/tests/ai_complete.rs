//! `pi.ai.complete` — the streaming-LLM convenience helper dogfood
//! extensions (context-janitor, RLM) reach for through
//! `@earendil-works/pi-ai#completeSimple`. Exercised unprivileged from a
//! file-backed extension command over a custom `streamSimple` provider (the
//! PLAN 9.4 custom-stream dispatch seam): the helper must dispatch ahead of
//! Rust providers, forward `onChunk` streaming deltas, accept a
//! `completeSimple`-style `{ systemPrompt, messages }` request, and return
//! the final AssistantMessage.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use serde_json::json;

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

#[test]
fn complete_streams_through_custom_provider_and_returns_final_message() {
    let host = host();
    host.load(
        "<probe>",
        r#"
            local pi = ...
            pi.register_command("complete-probe", {
                handler = function(args)
                    local req = pi.json.decode(args)
                    local chunks = {}
                    local final = pi.ai.complete(req.model, {
                        systemPrompt = "You are a janitor.",
                        messages = { { role = "user", content = req.question, timestamp = 0 } },
                    }, {
                        onChunk = function(partial)
                            local text = ""
                            if partial and partial.content and partial.content[1] then
                                text = partial.content[1].text or ""
                            end
                            chunks[#chunks + 1] = text
                        end,
                    })
                    local text = ""
                    if final.content and final.content[1] then text = final.content[1].text or "" end
                    return {
                        text = text,
                        stopReason = final.stopReason,
                        provider = final.provider,
                        chunks = chunks,
                    }
                end,
            })
        "#,
    )
    .expect("probe loads");

    host.load(
        "<custom>",
        r#"
            local pi = ...
            pi.register_provider("janitor-stream", {
                api = "janitor-complete-api",
                streamSimple = function(model, context, options, on_event)
                    on_event({ type = "start", partial = { role = "assistant", content = {}, stopReason = "stop", timestamp = 0 } })
                    on_event({ type = "text_delta", delta = "clean", partial = { role = "assistant",
                        content = { { type = "text", text = "clean" } }, stopReason = "stop", timestamp = 0 } })
                    on_event({ type = "text_delta", delta = "ed", partial = { role = "assistant",
                        content = { { type = "text", text = "cleaned" } }, stopReason = "stop", timestamp = 0 } })
                    return { role = "assistant",
                        content = { { type = "text", text = "cleaned" } },
                        api = "janitor-complete-api", provider = "janitor-stream",
                        model = model.id, stopReason = "stop", timestamp = 0 }
                end,
            })
        "#,
    )
    .expect("custom provider loads");

    let model = json!({
        "id":"janitor-1", "name":"Janitor", "api":"janitor-complete-api", "provider":"janitor-stream",
        "baseUrl":"", "reasoning":false, "input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000, "maxTokens":1024
    });
    let got = host
        .call_command("complete-probe", &json!({ "model": model, "question": "drop stale hunk?" }).to_string())
        .unwrap()
        .unwrap();
    // Returns the final message text from the custom provider.
    assert_eq!(got["text"], "cleaned", "{got}");
    assert_eq!(got["stopReason"], "stop");
    assert_eq!(got["provider"], "janitor-stream");
    // onChunk deltas were forwarded for every streaming partial (start has an
    // empty text, then each text_delta rolls forward the accumulated text).
    let chunks: Vec<&str> = got["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(chunks, vec!["", "clean", "cleaned"], "{got}");
}

/// `complete` accepts a raw Context table (system_prompt snake_case) too, and
/// missing providers surface a failure message rather than a thrown error.
#[test]
fn complete_accepts_raw_context_and_failure_message_shape() {
    let host = host();
    host.load(
        "<probe>",
        r#"
            local pi = ...
            pi.register_command("complete-shape", {
                handler = function(args)
                    local req = pi.json.decode(args)
                    local final = pi.ai.complete(req.model, {
                        system_prompt = "root",
                        messages = req.messages,
                    }, {})
                    return {
                        stopReason = final.stopReason,
                        hasError = final.errorMessage ~= nil,
                    }
                end,
            })
        "#,
    )
    .expect("probe loads");

    // A model whose provider has no stream handler: stream_simple returns a
    // failure message with stop_reason=error instead of throwing.
    let model = json!({
        "id":"none-1", "name":"None", "api":"no-such-api", "provider":"ghost",
        "baseUrl":"", "reasoning":false, "input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000, "maxTokens":1024
    });
    let got = host
        .call_command(
            "complete-shape",
            &json!({ "model": model, "messages": [{ "role": "user", "content": "hi", "timestamp": 0 }] })
                .to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(got["stopReason"], "error", "{got}");
    assert_eq!(got["hasError"], true, "{got}");
}
