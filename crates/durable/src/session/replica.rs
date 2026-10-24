use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::cancel::CancelToken;
use crate::chord::delta::Op;
use crate::chord::state::{Attached, Handle, Listener};
use crate::chord::watch::{Deliveries, Detach, Watch as ChordWatch, WatchEnd, WatchError};
use crate::types::{JsonObject, JsonValue};

const LOCK: &str = "session replica lock poisoned";

pub fn value_json(value: &Option<JsonObject>) -> JsonValue {
    match value {
        Some(object) => JsonValue::Object(object.clone()),
        None => JsonValue::Null,
    }
}

pub struct AttachedState<G> {
    attached: Arc<Mutex<Attached>>,
    guard: Option<G>,
}

impl<G> AttachedState<G> {
    pub fn new(attached: Arc<Mutex<Attached>>, guard: G) -> Self {
        Self {
            attached,
            guard: Some(guard),
        }
    }

    pub fn value(&self) -> JsonValue {
        self.attached.lock().expect(LOCK).value().clone()
    }

    pub fn subscribe(&self, cancel: &CancelToken, listener: Listener) -> Handle {
        self.attached
            .lock()
            .expect(LOCK)
            .subscribe(cancel, listener)
    }

    pub fn unsubscribe(&self, handle: Handle) {
        self.attached.lock().expect(LOCK).unsubscribe(handle);
    }

    pub fn dispose(&mut self) {
        self.guard.take();
    }
}

pub fn feeder(attached: Arc<Mutex<Attached>>) -> impl Fn(&JsonValue, &[Op]) + Send + Sync {
    let sequence = AtomicU64::new(0);
    move |value, ops| {
        let cursor = sequence.fetch_add(1, Ordering::SeqCst) + 1;
        attached
            .lock()
            .expect(LOCK)
            .receive(cursor, value.clone(), ops, &CancelToken::new())
            .expect("state cursors are contiguous by construction");
    }
}

#[derive(Clone)]
pub struct DocumentState(Arc<Mutex<Attached>>);

impl DocumentState {
    pub fn value(&self) -> JsonValue {
        self.0.lock().expect(LOCK).value().clone()
    }

    pub fn subscribe(&self, cancel: &CancelToken, listener: Listener) -> Handle {
        self.0.lock().expect(LOCK).subscribe(cancel, listener)
    }

    pub fn unsubscribe(&self, handle: Handle) {
        self.0.lock().expect(LOCK).unsubscribe(handle);
    }
}

pub struct StateSource {
    attached: Arc<Mutex<Attached>>,
    deliveries: Arc<Deliveries>,
    cursor: u64,
    retired: bool,
    closed: bool,
    detach: Option<Detach>,
}

impl StateSource {
    pub fn new(
        value: Option<JsonObject>,
        attached: Attached,
        deliveries: Arc<Deliveries>,
    ) -> (DocumentState, Arc<Mutex<StateSource>>) {
        let retired = value.is_none();
        let attached = Arc::new(Mutex::new(attached));
        let source = Arc::new(Mutex::new(StateSource {
            attached: Arc::clone(&attached),
            deliveries,
            cursor: 0,
            retired,
            closed: false,
            detach: None,
        }));
        (DocumentState(attached), source)
    }

    pub fn set_detach(&mut self, detach: Detach) {
        self.detach = Some(detach);
    }

    fn advance(&mut self, value: Option<JsonObject>, ops: Vec<Op>) {
        if self.closed || self.retired {
            return;
        }
        self.cursor += 1;
        self.retired = value.is_none();
        let attached = Arc::clone(&self.attached);
        let cursor = self.cursor;
        let value = value_json(&value);
        self.deliveries.push(move || {
            attached
                .lock()
                .expect(LOCK)
                .receive(cursor, value, &ops, &CancelToken::new())
                .expect("state cursors are contiguous by construction");
        });
    }

    fn close_session(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(detach) = self.detach.take() {
            detach();
        }
    }
}

#[derive(Clone)]
pub enum Sink {
    State(Arc<Mutex<StateSource>>),
    Watch(ChordWatch<JsonObject>),
}

impl Sink {
    pub fn advance(&self, value: Option<JsonObject>, ops: Vec<Op>) {
        match self {
            Sink::State(source) => source.lock().expect(LOCK).advance(value, ops),
            Sink::Watch(watch) => match value {
                Some(value) => watch.advance(value, ops),
                None => watch.retire(),
            },
        }
    }

    pub fn close_session(&self) {
        match self {
            Sink::State(source) => source.lock().expect(LOCK).close_session(),
            Sink::Watch(watch) => {
                watch.terminate(WatchEnd::Ended);
            }
        }
    }
}

pub fn observed_operations(
    observed: &mut u64,
    value: &Option<JsonObject>,
    version: Option<u64>,
    ops: &[Op],
) -> Vec<Op> {
    match (value, version) {
        (None, _) => crate::chord::watch::retirement_operations(),
        (Some(_), Some(version)) if version == *observed => ops.to_vec(),
        (Some(object), Some(version)) => {
            *observed = version;
            vec![Op::Replace(JsonValue::Object(object.clone()))]
        }
        (Some(_), None) => ops.to_vec(),
    }
}

pub fn watch_error(error: impl std::error::Error + Send + Sync + 'static) -> WatchError {
    Arc::new(error)
}
