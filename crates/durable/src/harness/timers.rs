use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::cancel::CancelToken;
use crate::errors::Error;
use crate::turn::pause;

/// Longest delay a host timer may take at once; a longer one is waited for in several steps.
pub const MAX_TIMER_DELAY: u64 = 2_147_483_647;

pub type TimerCallback = Arc<dyn Fn() + Send + Sync>;
pub type Disarm = Box<dyn Fn() + Send + Sync>;

/// A host timer, given as `HarnessOptions.set_timer`: call the callback once, no sooner than the
/// delay, and return a function that disarms it. `background` timers only free memory, and a host
/// may decline one by returning `None`. Other timers pace or resume work: one the host declines is
/// armed on a thread instead.
pub type SetTimer = Arc<dyn Fn(TimerCallback, u64, bool) -> Option<Disarm> + Send + Sync>;

pub type Report = Arc<dyn Fn(Error) + Send + Sync>;

/// The default `HarnessOptions.set_timer`: a thread that waits, which never keeps a Rust process
/// alive once `main` returns.
pub fn native_timer(callback: TimerCallback, delay_ms: u64, _background: bool) -> Option<Disarm> {
    let token = CancelToken::new();
    let cancel = token.clone();
    thread::spawn(move || {
        let mut remaining = delay_ms;
        loop {
            let step = remaining.min(MAX_TIMER_DELAY);
            if pause(|| token.wait_timeout(Duration::from_millis(step))) {
                return;
            }
            remaining -= step;
            if remaining == 0 {
                break;
            }
        }
        callback();
    });
    Some(Box::new(move || cancel.cancel()))
}

/// The timers of the scheduler and its tasks, armed through the host's `set_timer`.
#[derive(Clone)]
pub struct Timers {
    arm: SetTimer,
    report: Report,
    reported: Arc<AtomicBool>,
}

impl Timers {
    pub fn new(set_timer: Option<SetTimer>, report: Report) -> Self {
        Self {
            arm: set_timer.unwrap_or_else(|| Arc::new(native_timer)),
            report,
            reported: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A timer that paces or resumes work; when the host declines it, a thread arms it instead and
    /// the first such decline is reported.
    pub fn foreground(&self, callback: TimerCallback, delay_ms: u64) -> Disarm {
        if let Some(disarm) = (self.arm)(Arc::clone(&callback), delay_ms, false) {
            return disarm;
        }
        if !self.reported.swap(true, Ordering::SeqCst) {
            (self.report)(Error::Invalid(
                "HarnessOptions.set_timer declined a timer that paces work; using a thread".into(),
            ));
        }
        native_timer(callback, delay_ms, false).unwrap_or_else(|| Box::new(|| {}))
    }

    /// A timer that only frees memory; `None` when the host declines it.
    pub fn background(&self, callback: TimerCallback, delay_ms: u64) -> Option<Disarm> {
        (self.arm)(callback, delay_ms, true)
    }
}
