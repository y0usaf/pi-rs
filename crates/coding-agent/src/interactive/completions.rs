use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use pi_extensions::EVENT_BUDGET;
use pi_tui::autocomplete::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
    Completion, Suggesting,
};
use pi_tui::components::editor::Editor;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::hosted::ToHost;
use crate::extensions::Live;

#[derive(Deserialize, Serialize)]
pub struct ItemWire {
    value: String,
    #[serde(default)]
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl From<ItemWire> for AutocompleteItem {
    fn from(item: ItemWire) -> Self {
        AutocompleteItem {
            value: item.value,
            label: item.label,
            description: item.description,
        }
    }
}

impl From<&AutocompleteItem> for ItemWire {
    fn from(item: &AutocompleteItem) -> Self {
        ItemWire {
            value: item.value.clone(),
            label: item.label.clone(),
            description: item.description.clone(),
        }
    }
}

#[derive(Deserialize)]
pub struct SuggestionsWire {
    items: Vec<ItemWire>,
    prefix: String,
}

impl From<SuggestionsWire> for AutocompleteSuggestions {
    fn from(suggestions: SuggestionsWire) -> Self {
        AutocompleteSuggestions {
            items: suggestions.items.into_iter().map(Into::into).collect(),
            prefix: suggestions.prefix,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionWire {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
}

impl From<CompletionWire> for Completion {
    fn from(completion: CompletionWire) -> Self {
        let cursor_col = byte_col(
            &completion.lines,
            completion.cursor_line,
            completion.cursor_col,
        );
        Completion {
            lines: completion.lines,
            cursor_line: completion.cursor_line,
            cursor_col,
        }
    }
}

pub fn utf16_col(lines: &[String], line: usize, byte: usize) -> usize {
    lines.get(line).map_or(0, |text| {
        text.char_indices()
            .take_while(|(at, _)| *at < byte)
            .map(|(_, c)| c.len_utf16())
            .sum()
    })
}

pub fn byte_col(lines: &[String], line: usize, utf16: usize) -> usize {
    let Some(text) = lines.get(line) else {
        return 0;
    };
    let mut units = 0;
    for (at, c) in text.char_indices() {
        if units >= utf16 {
            return at;
        }
        units += c.len_utf16();
    }
    text.len()
}

#[derive(Deserialize)]
#[serde(tag = "method")]
pub enum CompletionShown {
    #[serde(rename = "autocompleteChain", rename_all = "camelCase")]
    Chain {
        active: bool,
        #[serde(default)]
        trigger_characters: Vec<String>,
        #[serde(default)]
        should_trigger: bool,
    },
    #[serde(rename = "autocompleteSuggested")]
    Suggested {
        request: u64,
        #[serde(default)]
        suggestions: Option<SuggestionsWire>,
    },
    #[serde(rename = "autocompleteApplied")]
    Applied {
        request: u64,
        #[serde(default)]
        completion: Option<CompletionWire>,
    },
    #[serde(rename = "autocompleteTriggered")]
    Triggered { request: u64, trigger: bool },
}

impl From<CompletionShown> for Completing {
    fn from(shown: CompletionShown) -> Self {
        match shown {
            CompletionShown::Chain {
                active,
                trigger_characters,
                should_trigger,
            } => Completing::Chain(active.then_some(Chain {
                triggers: trigger_characters,
                should_trigger,
            })),
            CompletionShown::Suggested {
                request,
                suggestions,
            } => Completing::Suggested {
                request,
                suggestions: suggestions.map(Into::into),
            },
            CompletionShown::Applied {
                request,
                completion,
            } => Completing::Applied {
                request,
                completion: completion.map(Into::into),
            },
            CompletionShown::Triggered { request, trigger } => {
                Completing::Triggered { request, trigger }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "method")]
pub enum CompletionAsked {
    #[serde(rename = "autocompleteBaseSuggest", rename_all = "camelCase")]
    Suggest {
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
        #[serde(default)]
        force: bool,
    },
    #[serde(rename = "autocompleteBaseApply", rename_all = "camelCase")]
    Apply {
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
        item: ItemWire,
        prefix: String,
    },
    #[serde(rename = "autocompleteBaseShouldTrigger", rename_all = "camelCase")]
    ShouldTrigger {
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
    },
}

impl CompletionAsked {
    pub fn answered_by(self, answer: Sender<Value>) -> Completing {
        Completing::Base(match self {
            CompletionAsked::Suggest {
                lines,
                cursor_line,
                cursor_col,
                force,
            } => BaseAsk::Suggest {
                lines,
                cursor_line,
                cursor_col,
                force,
                answer,
            },
            CompletionAsked::Apply {
                lines,
                cursor_line,
                cursor_col,
                item,
                prefix,
            } => BaseAsk::Apply {
                lines,
                cursor_line,
                cursor_col,
                item: item.into(),
                prefix,
                answer,
            },
            CompletionAsked::ShouldTrigger {
                lines,
                cursor_line,
                cursor_col,
            } => BaseAsk::ShouldTrigger {
                lines,
                cursor_line,
                cursor_col,
                answer,
            },
        })
    }
}

pub struct Chain {
    pub triggers: Vec<String>,
    pub should_trigger: bool,
}

pub enum BaseAsk {
    Suggest {
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
        answer: Sender<Value>,
    },
    Apply {
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
        item: AutocompleteItem,
        prefix: String,
        answer: Sender<Value>,
    },
    ShouldTrigger {
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
        answer: Sender<Value>,
    },
}

pub enum Completing {
    Base(BaseAsk),
    Chain(Option<Chain>),
    Suggested {
        request: u64,
        suggestions: Option<AutocompleteSuggestions>,
    },
    Applied {
        request: u64,
        completion: Option<Completion>,
    },
    Triggered {
        request: u64,
        trigger: bool,
    },
}

struct Inbox {
    receiver: Receiver<Completing>,
    held: RefCell<VecDeque<Completing>>,
    timed_out: Cell<bool>,
}

impl Inbox {
    fn next(&self) -> Option<Completing> {
        let held = self.held.borrow_mut().pop_front();
        held.or_else(|| self.receiver.try_recv().ok())
    }

    fn keep(&self, message: Completing, base: &CombinedAutocompleteProvider) {
        match message {
            Completing::Base(asked) => answer_base(asked, base),
            other => self.held.borrow_mut().push_back(other),
        }
    }
}

fn answer_base(asked: BaseAsk, base: &CombinedAutocompleteProvider) {
    match asked {
        BaseAsk::Suggest {
            lines,
            cursor_line,
            cursor_col,
            force,
            answer,
        } => {
            let col = byte_col(&lines, cursor_line, cursor_col);
            let suggestions = base.suggestions(&lines, cursor_line, col, force);
            let _ = answer.send(suggestions.map_or(Value::Null, |suggestions| {
                json!({
                    "items": suggestions.items.iter().map(ItemWire::from).collect::<Vec<_>>(),
                    "prefix": suggestions.prefix,
                })
            }));
        }
        BaseAsk::Apply {
            lines,
            cursor_line,
            cursor_col,
            item,
            prefix,
            answer,
        } => {
            let col = byte_col(&lines, cursor_line, cursor_col);
            let done = base.apply_completion(&lines, cursor_line, col, &item, &prefix);
            let _ = answer.send(json!({
                "lines": done.lines,
                "cursorLine": done.cursor_line,
                "cursorCol": utf16_col(&done.lines, done.cursor_line, done.cursor_col),
            }));
        }
        BaseAsk::ShouldTrigger {
            lines,
            cursor_line,
            cursor_col,
            answer,
        } => {
            let col = byte_col(&lines, cursor_line, cursor_col);
            let _ = answer.send(json!(base.should_trigger_file_completion(
                &lines,
                cursor_line,
                col
            )));
        }
    }
}

struct Remote {
    base: Rc<CombinedAutocompleteProvider>,
    live: &'static Live,
    inbox: Rc<Inbox>,
    chain: Chain,
    calls: Cell<u64>,
}

impl Remote {
    fn call(&self) -> u64 {
        self.calls.set(self.calls.get() + 1);
        self.calls.get()
    }

    fn wait(&self, request: u64) -> Option<Completing> {
        let deadline = Instant::now() + EVENT_BUDGET;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(message) = self.inbox.receiver.recv_timeout(left) else {
                self.inbox.timed_out.set(true);
                return None;
            };
            match message {
                Completing::Applied {
                    request: answered, ..
                }
                | Completing::Triggered {
                    request: answered, ..
                } if answered == request => {
                    return Some(message);
                }
                other => self.inbox.keep(other, &self.base),
            }
        }
    }
}

impl AutocompleteProvider for Remote {
    fn trigger_characters(&self) -> Option<Vec<String>> {
        Some(self.chain.triggers.clone())
    }

    fn get_suggestions(
        &self,
        request: u64,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Suggesting {
        self.live.ui(&ToHost::AutocompleteSuggest {
            request,
            lines,
            cursor_line,
            cursor_col: utf16_col(lines, cursor_line, cursor_col),
            force,
        });
        Suggesting::Pending
    }

    fn abandon(&self, request: u64) {
        self.live.ui(&ToHost::AutocompleteAbandon { request });
    }

    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> Completion {
        let request = self.call();
        self.live.ui(&ToHost::AutocompleteApply {
            request,
            lines,
            cursor_line,
            cursor_col: utf16_col(lines, cursor_line, cursor_col),
            item: ItemWire::from(item),
            prefix,
        });
        match self.wait(request) {
            Some(Completing::Applied {
                completion: Some(completion),
                ..
            }) => completion,
            _ => self
                .base
                .apply_completion(lines, cursor_line, cursor_col, item, prefix),
        }
    }

    fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        if !self.chain.should_trigger {
            return true;
        }
        let request = self.call();
        self.live.ui(&ToHost::AutocompleteShouldTrigger {
            request,
            lines,
            cursor_line,
            cursor_col: utf16_col(lines, cursor_line, cursor_col),
        });
        match self.wait(request) {
            Some(Completing::Triggered { trigger, .. }) => trigger,
            _ => self
                .base
                .should_trigger_file_completion(lines, cursor_line, cursor_col),
        }
    }
}

pub struct Completions {
    inbox: Rc<Inbox>,
    base: Rc<CombinedAutocompleteProvider>,
    remote: Option<Rc<Remote>>,
}

pub enum Served {
    Quiet,
    Delivered,
    TimedOut,
}

impl Completions {
    pub fn new(receiver: Receiver<Completing>, base: Rc<CombinedAutocompleteProvider>) -> Self {
        Completions {
            inbox: Rc::new(Inbox {
                receiver,
                held: RefCell::new(VecDeque::new()),
                timed_out: Cell::new(false),
            }),
            base,
            remote: None,
        }
    }

    pub fn set_base(&mut self, editor: &RefCell<Editor>, base: Rc<CombinedAutocompleteProvider>) {
        self.base = base;
        let (chain, live) = self
            .remote
            .take()
            .map(|remote| {
                let chain = Chain {
                    triggers: remote.chain.triggers.clone(),
                    should_trigger: remote.chain.should_trigger,
                };
                (chain, remote.live)
            })
            .unzip();
        self.install(editor, chain, live);
    }

    fn install(
        &mut self,
        editor: &RefCell<Editor>,
        chain: Option<Chain>,
        live: Option<&'static Live>,
    ) {
        self.remote = chain.zip(live).map(|(chain, live)| {
            Rc::new(Remote {
                base: self.base.clone(),
                live,
                inbox: self.inbox.clone(),
                chain,
                calls: Cell::new(0),
            })
        });
        let provider: Rc<dyn AutocompleteProvider> = match &self.remote {
            Some(remote) => remote.clone(),
            None => self.base.clone(),
        };
        editor.borrow_mut().set_autocomplete_provider(provider);
    }

    pub fn serve(&mut self, editor: &RefCell<Editor>, live: Option<&'static Live>) -> Served {
        let mut served = Served::Quiet;
        while let Some(message) = self.inbox.next() {
            match message {
                Completing::Base(asked) => answer_base(asked, &self.base),
                Completing::Chain(chain) => {
                    self.install(editor, chain, live);
                    served = Served::Delivered;
                }
                Completing::Suggested {
                    request,
                    suggestions,
                } => {
                    editor
                        .borrow_mut()
                        .deliver_suggestions(request, suggestions);
                    served = Served::Delivered;
                }
                Completing::Applied { .. } | Completing::Triggered { .. } => {}
            }
        }
        if self.inbox.timed_out.replace(false) {
            self.install(editor, None, live);
            return Served::TimedOut;
        }
        served
    }
}
