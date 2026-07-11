//! Round-trip fixture tests: every fixture deserializes into its typed shape
//! and re-serializes to the identical `serde_json::Value` (numbers normalized
//! to f64 — JSON does not distinguish `0` from `0.0`, Rust types do).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai_types::{
    AssistantImages, AssistantMessageEvent, Context, ImagesModel, Message, Model, ToolCall,
    UserContent,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {path}: {e}"))
}

/// Normalize every number to f64 so `5` and `5.0` compare equal.
fn normalize(value: &Value) -> Value {
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| value.clone()),
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), normalize(v))).collect())
        }
        _ => value.clone(),
    }
}

fn roundtrip<T: DeserializeOwned + Serialize>(name: &str) -> T {
    let original = fixture(name);
    let typed: T = serde_json::from_value(original.clone())
        .unwrap_or_else(|e| panic!("{name}: deserialize failed: {e}"));
    let reserialized =
        serde_json::to_value(&typed).unwrap_or_else(|e| panic!("{name}: serialize failed: {e}"));
    assert_eq!(
        normalize(&original),
        normalize(&reserialized),
        "{name}: round-trip mismatch"
    );
    typed
}

const MESSAGE_FIXTURES: &[&str] = &[
    "message_assistant_tooluse.json",
    "message_assistant_stop.json",
    "message_assistant_error.json",
    "message_assistant_aborted.json",
    "message_assistant_diagnostics.json",
    "message_assistant_google.json",
    "message_user_string.json",
    "message_user_blocks.json",
    "message_user_image.json",
    "message_toolresult.json",
    "message_toolresult_details.json",
    "message_toolresult_image.json",
];

const MODEL_FIXTURES: &[&str] = &[
    "model_anthropic_opus47.json",
    "model_antling_ring.json",
    "model_copilot_haiku45.json",
    "model_bedrock_nova2lite.json",
];

#[test]
fn messages_roundtrip() {
    for name in MESSAGE_FIXTURES {
        let _: Message = roundtrip(name);
    }
}

#[test]
fn models_roundtrip() {
    for name in MODEL_FIXTURES {
        let _: Model = roundtrip(name);
    }
}

#[test]
fn images_model_roundtrips() {
    let _: ImagesModel = roundtrip("images_model_flux2flex.json");
}

#[test]
fn assistant_images_roundtrips() {
    let _: AssistantImages = roundtrip("assistant_images_openrouter.json");
}

#[test]
fn events_roundtrip() {
    let events: Vec<AssistantMessageEvent> = roundtrip("events_stream.json");
    assert_eq!(events.len(), 12, "all 12 event kinds covered");
}

#[test]
fn context_roundtrips() {
    let context: Context = roundtrip("context_full.json");
    assert_eq!(context.messages.len(), 3);
    assert!(context.system_prompt.is_some());
    assert_eq!(context.tools.as_ref().map(Vec::len), Some(1));
}

/// The `role` markers must drive union dispatch: a toolResult message shares
/// its field shape with a user blocks message (`content` array + `timestamp`)
/// and would misparse without tag validation.
#[test]
fn message_union_dispatches_on_role() {
    assert!(matches!(
        serde_json::from_value::<Message>(fixture("message_toolresult.json")).unwrap(),
        Message::ToolResult(_)
    ));
    assert!(matches!(
        serde_json::from_value::<Message>(fixture("message_user_blocks.json")).unwrap(),
        Message::User(user) if matches!(user.content, UserContent::Blocks(_))
    ));
    assert!(matches!(
        serde_json::from_value::<Message>(fixture("message_user_string.json")).unwrap(),
        Message::User(user) if matches!(user.content, UserContent::Text(_))
    ));
    assert!(matches!(
        serde_json::from_value::<Message>(fixture("message_assistant_stop.json")).unwrap(),
        Message::Assistant(_)
    ));
}

/// A wrong literal tag must be rejected, not silently accepted.
#[test]
fn wrong_role_tag_is_rejected() {
    let mut value = fixture("message_user_string.json");
    value["role"] = "toolResult".into();
    assert!(serde_json::from_value::<Message>(value).is_err());

    let bad_block = serde_json::json!({ "type": "image", "text": "hi" });
    assert!(serde_json::from_value::<pi_rs_ai_types::TextOrImageContent>(bad_block).is_err());
}

/// Standalone `ToolCall` (as carried by `toolcall_end`) keeps its literal tag.
#[test]
fn tool_call_carries_type_tag() {
    let tool_call = ToolCall {
        id: "tc_1".into(),
        name: "bash".into(),
        ..Default::default()
    };
    let value = serde_json::to_value(&tool_call).unwrap();
    assert_eq!(value["type"], "toolCall");
}

/// Typed compat decoding on demand (the spec's api-conditional `compat`).
#[test]
fn model_compat_decodes_typed() {
    let model: Model = roundtrip("model_anthropic_opus47.json");
    let compat: pi_rs_ai_types::AnthropicMessagesCompat = model.compat().unwrap().unwrap();
    assert_eq!(compat.force_adaptive_thinking, Some(true));
    assert_eq!(compat.supports_temperature, Some(false));

    let model: Model = roundtrip("model_antling_ring.json");
    let compat: pi_rs_ai_types::OpenAICompletionsCompat = model.compat().unwrap().unwrap();
    assert_eq!(
        compat.max_tokens_field,
        Some(pi_rs_ai_types::MaxTokensField::MaxTokens)
    );
    assert_eq!(
        compat.thinking_format,
        Some(pi_rs_ai_types::ThinkingFormat::AntLing)
    );

    let model: Model = roundtrip("model_bedrock_nova2lite.json");
    assert!(
        model
            .compat::<pi_rs_ai_types::AnthropicMessagesCompat>()
            .unwrap()
            .is_none()
    );
}
