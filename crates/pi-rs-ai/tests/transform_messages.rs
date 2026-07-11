//! Behavioral pins for `protocols::transform_messages` against the
//! spec's `providers/transform-messages.ts`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::protocols::transform_messages::transform_messages;
use pi_rs_ai_types::{AssistantContent, Message, Model, TextOrImageContent, UserContent};
use serde_json::{Value, json};

fn model(provider: &str, id: &str, vision: bool) -> Model {
    let input = if vision {
        json!(["text", "image"])
    } else {
        json!(["text"])
    };
    serde_json::from_value(json!({
        "id": id,
        "name": id,
        "api": "anthropic-messages",
        "provider": provider,
        "baseUrl": "https://api.anthropic.com",
        "reasoning": true,
        "input": input,
        "cost": { "input": 5, "output": 25, "cacheRead": 0.5, "cacheWrite": 6.25 },
        "contextWindow": 1000000,
        "maxTokens": 128000
    }))
    .unwrap()
}

fn messages(value: Value) -> Vec<Message> {
    serde_json::from_value(value).unwrap()
}

fn assistant(provider: &str, id: &str, content: Value, stop_reason: &str) -> Value {
    json!({
        "role": "assistant",
        "content": content,
        "api": "anthropic-messages",
        "provider": provider,
        "model": id,
        "usage": {
            "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
            "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 }
        },
        "stopReason": stop_reason,
        "timestamp": 1
    })
}

#[test]
fn cross_model_thinking_becomes_text_and_signatures_drop() {
    let target = model("anthropic", "claude-opus-4-7", true);
    let msgs = messages(json!([assistant(
        "openai",
        "gpt-5",
        json!([
            { "type": "thinking", "thinking": "hidden plan", "thinkingSignature": "sig" },
            { "type": "thinking", "thinking": "   " },
            { "type": "thinking", "thinking": "redacted", "thinkingSignature": "opaque", "redacted": true },
            { "type": "text", "text": "hello", "textSignature": "ts1" },
            { "type": "toolCall", "id": "call|1", "name": "read", "arguments": {}, "thoughtSignature": "g" }
        ]),
        "toolUse"
    )]));

    let out = transform_messages(&msgs, &target, Some(&|id, _m, _s| id.replace('|', "_")));
    // assistant + synthetic tool result for the orphaned call
    assert_eq!(out.len(), 2);
    let Message::Assistant(a) = &out[0] else {
        panic!()
    };
    // thinking → text; blank thinking dropped; redacted dropped cross-model
    assert_eq!(a.content.len(), 3);
    let AssistantContent::Text(t0) = &a.content[0] else {
        panic!()
    };
    assert_eq!(t0.text, "hidden plan");
    let AssistantContent::Text(t1) = &a.content[1] else {
        panic!()
    };
    assert_eq!(t1.text, "hello");
    assert_eq!(t1.text_signature, None);
    let AssistantContent::ToolCall(tc) = &a.content[2] else {
        panic!()
    };
    assert_eq!(tc.id, "call_1");
    assert_eq!(tc.thought_signature, None);
    // synthetic result carries the normalized id
    let Message::ToolResult(tr) = &out[1] else {
        panic!()
    };
    assert_eq!(tr.tool_call_id, "call_1");
    assert!(tr.is_error);
    let TextOrImageContent::Text(text) = &tr.content[0] else {
        panic!()
    };
    assert_eq!(text.text, "No result provided");
}

#[test]
fn same_model_keeps_signed_thinking_even_when_empty() {
    let target = model("anthropic", "claude-opus-4-7", true);
    let msgs = messages(json!([assistant(
        "anthropic",
        "claude-opus-4-7",
        json!([
            { "type": "thinking", "thinking": "", "thinkingSignature": "sig" },
            { "type": "thinking", "thinking": "", "thinkingSignature": "" }
        ]),
        "stop"
    )]));
    let out = transform_messages(&msgs, &target, None);
    let Message::Assistant(a) = &out[0] else {
        panic!()
    };
    // Signed empty thinking survives; unsigned empty thinking drops.
    assert_eq!(a.content.len(), 1);
}

