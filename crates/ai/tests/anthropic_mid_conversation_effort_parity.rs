//! `anthropic-mid-conversation-effort.test.ts`: the fixed top-level effort sent with
//! per-message effort markers.

mod common;

use std::sync::Arc;

use common::FakeServer;
use pi_ai::api::anthropic;
use pi_ai::types::{
    AssistantBlock, AssistantMessage, AssistantRole, CacheRetention, Cost, Message, Model,
    SimpleStreamOptions, StopReason, TextContent, TextType, ThinkingContent, ThinkingLevel,
    ThinkingType, TranscriptContext, Usage, UserContent, UserMessage, UserRole,
};
use serde_json::{Value, json};

const STREAM: [(&str, &str); 5] = [
    (
        "message_start",
        r#"{"message":{"id":"msg_1","model":"claude-fable-5-1","usage":{"input_tokens":1,"output_tokens":1}}}"#,
    ),
    (
        "content_block_start",
        r#"{"index":0,"content_block":{"type":"text","text":""}}"#,
    ),
    (
        "content_block_delta",
        r#"{"index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    ),
    (
        "message_delta",
        r#"{"delta":{"stop_reason":"end_turn"},"usage":{}}"#,
    ),
    ("message_stop", "{}"),
];

fn model(server: &FakeServer) -> Model {
    let mut model = Model::new("anthropic", "claude-fable-5-1", "anthropic-messages", 200_000, 32_000, true);
    model.name = "Claude Fable 5.1".to_owned();
    model.base_url = server.base_url.clone();
    model.thinking_level_map = json!({
        "off": null,
        "minimal": "low",
        "low": "low",
        "medium": "medium",
        "high": "high",
        "max": "max",
    })
    .as_object()
    .cloned();
    model.compat = Some(json!({"forceAdaptiveThinking": true, "supportsMidConvoEffort": true}));
    model
}

fn user(text: &str, timestamp: u64) -> Message {
    Message::User(Arc::new(UserMessage {
        role: UserRole,
        content: UserContent::Text(text.to_owned()),
        timestamp,
    }))
}

fn assistant(model: &Model, level: &str) -> Message {
    Message::Assistant(Arc::new(AssistantMessage {
        role: AssistantRole,
        content: vec![
            AssistantBlock::Thinking(ThinkingContent {
                kind: ThinkingType,
                thinking: "reasoning".to_owned(),
                thinking_signature: Some("signature".to_owned()),
                redacted: None,
            }),
            AssistantBlock::Text(TextContent {
                kind: TextType,
                text: "answer".to_owned(),
                text_signature: None,
            }),
        ],
        api: "anthropic-messages".to_owned(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_model: None,
        response_id: None,
        provider_thinking_level: Some(level.to_owned()),
        thinking_level: None,
        diagnostics: None,
        usage: Usage {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: None,
            total_tokens: 0,
            cost: Cost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.0,
            },
        },
        stop_reason: StopReason::Stop,
        deferred: None,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        duration_ms: None,
        timestamp: 1,
    }))
}

fn capture(model: &Model, messages: Vec<Message>, level: ThinkingLevel) -> AssistantMessage {
    let api = anthropic::api();
    let stream = api.stream_simple(
        model,
        TranscriptContext { messages },
        SimpleStreamOptions {
            api_key: Some("test-key".to_owned()),
            cache_retention: Some(CacheRetention::None),
            reasoning: Some(level),
            ..SimpleStreamOptions::default()
        },
    );
    while stream.next_event().is_some() {}
    stream.result().expect("the stream ends with a message")
}

fn markers(body: &Value) -> Vec<Value> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "system")
        .cloned()
        .collect()
}

#[test]
fn sends_a_fixed_low_top_level_effort_with_per_message_markers() {
    let server = FakeServer::start(STREAM.to_vec());
    let model = model(&server);

    let first = capture(&model, vec![user("one", 1)], ThinkingLevel::Low);
    capture(
        &model,
        vec![user("one", 1), assistant(&model, "low"), user("two", 2)],
        ThinkingLevel::High,
    );

    let bodies = server.bodies();
    assert_eq!(bodies.len(), 2, "one request per capture");
    let (first_body, second_body) = (&bodies[0], &bodies[1]);

    let first_messages = first_body["messages"].as_array().expect("messages");
    assert_eq!(
        first_messages,
        &vec![
            json!({"role": "user", "content": "one"}),
            json!({"role": "system", "content": [], "output_config": {"effort": "low"}}),
        ]
    );
    assert_eq!(
        &second_body["messages"].as_array().expect("messages")[..first_messages.len()],
        first_messages.as_slice(),
        "the historical marker prefix is reconstructed exactly"
    );
    assert_eq!(
        markers(second_body).last(),
        Some(&json!({"role": "system", "content": [], "output_config": {"effort": "high"}}))
    );
    assert_eq!(first_body["output_config"], json!({"effort": "low"}));
    assert_eq!(second_body["output_config"], json!({"effort": "low"}));
    assert_eq!(
        second_body["thinking"],
        json!({
            "type": "adaptive",
            "display": "summarized",
            "block_binding": {"prefix_mismatch_behavior": "drop_block"},
        })
    );
    assert_eq!(first.provider_thinking_level.as_deref(), Some("low"));
}
