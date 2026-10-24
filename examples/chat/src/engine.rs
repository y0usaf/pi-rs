use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use pi_durable::ai::auth::env_api_key;
use pi_durable::ai::event_stream::AssistantMessageEventStream;
use pi_durable::ai::models::{Models, ModelsError};
use pi_durable::ai::providers::faux::{Faux, FauxOptions, FauxResponse, FauxTokenSize};
use pi_durable::ai::registry::Providers;
use pi_durable::ai::thinking::{clamp_thinking_level, supported_thinking_levels};
use pi_durable::ai::types::{
    AssistantBlock, AssistantMessage, DeferredHandle, Message, Model, ModelThinkingLevel,
    SimpleStreamOptions, StopReason, TranscriptContext, UserBlock, UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::registry::Registry;
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::{ConversationCreateOptions, Extension, HarnessOptions, ModelRef};
use pi_durable::ids::ConversationId;
use pi_durable::session::types::DocumentWatch;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::tasks::TaskStatus;
use pi_durable::types::{
    ConversationOwnership, ConversationQuery, EntryQuery, EntryRecord, JsonObject, ScanOrder,
    Storage, TaskQuery,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const INSTRUCTIONS: &str = "You are talking through a small window that shows plain text. Answer in plain prose, without Markdown.";

const SCRIPT: &str = "This is pi-durable's faux model, streaming a scripted reply a few words at a time. Each piece is committed to SQLite as it arrives. Kill this window now, with kill -9 if you like, and open it again: the conversation is still here, and this turn picks itself up and finishes. Set an API key such as AI_GATEWAY_API_KEY and pick a real model under the message box to talk to one.";

const FAUX: &str = "faux";

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    User(String),
    Reply {
        text: String,
        resumed: bool,
        stopped: bool,
        live: bool,
        thinking: bool,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub levels: Vec<ModelThinkingLevel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: u64,
    pub title: String,
    pub at: u64,
    pub live: bool,
}

pub enum Command {
    Send(String),
    Open(Option<u64>),
    Model(Choice),
    Thinking(ModelThinkingLevel),
    Stop,
    Refresh,
}

pub enum Update {
    Models(Vec<Choice>),
    Sessions(Vec<Session>),
    Opened(Option<u64>),
    View(Option<u64>, Vec<Line>),
    Agent(Choice, ModelThinkingLevel),
    Failed(String),
}

pub fn model_ref(spec: &str) -> std::result::Result<ModelRef, String> {
    match spec.split_once('/') {
        Some((provider, model_id)) if !provider.is_empty() && !model_id.is_empty() => {
            Ok(ModelRef {
                provider: provider.to_owned(),
                model_id: model_id.to_owned(),
            })
        }
        _ => Err(format!("PI_CHAT_MODEL={spec:?} is not PROVIDER/MODEL")),
    }
}

pub fn start(
    db: &Path,
    model: Option<ModelRef>,
) -> (mpsc::Sender<Command>, async_channel::Receiver<Update>) {
    let (commands, inbox) = mpsc::channel();
    let (out, updates) = async_channel::unbounded();
    let db = db.to_owned();
    let poke = commands.clone();
    thread::spawn(move || {
        if let Err(error) = run(&db, model, &inbox, poke, &out) {
            out.try_send(Update::Failed(error.to_string())).ok();
        }
    });
    (commands, updates)
}

struct Engine {
    harness: Harness,
    storage: Arc<dyn Storage>,
    router: Arc<Router>,
    out: async_channel::Sender<Update>,
    poke: mpsc::Sender<Command>,
    cancel: CancelToken,
    current: Option<Conversation>,
    watch: Option<DocumentWatch>,
    model: ModelRef,
    thinking: ModelThinkingLevel,
}

fn run(
    db: &Path,
    model: Option<ModelRef>,
    inbox: &mpsc::Receiver<Command>,
    poke: mpsc::Sender<Command>,
    out: &async_channel::Sender<Update>,
) -> Result<()> {
    let router = Arc::new(Router::new()?);
    let faux = ModelRef {
        provider: FAUX.into(),
        model_id: "faux-1".into(),
    };
    let model = match model {
        Some(model) if router.get_model(&model.provider, &model.model_id).is_none() => {
            out.try_send(Update::Failed(format!(
                "unknown model {}/{}",
                model.provider, model.model_id
            )))
            .ok();
            faux
        }
        Some(model) => model,
        None => faux,
    };
    let registry = Arc::new(Registry::default());
    registry.install(Extension {
        name: "chat".into(),
        tools: vec![],
        sections: vec![],
        wraps: vec![],
        hooks: vec![],
        tasks: vec![],
    })?;
    let path = db.to_str().ok_or("the database path is not UTF-8")?;
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(path, &Default::default())?);
    let harness = Harness::open(
        Arc::clone(&storage),
        HarnessOptions {
            models: router.clone(),
            registry,
            settings: None,
            conversation_created: None,
            now: None,
            env: None,
            set_timer: None,
            on_report: Some(Arc::new(|error| eprintln!("pi-chat: {error}"))),
        },
    )?;
    out.try_send(Update::Models(router.choices())).ok();
    let mut engine = Engine {
        harness,
        storage,
        router,
        out: out.clone(),
        poke,
        cancel: CancelToken::new(),
        current: None,
        watch: None,
        model,
        thinking: ModelThinkingLevel::Off,
    };
    let sessions = engine.sessions()?;
    let latest = sessions.first().map(|session| session.id);
    engine.out.try_send(Update::Sessions(sessions)).ok();
    engine.open(latest)?;
    engine.harness.resume()?;
    for command in inbox {
        if let Err(error) = engine.apply(command) {
            engine.out.try_send(Update::Failed(error.to_string())).ok();
        }
    }
    engine.watch = None;
    engine.harness.close(&engine.cancel)?;
    Ok(())
}

impl Engine {
    fn apply(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Send(prompt) => self.send(prompt),
            Command::Open(id) => self.open(id),
            Command::Model(choice) => {
                self.model = ModelRef {
                    provider: choice.provider,
                    model_id: choice.id,
                };
                self.configure()
            }
            Command::Thinking(level) => {
                self.thinking = level;
                self.configure()
            }
            Command::Stop => match &self.current {
                Some(conversation) => Ok(conversation.abort(false, &self.cancel)?),
                None => Ok(()),
            },
            Command::Refresh => {
                let sessions = self.sessions()?;
                self.out.try_send(Update::Sessions(sessions)).ok();
                Ok(())
            }
        }
    }

    fn send(&mut self, prompt: String) -> Result<()> {
        if self.current.is_none() {
            let created = self.harness.create_conversation(
                ConversationCreateOptions {
                    ownership: ConversationOwnership::Ownerless,
                    agent: Some(AgentChange {
                        model: Change::Set(self.model.clone()),
                        thinking_level: Change::Set(self.thinking),
                        instructions: Change::Set(INSTRUCTIONS.into()),
                        ..Default::default()
                    }),
                    init: None,
                },
                &self.cancel,
            )?;
            self.open(Some(created.id().0))?;
        }
        let Some(conversation) = &self.current else {
            return Err("the new conversation did not open".into());
        };
        conversation.submit(
            SubmissionDraft::Input {
                request_id: None,
                content: serde_json::from_value(json!(prompt))?,
                when_busy: WhenBusy::default(),
            },
            &self.cancel,
        )?;
        Ok(())
    }

    fn open(&mut self, id: Option<u64>) -> Result<()> {
        self.watch = None;
        self.current = match id {
            Some(id) => self
                .harness
                .conversation(ConversationId(id), &self.cancel)?,
            None => None,
        };
        let session = self
            .current
            .as_ref()
            .map(|conversation| conversation.id().0);
        self.out.try_send(Update::Opened(session)).ok();
        let Some(conversation) = &self.current else {
            self.out.try_send(Update::View(None, Vec::new())).ok();
            return self.agent();
        };
        let agent = conversation.agent(&self.cancel)?;
        if let Some(model) = agent.model {
            self.model = model;
        }
        self.thinking = agent.thinking_level;
        let mut watch = conversation.watch(&self.cancel)?;
        let first = watch.value().map(|view| lines(&view)).unwrap_or_default();
        let mut seen = shape(&first);
        self.out.try_send(Update::View(session, first)).ok();
        let sink = self.out.clone();
        let poke = self.poke.clone();
        watch.start(Box::new(move |view: Option<&JsonObject>, _ops, _cancel| {
            if let Some(view) = view {
                let lines = lines(view);
                if seen != shape(&lines) {
                    seen = shape(&lines);
                    poke.send(Command::Refresh).ok();
                }
                sink.try_send(Update::View(session, lines)).ok();
            }
            Ok(())
        }))?;
        self.watch = Some(watch);
        self.agent()
    }

    fn configure(&mut self) -> Result<()> {
        if let Some(model) = self
            .router
            .get_model(&self.model.provider, &self.model.model_id)
        {
            self.thinking = clamp_thinking_level(&model, self.thinking);
        }
        if let Some(conversation) = &self.current {
            conversation.configure(
                &AgentChange {
                    model: Change::Set(self.model.clone()),
                    thinking_level: Change::Set(self.thinking),
                    ..Default::default()
                },
                &self.cancel,
            )?;
        }
        self.agent()
    }

    fn agent(&self) -> Result<()> {
        let choice = match self
            .router
            .get_model(&self.model.provider, &self.model.model_id)
        {
            Some(model) => choice(&model),
            None => Choice {
                provider: self.model.provider.clone(),
                id: self.model.model_id.clone(),
                name: self.model.model_id.clone(),
                levels: vec![ModelThinkingLevel::Off],
            },
        };
        self.out.try_send(Update::Agent(choice, self.thinking)).ok();
        Ok(())
    }

    fn sessions(&self) -> Result<Vec<Session>> {
        let storage = &*self.storage;
        let cancel = &self.cancel;
        let mut found = self.harness.session().read_on_line(|| {
            let tasks = storage.scan_tasks(
                &TaskQuery {
                    order: Some(ScanOrder::Descending),
                    ..Default::default()
                },
                64,
                None,
                cancel,
            )?;
            let live: Vec<u64> = tasks
                .items
                .iter()
                .filter(|task| task.status() != TaskStatus::Terminal)
                .map(|task| task.conversation_id.0)
                .collect();
            let mut found = Vec::new();
            let mut cursor = None;
            loop {
                let page = storage.scan_conversations(
                    &ConversationQuery {
                        order: Some(ScanOrder::Descending),
                        ..Default::default()
                    },
                    64,
                    cursor.as_ref(),
                    cancel,
                )?;
                for record in page.items.iter().filter(|record| record.owner.is_none()) {
                    let query = |order| EntryQuery {
                        conversation_id: record.id,
                        min_entry_id: None,
                        max_entry_id: None,
                        order: Some(order),
                    };
                    let first =
                        storage.scan_entries(&query(ScanOrder::Ascending), 8, None, cancel)?;
                    let Some(title) = first.items.iter().find_map(said) else {
                        continue;
                    };
                    let last =
                        storage.scan_entries(&query(ScanOrder::Descending), 8, None, cancel)?;
                    let at = last.items.iter().find_map(stamp).unwrap_or_default();
                    let order = last.items.first().map_or(0, |entry| entry.id.0);
                    found.push((
                        order,
                        Session {
                            id: record.id.0,
                            title,
                            at,
                            live: live.contains(&record.id.0),
                        },
                    ));
                }
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Ok(found)
        })?;
        found.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
        Ok(found.into_iter().map(|(_, session)| session).collect())
    }
}

fn said(entry: &EntryRecord) -> Option<String> {
    match entry.model.as_ref()?.first()? {
        Message::User(user) => Some(user_text(&user.content)),
        _ => None,
    }
}

fn stamp(entry: &EntryRecord) -> Option<u64> {
    entry
        .model
        .as_ref()?
        .iter()
        .find_map(|message| match message {
            Message::User(user) => Some(user.timestamp),
            Message::Assistant(reply) => Some(reply.timestamp),
            Message::ToolResult(_) | Message::System(_) => None,
        })
}

fn shape(lines: &[Line]) -> (usize, bool) {
    let live = matches!(lines.last(), Some(Line::Reply { live: true, .. }));
    (lines.len(), live)
}

fn choice(model: &Model) -> Choice {
    Choice {
        provider: model.provider.clone(),
        id: model.id.clone(),
        name: model.name.clone(),
        levels: supported_thinking_levels(model),
    }
}

fn lines(view: &JsonObject) -> Vec<Line> {
    let mut lines = Vec::new();
    let entries = view.get("entries").and_then(Value::as_array);
    for entry in entries.into_iter().flatten() {
        let Some(message) = entry.pointer("/model/0") else {
            continue;
        };
        match serde_json::from_value::<Message>(message.clone()) {
            Ok(Message::User(user)) => lines.push(Line::User(user_text(&user.content))),
            Ok(Message::Assistant(reply)) => match reply.stop_reason {
                StopReason::Aborted => lines.push(Line::Reply {
                    text: reply_text(&reply.content),
                    resumed: false,
                    stopped: true,
                    live: false,
                    thinking: false,
                }),
                StopReason::Error => lines.push(Line::Failed(
                    reply
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "the model call failed".into()),
                )),
                StopReason::Pending | StopReason::Deferred => {}
                StopReason::Stop | StopReason::Length | StopReason::ToolUse => {
                    let resumed = resume(&mut lines);
                    lines.push(Line::Reply {
                        text: reply_text(&reply.content),
                        resumed,
                        stopped: false,
                        live: false,
                        thinking: false,
                    })
                }
            },
            Ok(Message::System(_) | Message::ToolResult(_)) => {}
            Err(error) => lines.push(Line::Failed(format!("unreadable entry: {error}"))),
        }
    }
    let live = view.get("docs").and_then(|docs| docs.get("pi.live"));
    if live
        .and_then(|live| live.get("run"))
        .is_some_and(|run| !run.is_null())
    {
        let blocks = live
            .and_then(|live| live.pointer("/generation/message/content"))
            .and_then(Value::as_array);
        let field = |name: &str| -> String {
            blocks
                .into_iter()
                .flatten()
                .filter_map(|block| block.get(name).and_then(Value::as_str))
                .collect()
        };
        let text = field("text");
        let thinking = !field("thinking").is_empty();
        let resumed = resume(&mut lines);
        lines.push(Line::Reply {
            text,
            resumed,
            stopped: false,
            live: true,
            thinking,
        });
    }
    lines
}

