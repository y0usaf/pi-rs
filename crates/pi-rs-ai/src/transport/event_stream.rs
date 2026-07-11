//! Port of `utils/event-stream.ts` — the async event stream with a final
//! result, used by every stream function.
//!
//! Spec semantics preserved:
//! - `push` on a completed stream is dropped;
//! - a completing event resolves the final result *and* is still
//!   delivered to the consumer;
//! - `end` marks the stream done and wakes waiters;
//! - the final result may resolve while iteration is still draining.
//!
//! Divergence (mechanism, not behavior): the spec's `result()` promise
//! pends forever if the stream ends without a completing event; here
//! [`EventStream::result`] resolves `None` in that case so no consumer
//! can hang. Every spec stream function settles the result before
//! `end()`, so the practiced paths are identical.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use pi_rs_ai_types::{AssistantMessage, AssistantMessageEvent};
use tokio::sync::{Notify, watch};

struct State<T> {
    queue: VecDeque<T>,
    done: bool,
}

enum ResultSlot<R> {
    Pending,
    Settled(Option<R>),
}

type CompleteFn<T, R> = Arc<dyn Fn(&T) -> Option<R> + Send + Sync>;

/// Spec: `EventStream<T, R>`. Cloneable; producers `push`/`end`,
/// consumers `next`/`result` — same handle, as in the spec's single
/// object.
pub struct EventStream<T, R> {
    state: Arc<Mutex<State<T>>>,
    notify: Arc<Notify>,
    complete: CompleteFn<T, R>,
    result_tx: Arc<watch::Sender<ResultSlot<R>>>,
}

impl<T, R> Clone for EventStream<T, R> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            notify: Arc::clone(&self.notify),
            complete: Arc::clone(&self.complete),
            result_tx: Arc::clone(&self.result_tx),
        }
    }
}

impl<T, R: Clone> EventStream<T, R> {
    /// Spec: `new EventStream(isComplete, extractResult)` — merged into
    /// one closure returning `Some(result)` when the event completes the
    /// stream.
    pub fn new(complete: impl Fn(&T) -> Option<R> + Send + Sync + 'static) -> Self {
        let (result_tx, _rx) = watch::channel(ResultSlot::Pending);
        Self {
            state: Arc::new(Mutex::new(State {
                queue: VecDeque::new(),
                done: false,
            })),
            notify: Arc::new(Notify::new()),
            complete: Arc::new(complete),
            result_tx: Arc::new(result_tx),
        }
    }

    /// Spec: `push(event)`.
    pub fn push(&self, event: T) {
        {
            let mut state = self.lock_state();
            if state.done {
                return;
            }
            if let Some(result) = (self.complete)(&event) {
                state.done = true;
                self.settle(Some(result));
            }
            state.queue.push_back(event);
        }
        self.notify.notify_one();
    }

    /// Spec: `end()` (no result argument).
    pub fn end(&self) {
        self.finish(None);
    }

    /// Spec: `end(result)`.
    pub fn end_with(&self, result: R) {
        self.finish(Some(result));
    }

    fn finish(&self, result: Option<R>) {
        {
            let mut state = self.lock_state();
            state.done = true;
        }
        // Settling with `None` (when no completing event arrived) is the
        // documented divergence: resolve rather than hang.
        self.settle(result);
        // Spec wakes every waiting consumer.
        self.notify.notify_waiters();
        self.notify.notify_one();
    }

    /// Spec: the `Symbol.asyncIterator` loop — queued events first, then
    /// `None` once done.
    pub async fn next(&self) -> Option<T> {
        loop {
            let notified = self.notify.notified();
            {
                let mut state = self.lock_state();
                if let Some(event) = state.queue.pop_front() {
                    return Some(event);
                }
                if state.done {
                    return None;
                }
            }
            notified.await;
        }
    }

    /// Spec: `result()` — resolves with the final result. `None` only if
    /// the stream ended without one (see module divergence note).
    pub async fn result(&self) -> Option<R> {
        let mut rx = self.result_tx.subscribe();
        match rx
            .wait_for(|slot| matches!(slot, ResultSlot::Settled(_)))
            .await
        {
            Ok(slot) => match &*slot {
                ResultSlot::Settled(result) => result.clone(),
                ResultSlot::Pending => None,
            },
            // The sender lives in `self`; the channel cannot close.
            Err(_) => None,
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// First settle wins, matching a promise's resolve-once semantics.
    fn settle(&self, result: Option<R>) {
        let mut result = Some(result);
        self.result_tx.send_if_modified(|slot| {
            if matches!(slot, ResultSlot::Pending)
                && let Some(result) = result.take()
            {
                *slot = ResultSlot::Settled(result);
                return true;
            }
            false
        });
    }
}

/// Spec: `AssistantMessageEventStream` — completes on `done` (final
/// message) or `error` (error message).
pub type AssistantMessageEventStream = EventStream<AssistantMessageEvent, AssistantMessage>;

/// Spec: `createAssistantMessageEventStream()` (the extension-surface
/// factory; exposed to Lua when the provider binding lands, WS2.4).
pub fn create_assistant_message_event_stream() -> AssistantMessageEventStream {
    EventStream::new(|event| match event {
        AssistantMessageEvent::Done { message, .. } => Some(message.clone()),
        AssistantMessageEvent::Error { error, .. } => Some(error.clone()),
        _ => None,
    })
}
