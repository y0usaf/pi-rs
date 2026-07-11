//! Behavioral parity tests for `transport::event_stream` against the
//! spec's `utils/event-stream.ts`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::transport::EventStream;
use pi_rs_ai::transport::create_assistant_message_event_stream;
use pi_rs_ai_types::{
    AssistantMessage, AssistantMessageEvent, AssistantRole, StopReason, Usage, now_ms,
};

fn message(stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![],
        api: "anthropic-messages".to_string(),
        provider: "anthropic".to_string(),
        model: "test-model".to_string(),
        response_model: None,
        response_id: None,
        diagnostics: None,
        usage: Usage::default(),
        stop_reason,
        error_message: None,
        timestamp: now_ms(),
    }
}

/// Never-completing stream: events queue and drain in order.
fn plain_stream() -> EventStream<i32, i32> {
    EventStream::new(|_| None)
}

#[tokio::test]
async fn queued_events_drain_in_order_then_end() {
    let stream = plain_stream();
    stream.push(1);
    stream.push(2);
    stream.push(3);
    stream.end();
    assert_eq!(stream.next().await, Some(1));
    assert_eq!(stream.next().await, Some(2));
    assert_eq!(stream.next().await, Some(3));
    assert_eq!(stream.next().await, None);
    // Iteration past done stays done (spec: iterator returns).
    assert_eq!(stream.next().await, None);
}

#[tokio::test]
async fn waiting_consumer_is_woken_by_push_and_end() {
    let stream = plain_stream();
    let consumer = {
        let stream = stream.clone();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(event) = stream.next().await {
                seen.push(event);
            }
            seen
        })
    };
    // Let the consumer park.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    stream.push(7);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    stream.end();
    assert_eq!(consumer.await.unwrap(), vec![7]);
}

#[tokio::test]
async fn completing_event_resolves_result_and_is_still_delivered() {
    // Spec: isComplete/extractResult — completing event sets the final
    // result, is delivered, and later pushes are dropped.
    let stream: EventStream<i32, i32> = EventStream::new(|e| (*e >= 100).then_some(*e));
    stream.push(1);
    stream.push(100);
    stream.push(2); // dropped: pushed after completion
    assert_eq!(stream.next().await, Some(1));
    assert_eq!(stream.next().await, Some(100));
    assert_eq!(stream.next().await, None);
    assert_eq!(stream.result().await, Some(100));
}

#[tokio::test]
async fn result_resolves_for_pending_waiter() {
    let stream: EventStream<i32, i32> = EventStream::new(|e| (*e >= 100).then_some(*e));
    let waiter = {
        let stream = stream.clone();
        tokio::spawn(async move { stream.result().await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    stream.push(100);
    assert_eq!(waiter.await.unwrap(), Some(100));
}

#[tokio::test]
async fn end_with_explicit_result() {
    let stream = plain_stream();
    stream.end_with(42);
    assert_eq!(stream.result().await, Some(42));
    assert_eq!(stream.next().await, None);
}

#[tokio::test]
async fn first_settle_wins() {
    // Spec: a promise resolves once — end(result) after a completing
    // event must not overwrite.
    let stream: EventStream<i32, i32> = EventStream::new(|e| (*e >= 100).then_some(*e));
    stream.push(100);
    stream.end_with(999);
    assert_eq!(stream.result().await, Some(100));
}

#[tokio::test]
async fn end_without_result_resolves_none() {
    // Documented divergence: the spec's result() would pend forever here;
    // pi-rs resolves None so no consumer can hang.
    let stream = plain_stream();
    stream.end();
    assert_eq!(stream.result().await, None);
}

#[tokio::test]
async fn assistant_stream_completes_on_done_and_error() {
    // done → final message.
    let stream = create_assistant_message_event_stream();
    let done_message = message(StopReason::Stop);
    stream.push(AssistantMessageEvent::Start {
        partial: message(StopReason::Stop),
    });
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message: done_message.clone(),
    });
    stream.end();
    assert_eq!(stream.result().await, Some(done_message));

    // error → the error message is the final result (spec extractResult).
    let stream = create_assistant_message_event_stream();
    let mut error_message = message(StopReason::Error);
    error_message.error_message = Some("boom".to_string());
    stream.push(AssistantMessageEvent::Error {
        reason: StopReason::Error,
        error: error_message.clone(),
    });
    stream.end();
    assert_eq!(stream.result().await, Some(error_message));
}