fn resume(lines: &mut Vec<Line>) -> bool {
    let stopped = matches!(lines.last(), Some(Line::Reply { stopped: true, .. }));
    if stopped {
        lines.pop();
    }
    stopped
}

fn user_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserBlock::Text(text) => Some(text.text.as_str()),
                UserBlock::Image(_) => None,
            })
            .collect(),
    }
}

fn reply_text(content: &[AssistantBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            AssistantBlock::Text(text) => Some(text.text.as_str()),
            AssistantBlock::Thinking(_) | AssistantBlock::ToolCall(_) => None,
        })
        .collect()
}

struct Router {
    builtin: Providers,
    faux: Faux,
    reply: AssistantMessage,
}

impl Router {
    fn new() -> Result<Self> {
        let faux = Faux::new(FauxOptions {
            tokens_per_second: Some(14.0),
            token_size: FauxTokenSize {
                min: Some(1),
                max: Some(2),
            },
            ..Default::default()
        });
        let reply = serde_json::from_value(json!({
            "role": "assistant",
            "content": [{"type": "text", "text": SCRIPT}],
            "api": FAUX, "provider": FAUX, "model": "faux-1",
            "stopReason": "stop", "timestamp": 0,
            "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}
        }))?;
        Ok(Self {
            builtin: Providers::builtin(),
            faux,
            reply,
        })
    }

