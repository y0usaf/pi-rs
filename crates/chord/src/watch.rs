use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use pi_rt::cancel::CancelToken;

use crate::delta::Op;
use crate::error::Error;
use crate::json::{JsonObject, JsonValue};

const LOCK: &str = "watch lock poisoned";
const MAX_PENDING_FRAMES: usize = 100;

pub type WatchError = Arc<dyn std::error::Error + Send + Sync>;

pub type Detach = Box<dyn FnOnce() + Send>;

pub type FrameListener<T> =
    Box<dyn FnMut(Option<&T>, &[Op], &CancelToken) -> Result<(), WatchError> + Send>;

pub type ValueListener<T> =
    Box<dyn FnMut(Option<&T>, &CancelToken) -> Result<(), WatchError> + Send>;

pub type Overflow<T> = Arc<dyn Fn(&T) -> Result<(T, Vec<Op>), WatchError> + Send + Sync>;

#[derive(Clone)]
pub enum WatchEnd {
    Stopped,
    Cancelled,
    Ended,
    Retired,
    ListenerError(WatchError),
}

impl WatchEnd {
    pub fn reason(&self) -> &'static str {
        match self {
            WatchEnd::Stopped => "stopped",
            WatchEnd::Cancelled => "cancelled",
            WatchEnd::Ended => "ended",
            WatchEnd::Retired => "retired",
            WatchEnd::ListenerError(_) => "listener_error",
        }
    }

    pub fn error(&self) -> Option<&WatchError> {
        match self {
            WatchEnd::ListenerError(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Debug for WatchEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.error() {
            Some(error) => write!(f, "listener_error({error})"),
            None => f.write_str(self.reason()),
        }
    }
}

pub trait WatchHandle<T> {
    fn value(&self) -> Option<T>;
    fn start(&mut self, listener: FrameListener<T>) -> Result<(), Error>;
    fn stop(&mut self) -> WatchEnd;
    fn closed(&self) -> Option<WatchEnd>;
}

pub trait NativeWatchHandle<T> {
    fn value(&self) -> Option<T>;
    fn start(&mut self, listener: ValueListener<T>) -> Result<(), Error>;
    fn stop(&mut self) -> WatchEnd;
    fn closed(&self) -> Option<WatchEnd>;
}

pub fn retirement_operations() -> Vec<Op> {
    vec![Op::Replace(JsonValue::Null)]
}

#[derive(Default)]
pub struct Deliveries {
    queue: Mutex<Queue>,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Box<dyn FnOnce() + Send>>,
    draining: bool,
}

impl Deliveries {
    pub fn push(&self, job: impl FnOnce() + Send + 'static) {
        self.queue.lock().expect(LOCK).jobs.push_back(Box::new(job));
    }

    pub fn drain(&self) {
        {
            let mut queue = self.queue.lock().expect(LOCK);
            if queue.draining {
                return;
            }
            queue.draining = true;
        }
        loop {
            let job = {
                let mut queue = self.queue.lock().expect(LOCK);
                match queue.jobs.pop_front() {
                    Some(job) => job,
                    None => {
                        queue.draining = false;
                        return;
                    }
                }
            };
            job();
        }
    }
}

struct Frame<T> {
    value: Option<T>,
    ops: Vec<Op>,
}

struct Core<T> {
    value: Option<T>,
    pending: VecDeque<Frame<T>>,
    started: bool,
    retired: bool,
    listener: Option<FrameListener<T>>,
    end: Option<WatchEnd>,
    detach: Option<Detach>,
}

struct Inner<T> {
    core: Mutex<Core<T>>,
    deliveries: Arc<Deliveries>,
    overflow: Overflow<T>,
}

impl<T: Clone + Send + 'static> Inner<T> {
    fn new(value: Option<T>, deliveries: Arc<Deliveries>, overflow: Overflow<T>) -> Inner<T> {
        Inner {
            core: Mutex::new(Core {
                value,
                pending: VecDeque::new(),
                started: false,
                retired: false,
                listener: None,
                end: None,
                detach: None,
            }),
            deliveries,
            overflow,
        }
    }

    fn set_detach(&self, detach: Detach) {
        self.core.lock().expect(LOCK).detach = Some(detach);
    }

    fn advance(self: &Arc<Self>, value: Option<T>, ops: Vec<Op>) {
        let mut core = self.core.lock().expect(LOCK);
        if core.end.is_some() || core.retired {
            return;
        }
        core.retired = value.is_none();
        let failure = match core.pending.len() >= MAX_PENDING_FRAMES {
            false => {
                core.pending.push_back(Frame { value, ops });
                None
            }
            true => match value.as_ref().map(|value| (self.overflow)(value)) {
                None => {
                    core.pending.clear();
                    core.pending.push_back(Frame {
                        value: None,
                        ops: retirement_operations(),
                    });
                    None
                }
                Some(Ok((value, ops))) => {
                    core.pending.clear();
                    core.pending.push_back(Frame {
                        value: Some(value),
                        ops,
                    });
                    None
                }
                Some(Err(error)) => Some(error),
            },
        };
        let started = core.started;
        drop(core);
        if let Some(error) = failure {
            self.terminate(WatchEnd::ListenerError(error));
            return;
        }
        if started {
            self.schedule();
        }
    }

    fn schedule(self: &Arc<Self>) {
        let shared = Arc::clone(self);
        self.deliveries.push(move || shared.drain());
    }

    fn start(self: &Arc<Self>, listener: FrameListener<T>) -> Result<(), Error> {
        let mut core = self.core.lock().expect(LOCK);
        if core.started {
            return Err(Error::State("Watch is already started".into()));
        }
        if core.end.is_some() {
            return Err(Error::State("Watch is stopped".into()));
        }
        core.started = true;
        core.listener = Some(listener);
        drop(core);
        self.schedule();
        Ok(())
    }

    fn drain(&self) {
        loop {
            let (value, ops, mut listener) = {
                let mut core = self.core.lock().expect(LOCK);
                if core.end.is_some() {
                    return;
                }
                let Some(listener) = core.listener.take() else {
                    return;
                };
                let Some(frame) = core.pending.pop_front() else {
                    core.listener = Some(listener);
                    return;
                };
                core.value = frame.value.clone();
                (frame.value, frame.ops, listener)
            };
            let retired = value.is_none();
            if let Err(error) = listener(value.as_ref(), &ops, &CancelToken::new()) {
                self.terminate(WatchEnd::ListenerError(error));
                return;
            }
            if retired {
                self.terminate(WatchEnd::Retired);
                return;
            }
            let mut core = self.core.lock().expect(LOCK);
            if core.end.is_some() {
                return;
            }
            core.listener = Some(listener);
        }
    }

    fn terminate(&self, end: WatchEnd) -> WatchEnd {
        let (end, detach) = {
            let mut core = self.core.lock().expect(LOCK);
            if let Some(existing) = &core.end {
                return existing.clone();
            }
            core.end = Some(end.clone());
            core.pending.clear();
            core.listener = None;
            (end, core.detach.take())
        };
        if let Some(detach) = detach {
            detach();
        }
        end
    }

    fn observe_cancellation(self: &Arc<Self>, cancel: &CancelToken) {
        if self.core.lock().expect(LOCK).end.is_some() {
            return;
        }
        let shared = Arc::clone(self);
        cancel.on_cancel(move || {
            shared.terminate(WatchEnd::Cancelled);
        });
    }

    fn value(&self) -> Option<T> {
        self.core.lock().expect(LOCK).value.clone()
    }

    fn closed(&self) -> Option<WatchEnd> {
        self.core.lock().expect(LOCK).end.clone()
    }

    fn ended(&self) -> bool {
        self.core.lock().expect(LOCK).end.is_some()
    }
}

