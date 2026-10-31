//! `faux-provider.test.ts`: the faux provider's simulated prompt cache.

use std::sync::Arc;

use pi_ai::providers::faux::{Faux, FauxOptions, FauxResponse};
use pi_ai::types::{
    AssistantBlock, AssistantMessage, AssistantRole, CacheRetention, Cost, Message, Model,
    SimpleStreamOptions, StopReason, TextContent, TextType, TranscriptContext, Usage, UserContent,
    UserMessage, UserRole,
};

fn assistant(text: &str) -> Box<AssistantMessage> {
    Box::new(AssistantMessage {
        role: AssistantRole,
        content: vec![AssistantBlock::Text(TextContent {
            kind: TextType,
            text: text.to_owned(),
            text_signature: None,
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
        stop_reason: StopReason::Stop,
        deferred: None,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        duration_ms: None,
        timestamp: 1,
    })
}

fn user(text: &str) -> Message {
    Message::User(Arc::new(UserMessage {
        role: UserRole,
        content: UserContent::Text(text.to_owned()),
        timestamp: 1,
    }))
}

fn complete(faux: &Faux, model: &Model, messages: Vec<Message>) -> AssistantMessage {
    let options = SimpleStreamOptions {
        session_id: Some("session-1".to_owned()),
        cache_retention: Some(CacheRetention::Short),
        ..SimpleStreamOptions::default()
    };
    let stream = faux.stream(model, TranscriptContext { messages }, options);
    while stream.next_event().is_some() {}
    stream
        .result()
        .expect("the faux stream ends with a message")
}

#[test]
fn counts_cached_characters_up_to_the_first_difference() {
    let faux = Faux::new(FauxOptions::default());
    let model = faux.default_model();
    faux.set_responses(vec![
        FauxResponse::Message(assistant("a")),
        FauxResponse::Message(assistant("b")),
        FauxResponse::Message(assistant("c")),
    ]);

    let first = complete(&faux, &model, vec![user("hello world")]);
    assert_eq!(
        (
            first.usage.input,
            first.usage.cache_read,
            first.usage.cache_write
        ),
        (0, 0, 4),
        "first prompt: {:?}",
        first.usage
    );

    let extended = complete(&faux, &model, vec![user("hello world"), user("next")]);
    assert_eq!(
        (
            extended.usage.input,
            extended.usage.cache_read,
            extended.usage.cache_write
        ),
        (0, 4, 3),
        "extended prompt: {:?}",
        extended.usage
    );

    let edited = complete(&faux, &model, vec![user("hello wide"), user("next")]);
    assert_eq!(
        (
            edited.usage.input,
            edited.usage.cache_read,
            edited.usage.cache_write
        ),
        (0, 3, 4),
        "edited prompt: {:?}",
        edited.usage
    );
}