    fn choices(&self) -> Vec<Choice> {
        let none = BTreeMap::new();
        let mut keyed = BTreeMap::new();
        let faux = self.faux.get_model(FAUX, "faux-1");
        faux.iter()
            .chain(self.builtin.models().iter().filter(|model| {
                *keyed
                    .entry(model.provider.clone())
                    .or_insert_with(|| env_api_key(&model.provider, &none).is_some())
            }))
            .map(choice)
            .collect()
    }

    fn models(&self, provider: &str) -> &dyn Models {
        if provider == FAUX {
            &self.faux
        } else {
            &self.builtin
        }
    }
}

impl Models for Router {
    fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        self.models(provider).get_model(provider, id)
    }

    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        if model.provider != FAUX {
            return self.builtin.stream_simple(model, context, options);
        }
        let mut reply = self.reply.clone();
        reply.timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        let said: Vec<String> = context
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::User(user) => Some(user_text(&user.content)),
                _ => None,
            })
            .collect();
        if let ([_, _, ..], Some(last), Some(AssistantBlock::Text(block))) =
            (said.as_slice(), said.last(), reply.content.first_mut())
        {
            block.text = format!(
                "Still the faux model, so all it can say back is “{last}” Pick a real model under the message box to talk to one."
            );
        }
        self.faux
            .set_responses(vec![FauxResponse::Message(Box::new(reply))]);
        self.faux.stream_simple(model, context, options)
    }

    fn fetch_deferred(
        &self,
        model: &Model,
        handle: &DeferredHandle,
        signal: &CancelToken,
    ) -> std::result::Result<AssistantMessage, ModelsError> {
        self.models(&model.provider)
            .fetch_deferred(model, handle, signal)
    }

    fn cancel_deferred(
        &self,
        model: &Model,
        handle: &DeferredHandle,
        signal: &CancelToken,
    ) -> std::result::Result<(), ModelsError> {
        self.models(&model.provider)
            .cancel_deferred(model, handle, signal)
    }
}