pub struct Watch<T>(Arc<Inner<T>>);

impl<T: Clone + Send + 'static> Clone for Watch<T> {
    fn clone(&self) -> Self {
        Watch(Arc::clone(&self.0))
    }
}

impl<T: Clone + Send + 'static> Watch<T> {
    pub fn new(value: Option<T>, deliveries: Arc<Deliveries>, overflow: Overflow<T>) -> Watch<T> {
        Watch(Arc::new(Inner::new(value, deliveries, overflow)))
    }

    pub fn set_detach(&self, detach: Detach) {
        self.0.set_detach(detach);
    }

    pub fn advance(&self, value: T, ops: Vec<Op>) {
        self.0.advance(Some(value), ops);
    }

    fn advance_optional(&self, value: Option<T>, ops: Vec<Op>) {
        self.0.advance(value, ops);
    }

    pub fn retire(&self) {
        self.0.advance(None, retirement_operations());
    }

    pub fn terminate(&self, end: WatchEnd) -> WatchEnd {
        self.0.terminate(end)
    }

    pub fn observe_cancellation(&self, cancel: &CancelToken) {
        self.0.observe_cancellation(cancel);
    }

    pub fn value(&self) -> Option<T> {
        self.0.value()
    }

    pub fn closed(&self) -> Option<WatchEnd> {
        self.0.closed()
    }

    pub fn ended(&self) -> bool {
        self.0.ended()
    }

    pub fn start(&self, listener: FrameListener<T>) -> Result<(), Error> {
        self.0.start(listener)
    }
}

