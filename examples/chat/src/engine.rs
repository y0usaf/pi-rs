use std::error::Error;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread;

use pi_durable::ai::event_stream::AssistantMessageEventStream;
use pi_durable::ai::models::{Models, ModelsError};
use pi_durable::ai::providers::faux::{Faux, FauxOptions, FauxResponse, FauxTokenSize};
use pi_durable::ai::registry::Providers;
use pi_durable::ai::types::{
    AssistantBlock, AssistantMessage, DeferredHandle, Message, Model, SimpleStreamOptions,
    StopReason, TranscriptContext, UserBlock, UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::harness::Harness;
use pi_durable::harness::registry::Registry;
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::{Extension, HarnessOptions, ModelRef, RootOptions};
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::{JsonObject, Storage};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub type Update = std::result::Result<Vec<Line>, String>;

const INSTRUCTIONS: &str = "You are talking through a small window that shows plain text. Answer in plain prose, without Markdown.";

const SCRIPT: &str = "No model is configured, so this is pi-durable's faux model, streaming a scripted reply a few words at a time. Each piece is committed to SQLite as it arrives. Kill this window now, with kill -9 if you like, and open it again: the conversation is still here, and this turn picks itself up and finishes. Set PI_CHAT_MODEL to a provider/model, such as vercel-ai-gateway/anthropic/claude-haiku-4.5, to talk to a real one.";

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    User(String),
    Reply {
        text: String,
        resumed: bool,
        live: bool,
    },
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
) -> (mpsc::Sender<String>, async_channel::Receiver<Update>) {
    let (prompts, inbox) = mpsc::channel();
    let (out, updates) = async_channel::unbounded();
    let db = db.to_owned();
    thread::spawn(move || {
        if let Err(error) = run(&db, model, &inbox, &out) {
            out.try_send(Err(error.to_string())).ok();
        }
    });
    (prompts, updates)
}

fn run(
    db: &Path,
    model: Option<ModelRef>,
    inbox: &mpsc::Receiver<String>,
    out: &async_channel::Sender<Update>,
) -> Result<()> {
    let (models, model): (Arc<dyn Models + Send + Sync>, ModelRef) = match model {
        Some(model) => {
            let providers = Providers::builtin();
            if providers
                .get_model(&model.provider, &model.model_id)
                .is_none()
            {
                return Err(format!("unknown model {}/{}", model.provider, model.model_id).into());
            }
            (Arc::new(providers), model)
        }
        None => (
            Arc::new(Scripted::new()?),
            ModelRef {
                provider: "faux".into(),
                model_id: "faux-1".into(),
            },
        ),
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
        storage,
        HarnessOptions {
            models,
            registry,
            settings: None,
            conversation_created: None,
            now: None,
            env: None,
            on_report: Some(Arc::new(|error| eprintln!("pi-chat: {error}"))),
        },
    )?;
    let cancel = CancelToken::new();
    let root = harness.root(
        RootOptions {
            agent: Some(AgentChange {
                model: Change::Set(model),
                instructions: Change::Set(INSTRUCTIONS.into()),
                ..Default::default()
            }),
            init: None,
        },
        &cancel,
    )?;
    let mut watch = root.watch(&cancel)?;
    if let Some(view) = watch.value() {
        out.try_send(Ok(lines(&view))).ok();
    }
    let sink = out.clone();
    watch.start(Box::new(move |view: Option<&JsonObject>, _ops, _cancel| {
        if let Some(view) = view {
            sink.try_send(Ok(lines(view))).ok();
        }
        Ok(())
    }))?;
    harness.resume()?;
    for prompt in inbox {
        root.submit(
            SubmissionDraft::Input {
                request_id: None,
                content: serde_json::from_value(json!(prompt))?,
                when_busy: WhenBusy::default(),
            },
            &cancel,
        )?;
    }
    drop(watch);
    harness.close(&cancel)?;
    Ok(())
}

fn lines(view: &JsonObject) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut resumed = false;
    let entries = view.get("entries").and_then(Value::as_array);
    for entry in entries.into_iter().flatten() {
        let Some(message) = entry.pointer("/model/0") else {
            continue;
        };
        match serde_json::from_value::<Message>(message.clone()) {
            Ok(Message::User(user)) => {
                resumed = false;
                lines.push(Line::User(user_text(&user.content)));
            }
            Ok(Message::Assistant(reply)) => match reply.stop_reason {
                StopReason::Aborted => resumed = true,
                StopReason::Error => lines.push(Line::Failed(
                    reply
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "the model call failed".into()),
                )),
                StopReason::Pending | StopReason::Deferred => {}
                StopReason::Stop | StopReason::Length | StopReason::ToolUse => {
                    lines.push(Line::Reply {
                        text: reply_text(&reply.content),
                        resumed: std::mem::take(&mut resumed),
                        live: false,
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
        let text = live
            .and_then(|live| live.pointer("/generation/message/content"))
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        lines.push(Line::Reply {
            text,
            resumed,
            live: true,
        });
    }
    lines
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

struct Scripted {
    faux: Faux,
    reply: AssistantMessage,
}

impl Scripted {
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
            "api": "faux", "provider": "faux", "model": "faux-1",
            "stopReason": "stop", "timestamp": 0,
            "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}
        }))?;
        Ok(Self { faux, reply })
    }
}

impl Models for Scripted {
    fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        self.faux.get_model(provider, id)
    }

    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        let mut reply = self.reply.clone();
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
                "Still the faux model, so all it can say back is “{last}” Set PI_CHAT_MODEL to a provider/model and open the window again to talk to a real one."
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
        self.faux.fetch_deferred(model, handle, signal)
    }

    fn cancel_deferred(
        &self,
        model: &Model,
        handle: &DeferredHandle,
        signal: &CancelToken,
    ) -> std::result::Result<(), ModelsError> {
        self.faux.cancel_deferred(model, handle, signal)
    }
}
