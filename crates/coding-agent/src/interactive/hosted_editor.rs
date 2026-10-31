use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use pi_extensions::EVENT_BUDGET;
use pi_tui::tui::{Component, ComponentRef, component};
use serde::Deserialize;

use super::hosted::{Hosted, ToHost};
use crate::extensions::Live;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditorAction {
    #[serde(rename = "app.interrupt")]
    Interrupt,
    #[serde(rename = "app.clear")]
    Clear,
    #[serde(rename = "app.exit")]
    Exit,
    #[serde(rename = "app.tools.expand")]
    Expand,
    #[serde(rename = "app.message.followUp")]
    FollowUp,
    #[serde(rename = "app.message.dequeue")]
    Dequeue,
    #[serde(rename = "app.thinking.toggle")]
    ThinkingToggle,
    #[serde(rename = "app.thinking.cycle")]
    ThinkingCycle,
    #[serde(rename = "app.model.cycleForward")]
    ModelForward,
    #[serde(rename = "app.model.cycleBackward")]
    ModelBackward,
    #[serde(rename = "app.model.select")]
    ModelSelect,
    #[serde(rename = "app.editor.external")]
    ExternalEditor,
    #[serde(rename = "app.suspend")]
    Suspend,
    #[serde(rename = "app.session.new")]
    SessionNew,
    #[serde(rename = "app.session.tree")]
    SessionTree,
    #[serde(rename = "app.session.fork")]
    SessionFork,
    #[serde(rename = "app.session.resume")]
    SessionResume,
}

struct EditorProxy {
    hosted: Rc<RefCell<Hosted>>,
    queue: Rc<RefCell<VecDeque<String>>>,
}

impl Component for EditorProxy {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.hosted.borrow_mut().render(width)
    }

    fn handle_input(&mut self, data: &str) {
        self.queue.borrow_mut().push_back(data.to_owned());
    }

    fn accepts_input(&self) -> bool {
        self.hosted.borrow().accepts_input()
    }

    fn wants_key_release(&self) -> bool {
        self.hosted.borrow().wants_key_release()
    }

    fn invalidate(&mut self) {
        self.hosted.borrow_mut().invalidate();
    }

    fn set_focused(&mut self, focused: bool) {
        self.hosted.borrow_mut().set_focused(focused);
    }

    fn is_focusable(&self) -> bool {
        self.hosted.borrow().is_focusable()
    }
}

pub enum Step {
    Idle,
    Sent,
    TimedOut,
}

pub struct HostedEditor {
    pub component: u64,
    pub shown: ComponentRef,
    queue: Rc<RefCell<VecDeque<String>>>,
    waiting: Option<(u64, Instant)>,
    sequence: u64,
    mirrored: String,
}

impl HostedEditor {
    pub fn new(id: u64, hosted: Rc<RefCell<Hosted>>, text: String) -> Self {
        let queue = Rc::new(RefCell::new(VecDeque::new()));
        let shown = component(EditorProxy {
            hosted,
            queue: queue.clone(),
        });
        HostedEditor {
            component: id,
            shown,
            queue,
            waiting: None,
            sequence: 0,
            mirrored: text,
        }
    }

    pub fn accept(&mut self, local: &str, text: &str) -> bool {
        if local != self.mirrored {
            return false;
        }
        self.mirrored = text.to_owned();
        true
    }

    pub fn done(&mut self, sequence: u64) {
        if self.waiting.is_some_and(|(waiting, _)| waiting == sequence) {
            self.waiting = None;
        }
    }

    pub fn in_flight(&self) -> bool {
        self.waiting.is_some() || !self.queue.borrow().is_empty()
    }

    pub fn sync(&mut self, live: &Live, local: &str) {
        if local == self.mirrored {
            return;
        }
        self.mirrored = local.to_owned();
        live.ui(&ToHost::EditorSetText {
            component: self.component,
            text: local,
        });
    }

    pub fn step(&mut self, live: &Live, held: bool) -> Step {
        if let Some((_, since)) = self.waiting {
            if since.elapsed() < EVENT_BUDGET {
                return Step::Idle;
            }
            self.waiting = None;
            return Step::TimedOut;
        }
        if held {
            return Step::Idle;
        }
        let Some(data) = self.queue.borrow_mut().pop_front() else {
            return Step::Idle;
        };
        self.sequence += 1;
        live.ui(&ToHost::EditorInput {
            component: self.component,
            sequence: self.sequence,
            data: &data,
        });
        self.waiting = Some((self.sequence, Instant::now()));
        Step::Sent
    }

    pub fn leftover(&self) -> Vec<String> {
        self.queue.borrow_mut().drain(..).collect()
    }
}