impl WatchHandle<JsonObject> for Watch<JsonObject> {
    fn value(&self) -> Option<JsonObject> {
        Watch::value(self)
    }

    fn start(&mut self, listener: FrameListener<JsonObject>) -> Result<(), Error> {
        Watch::start(self, listener)
    }

    fn stop(&mut self) -> WatchEnd {
        self.terminate(WatchEnd::Stopped)
    }

    fn closed(&self) -> Option<WatchEnd> {
        Watch::closed(self)
    }
}

pub struct NativeWatch<T>(Watch<T>);

impl<T: Clone + Send + 'static> Clone for NativeWatch<T> {
    fn clone(&self) -> Self {
        NativeWatch(self.0.clone())
    }
}

impl<T: Clone + Send + 'static> NativeWatch<T> {
    pub fn new(
        value: Option<T>,
        deliveries: Arc<Deliveries>,
        overflow: Overflow<T>,
    ) -> NativeWatch<T> {
        NativeWatch(Watch::new(value, deliveries, overflow))
    }

    pub fn set_detach(&self, detach: Detach) {
        self.0.set_detach(detach);
    }

    pub fn advance(&self, value: Option<T>) {
        self.0.advance_optional(value, Vec::new());
    }

    pub fn terminate(&self, end: WatchEnd) -> WatchEnd {
        self.0.terminate(end)
    }

    pub fn observe_cancellation(&self, cancel: &CancelToken) {
        self.0.observe_cancellation(cancel);
    }

    pub fn value(&self) -> Option<T> {
        self.0.value()
    }

    pub fn closed(&self) -> Option<WatchEnd> {
        self.0.closed()
    }

    pub fn ended(&self) -> bool {
        self.0.ended()
    }

    pub fn start(&self, listener: ValueListener<T>) -> Result<(), Error> {
        let mut listener = listener;
        self.0
            .start(Box::new(move |value, _ops, cancel| listener(value, cancel)))
    }
}

impl<T: Clone + Send + 'static> NativeWatchHandle<T> for NativeWatch<T> {
    fn value(&self) -> Option<T> {
        NativeWatch::value(self)
    }

    fn start(&mut self, listener: ValueListener<T>) -> Result<(), Error> {
        NativeWatch::start(self, listener)
    }

    fn stop(&mut self) -> WatchEnd {
        self.terminate(WatchEnd::Stopped)
    }

    fn closed(&self) -> Option<WatchEnd> {
        NativeWatch::closed(self)
    }
}
