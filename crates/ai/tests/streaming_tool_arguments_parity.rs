//! `streaming-tool-arguments.test.ts` and `bedrock-stream-state.test.ts`: every provider reads each
//! streamed tool-call argument delta once, shows growing partial arguments and finishes with the
//! exact arguments.

mod common;

use std::sync::Arc;

use common::FakeServer;
use pi_ai::api::{anthropic, openai_completions, pi_messages};
use pi_ai::providers::faux::{Faux, FauxOptions, FauxResponse, FauxTokenSize};
use pi_ai::types::{
    AssistantBlock, AssistantMessage, AssistantMessageEvent, AssistantRole, CacheRetention, Cost,
    Message, Model, SimpleStreamOptions, StopReason, ToolCall, ToolCallType, TranscriptContext,
    Usage, UserContent, UserMessage, UserRole,
};
use pi_ai::utils::streaming_json::ToolCallArgumentsStream;

const LINES: usize = 200;
const DELTA: usize = 24;

fn file_content() -> String {
    (0..LINES)
        .map(|line| format!("const line{line} = \"value {line}\"; // \\\\ done\\n"))
        .collect()
}

fn write_arguments() -> (String, String) {
    let content = file_content();
    let text = serde_json::json!({"path": "/src/file.ts", "content": content}).to_string();
    (content, text)
}

fn deltas(text: &str, size: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let mut end = (index + size).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == index {
            end += text[index..].chars().next().map_or(1, char::len_utf8);
        }
        pieces.push(&text[index..end]);
        index = end;
    }
    pieces
}

fn context() -> TranscriptContext {
    TranscriptContext {
        messages: vec![Message::User(Arc::new(UserMessage {
            role: UserRole,
            content: UserContent::Text("write".to_owned()),
            timestamp: 0,
        }))],
    }
}

fn model(server: &FakeServer, provider: &str, id: &str, api: &str) -> Model {
    let mut model = Model::new(provider, id, api, 200_000, 64_000, false);
    model.base_url = server.base_url.clone();
    model
}

fn options() -> SimpleStreamOptions {
    SimpleStreamOptions {
        api_key: Some("test-key".to_owned()),
        cache_retention: Some(CacheRetention::None),
        ..SimpleStreamOptions::default()
    }
}

/// Drive a provider's stream, checking that partial arguments grow and finish exactly.
fn check(
    stream: pi_ai::event_stream::AssistantMessageEventStream,
    expected: &str,
) -> AssistantMessage {
    let mut shown = 0;
    let mut partials = 0;
    let mut message = None;
    loop {
        let Some(event) = stream.next_event() else {
            break;
        };
        if let AssistantMessageEvent::ToolcallDelta {
            content_index,
            partial,
            ..
        } = &event
        {
            let call = partial.content.get(*content_index as usize);
            let arguments = match call {
                Some(AssistantBlock::ToolCall(call)) => &call.arguments,
                _ => panic!("a toolcall delta points at {call:?}"),
            };
            let shown_content = arguments.get("content").and_then(|value| value.as_str());
            if let Some(shown_content) = shown_content {
                partials += 1;
                assert!(
                    expected.starts_with(shown_content),
                    "partial content {:?} is not a prefix of the arguments",
                    &shown_content[..shown_content.len().min(40)]
                );
                assert!(
                    shown_content.len() >= shown,
                    "the partial arguments only grow"
                );
                shown = shown_content.len();
            }
        }
        message = Some(event);
    }
    let _ = message;
    let message = stream.result().expect("the stream ends with a message");
    assert_eq!(
        message.stop_reason,
        StopReason::ToolUse,
        "{:?}",
        message.error_message
    );
    assert!(partials > 100, "the arguments grew in {partials} deltas");
    assert_eq!(message.content.len(), 1);
    match &message.content[0] {
        AssistantBlock::ToolCall(call) => {
            assert_eq!(call.name, "write");
            assert_eq!(
                call.arguments
                    .get("content")
                    .and_then(|value| value.as_str()),
                Some(expected)
            );
        }
        other => panic!("expected a tool call, got {other:?}"),
    }
    message
}

