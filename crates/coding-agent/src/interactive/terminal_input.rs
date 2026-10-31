use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use pi_extensions::EVENT_BUDGET;
use pi_tui::tui::InputListenerResult;
use serde::Deserialize;

use super::hosted::ToHost;
use super::screen::Screen;
use crate::extensions::Live;

#[derive(Default, Deserialize)]
pub struct Verdict {
    #[serde(default)]
    consume: bool,
    data: Option<String>,
}

#[derive(Default)]
struct Queue {
    enabled: bool,
    pending: bool,
    items: VecDeque<String>,
}

struct Pending {
    sequence: u64,
    data: String,
    since: Instant,
}

pub struct TerminalInput {
    queue: Rc<RefCell<Queue>>,
    replaying: Rc<Cell<bool>>,
    deferred: Box<dyn Fn() -> bool>,
    pending: Option<Pending>,
    sequence: u64,
}

impl TerminalInput {
    pub fn install(tui: &mut Screen, deferred: impl Fn() -> bool + 'static) -> Self {
        let queue = Rc::new(RefCell::new(Queue::default()));
        let replaying = Rc::new(Cell::new(false));
        let (queued, replay) = (queue.clone(), replaying.clone());
        tui.add_input_listener(Box::new(move |data| {
            if replay.get() {
                return None;
            }
            let mut queue = queued.borrow_mut();
            if !queue.enabled && !queue.pending && queue.items.is_empty() {
                return None;
            }
            queue.items.push_back(data.to_owned());
            Some(InputListenerResult {
                consume: true,
                data: None,
            })
        }));
        Self {
            queue,
            replaying,
            deferred: Box::new(deferred),
            pending: None,
            sequence: 0,
        }
    }

    pub fn waiting(&self) -> bool {
        self.pending.is_some() || !self.queue.borrow().items.is_empty()
    }

    pub fn enable(&mut self, enabled: bool) {
        self.queue.borrow_mut().enabled = enabled;
    }

    pub fn answer(&mut self, tui: &mut Screen, sequence: u64, verdict: Verdict) -> bool {
        let Some(pending) = self.pending.take_if(|pending| pending.sequence == sequence) else {
            return false;
        };
        self.queue.borrow_mut().pending = false;
        if verdict.consume {
            return false;
        }
        self.replay(tui, &verdict.data.unwrap_or(pending.data))
    }

    fn replay(&self, tui: &mut Screen, data: &str) -> bool {
        if data.is_empty() {
            return false;
        }
        self.replaying.set(true);
        let drew = tui.handle_terminal_input(data);
        self.replaying.set(false);
        drew
    }

    pub fn dispatch(&mut self, tui: &mut Screen, live: Option<&Live>) -> (bool, bool) {
        let timed_out = self
            .pending
            .as_ref()
            .filter(|pending| pending.since.elapsed() >= EVENT_BUDGET)
            .map(|pending| pending.sequence);
        let mut drew = false;
        if let Some(sequence) = timed_out {
            self.enable(false);
            drew |= self.answer(tui, sequence, Verdict::default());
        }
        if self.pending.is_some() {
            return (drew, timed_out.is_some());
        }
        while !(self.deferred)() {
            let data = self.queue.borrow_mut().items.pop_front();
            let Some(data) = data else {
                break;
            };
            if self.queue.borrow().enabled
                && let Some(live) = live
            {
                self.sequence += 1;
                live.ui(&ToHost::TerminalInput {
                    sequence: self.sequence,
                    data: &data,
                });
                self.queue.borrow_mut().pending = true;
                self.pending = Some(Pending {
                    sequence: self.sequence,
                    data,
                    since: Instant::now(),
                });
                break;
            }
            drew |= self.replay(tui, &data);
        }
        (drew, timed_out.is_some())
    }
}
