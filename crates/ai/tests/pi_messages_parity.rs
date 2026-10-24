//! `pi-messages.test.ts`: the response model of a terminal pi-messages event.

mod common;

use std::sync::Arc;

use common::FakeServer;
use pi_ai::api::pi_messages;
use pi_ai::types::{
    AssistantMessage, Message, Model, SimpleStreamOptions, TranscriptContext, UserContent,
    UserMessage, UserRole,
};

fn model(server: &FakeServer) -> Model {
    let mut model = Model::new("radius", "auto", "pi-messages", 128_000, 16_384, false);
    model.name = "Radius Auto".to_owned();
    model.base_url = server.base_url.clone();
    model
}

fn context() -> TranscriptContext {
    TranscriptContext {
        messages: vec![Message::User(Arc::new(UserMessage {
            role: UserRole,
            content: UserContent::Text("Hello".to_owned()),
            timestamp: 1,
        }))],
    }
}

fn run(server: &FakeServer) -> Option<AssistantMessage> {
    let api = pi_messages::api();
    let stream = api.stream_simple(
        &model(server),
        context(),
        SimpleStreamOptions {
            api_key: Some("test-key".to_owned()),
            ..SimpleStreamOptions::default()
        },
    );
    while stream.next_event().is_some() {}
    stream.result()
}

const USAGE: &str = r#"{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15,"cost":{"input":0.1,"output":0.2,"cacheRead":0,"cacheWrite":0,"total":0.3}}"#;

#[test]
fn carries_the_response_model_of_a_done_event() {
    let done = format!(
        r#"{{"type":"done","reason":"stop","usage":{USAGE},"responseId":"resp_1","responseModel":"claude-opus-5-5"}}"#
    );
    let server = FakeServer::start(vec![("", r#"{"type":"start"}"#), ("", &done)]);
    let message = run(&server).expect("the stream ends with a message");
    assert_eq!(message.response_model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(message.response_id.as_deref(), Some("resp_1"));
    assert_eq!(message.model, "auto");
}

#[test]
fn leaves_the_response_model_absent_when_the_event_omits_it() {
    let done = format!(r#"{{"type":"done","reason":"stop","usage":{USAGE},"responseId":"resp_1"}}"#);
    let server = FakeServer::start(vec![("", r#"{"type":"start"}"#), ("", &done)]);
    let message = run(&server).expect("the stream ends with a message");
    assert_eq!(message.response_model, None);
}

#[test]
fn propagates_the_response_model_of_an_error_event() {
    let error = format!(
        r#"{{"type":"error","reason":"error","usage":{USAGE},"errorMessage":"Upstream failed","responseModel":"claude-opus-5-5"}}"#
    );
    let server = FakeServer::start(vec![("", r#"{"type":"start"}"#), ("", &error)]);
    let message = run(&server).expect("the stream ends with a message");
    assert_eq!(message.error_message.as_deref(), Some("Upstream failed"));
    assert_eq!(message.response_model.as_deref(), Some("claude-opus-5-5"));
}