#[test]
fn anthropic_messages_streams_growing_arguments() {
    let (content, text) = write_arguments();
    let mut script: Vec<(String, String)> = vec![
        (
            "message_start".to_owned(),
            r#"{"message":{"id":"msg_1","model":"claude-test","usage":{"input_tokens":1,"output_tokens":1}}}"#.to_owned(),
        ),
        (
            "content_block_start".to_owned(),
            r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"write","input":{}}}"#.to_owned(),
        ),
    ];
    for piece in deltas(&text, DELTA) {
        script.push((
            "content_block_delta".to_owned(),
            serde_json::json!({"index": 0, "delta": {"type": "input_json_delta", "partial_json": piece}})
                .to_string(),
        ));
    }
    script.push(("content_block_stop".to_owned(), r#"{"index":0}"#.to_owned()));
    script.push((
        "message_delta".to_owned(),
        r#"{"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":10}}"#.to_owned(),
    ));
    script.push(("message_stop".to_owned(), "{}".to_owned()));
    let script: Vec<(&str, &str)> = script
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_str()))
        .collect();
    let server = FakeServer::start(script);
    let model = model(&server, "anthropic", "claude-test", "anthropic-messages");
    let stream = anthropic::api().stream_simple(&model, context(), options());
    check(stream, &content);
}

#[test]
fn openai_completions_streams_growing_arguments() {
    let (content, text) = write_arguments();
    let mut script: Vec<String> = vec![
        serde_json::json!({"id": "chat_1", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "write", "arguments": ""}}]}}]})
            .to_string(),
    ];
    for piece in deltas(&text, DELTA) {
        script.push(
            serde_json::json!({"id": "chat_1", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": piece}}]}}]})
                .to_string(),
        );
    }
    script.push(
        serde_json::json!({"id": "chat_1", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
            .to_string(),
    );
    let mut pairs: Vec<(&str, &str)> = script.iter().map(|data| ("", data.as_str())).collect();
    pairs.push(("", "[DONE]"));
    let server = FakeServer::start(pairs);
    let model = model(&server, "openai", "gpt-test", "openai-completions");
    let stream = openai_completions::api().stream_simple(&model, context(), options());
    check(stream, &content);
}

#[test]
fn pi_messages_streams_growing_arguments() {
    let (content, text) = write_arguments();
    let mut script: Vec<String> = vec![
        r#"{"type":"start"}"#.to_owned(),
        r#"{"type":"toolcall_start","contentIndex":0,"id":"call_1","toolName":"write"}"#.to_owned(),
    ];
    for piece in deltas(&text, DELTA) {
        script.push(
            serde_json::json!({"type": "toolcall_delta", "contentIndex": 0, "delta": piece})
                .to_string(),
        );
    }
    script.push(
        serde_json::json!({"type": "toolcall_end", "contentIndex": 0, "toolCall": {"type": "toolCall", "id": "call_1", "name": "write", "arguments": serde_json::from_str::<serde_json::Value>(&text).expect("valid JSON")}})
            .to_string(),
    );
    script.push(
        r#"{"type":"done","reason":"toolUse","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}}"#
            .to_owned(),
    );
    let pairs: Vec<(&str, &str)> = script.iter().map(|data| ("", data.as_str())).collect();
    let server = FakeServer::start(pairs);
    let model = model(&server, "radius", "auto", "pi-messages");
    let stream = pi_messages::api().stream_simple(&model, context(), options());
    check(stream, &content);
}

#[test]
fn faux_streams_growing_arguments() {
    let (content, _) = write_arguments();
    let faux = Faux::new(FauxOptions {
        token_size: FauxTokenSize {
            min: Some(8),
            max: Some(8),
        },
        ..FauxOptions::default()
    });
    let arguments = serde_json::json!({"path": "/src/file.ts", "content": content});
    faux.set_responses(vec![FauxResponse::Message(Box::new(AssistantMessage {
        role: AssistantRole,
        content: vec![AssistantBlock::ToolCall(ToolCall {
            kind: ToolCallType,
            id: "call_1".to_owned(),
            name: "write".to_owned(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
            thought_signature: None,
            namespace: None,
        })],
        api: "faux".to_owned(),
        provider: "faux".to_owned(),
        model: "faux-1".to_owned(),
        response_model: None,
        response_id: None,
        provider_thinking_level: None,
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
                total_from_provider: None,
            },
        },
        stop_reason: StopReason::ToolUse,
        deferred: None,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        duration_ms: None,
        timestamp: 0,
    }))]);
    let model = faux.default_model();
    let stream = faux.stream(&model, context(), options());
    check(stream, &content);
}

#[test]
fn the_arguments_parser_keeps_one_parse_for_the_finished_text() {
    let (_, text) = write_arguments();
    let mut stream = ToolCallArgumentsStream::new();
    for piece in deltas(&text, DELTA) {
        stream.append(piece);
    }
    assert_eq!(stream.text(), text);
    let expected = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .expect("an object");
    assert_eq!(stream.finish(), expected);
}