#[test]
fn tool_result_ids_follow_normalized_tool_calls() {
    let target = model("anthropic", "claude-opus-4-7", true);
    let msgs = messages(json!([
        assistant("openai", "gpt-5", json!([
            { "type": "toolCall", "id": "fc|abc", "name": "read", "arguments": {} }
        ]), "toolUse"),
        {
            "role": "toolResult",
            "toolCallId": "fc|abc",
            "toolName": "read",
            "content": [{ "type": "text", "text": "ok" }],
            "isError": false,
            "timestamp": 1
        }
    ]));
    let out = transform_messages(&msgs, &target, Some(&|id, _m, _s| id.replace('|', "_")));
    assert_eq!(out.len(), 2);
    let Message::ToolResult(tr) = &out[1] else {
        panic!()
    };
    assert_eq!(tr.tool_call_id, "fc_abc");
    assert!(!tr.is_error);
}

#[test]
fn errored_assistant_turns_are_skipped() {
    let target = model("anthropic", "claude-opus-4-7", true);
    let msgs = messages(json!([
        { "role": "user", "content": "hi", "timestamp": 1 },
        assistant("anthropic", "claude-opus-4-7", json!([{ "type": "text", "text": "partial" }]), "error"),
        assistant("anthropic", "claude-opus-4-7", json!([{ "type": "text", "text": "done" }]), "stop")
    ]));
    let out = transform_messages(&msgs, &target, None);
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], Message::User(_)));
    let Message::Assistant(a) = &out[1] else {
        panic!()
    };
    let AssistantContent::Text(t) = &a.content[0] else {
        panic!()
    };
    assert_eq!(t.text, "done");
}

#[test]
fn user_message_interrupting_tool_flow_gets_synthetic_results_first() {
    let target = model("anthropic", "claude-opus-4-7", true);
    let msgs = messages(json!([
        assistant("anthropic", "claude-opus-4-7", json!([
            { "type": "toolCall", "id": "toolu_1", "name": "read", "arguments": {} }
        ]), "toolUse"),
        { "role": "user", "content": "never mind", "timestamp": 1 }
    ]));
    let out = transform_messages(&msgs, &target, None);
    assert_eq!(out.len(), 3);
    assert!(matches!(&out[0], Message::Assistant(_)));
    let Message::ToolResult(tr) = &out[1] else {
        panic!()
    };
    assert_eq!(tr.tool_call_id, "toolu_1");
    assert!(matches!(&out[2], Message::User(_)));
}

#[test]
fn non_vision_models_downgrade_images_to_placeholders() {
    let target = model("anthropic", "claude-text-only", false);
    let msgs = messages(json!([
        {
            "role": "user",
            "content": [
                { "type": "image", "data": "aaaa", "mimeType": "image/png" },
                { "type": "image", "data": "bbbb", "mimeType": "image/png" },
                { "type": "text", "text": "what is this?" }
            ],
            "timestamp": 1
        },
        {
            "role": "toolResult",
            "toolCallId": "toolu_1",
            "toolName": "screenshot",
            "content": [{ "type": "image", "data": "cccc", "mimeType": "image/png" }],
            "isError": false,
            "timestamp": 1
        }
    ]));
    let out = transform_messages(&msgs, &target, None);
    let Message::User(user) = &out[0] else {
        panic!()
    };
    let UserContent::Blocks(blocks) = &user.content else {
        panic!()
    };
    // Consecutive images collapse into one placeholder.
    assert_eq!(blocks.len(), 2);
    let TextOrImageContent::Text(p) = &blocks[0] else {
        panic!()
    };
    assert_eq!(p.text, "(image omitted: model does not support images)");
    let Message::ToolResult(tr) = &out[1] else {
        panic!()
    };
    let TextOrImageContent::Text(p) = &tr.content[0] else {
        panic!()
    };
    assert_eq!(
        p.text,
        "(tool image omitted: model does not support images)"
    );
}
