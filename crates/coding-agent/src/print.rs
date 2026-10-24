//! Print mode (`modes/print-mode.ts`) on pi-durable's Harness: the built-in tools and the
//! system prompt as a durable extension, one conversation per run, and the text output.

use std::sync::Arc;

use pi_durable::ai::registry::Providers;
use pi_durable::ai::types::{
    AssistantBlock, Message, Model, ModelThinkingLevel, StopReason, TextContent, TextType, Tool,
    UserBlock, UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::errors::Error;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::registry::Registry;
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::{
    CompactionOverrides, Extension, HarnessOptions, HarnessSettings, ModelRef, OutputLimitsSpec,
    PrepareArguments, PromptInput, PromptSection, Render, RootOptions, ToolErrorStyle, ToolExecute,
    ToolExecutionResult, ToolRegistration,
};
use pi_durable::types::{EntryQuery, ScanOrder};
use serde_json::Value;

use crate::models::ModelRegistry;
use crate::tools::{self, ToolContext, ToolSpec};

/// What every tool call of a run shares.
#[derive(Clone)]
pub struct ToolSettings {
    pub cwd: String,
    pub shell_path: Option<String>,
    pub command_prefix: Option<String>,
    /// `PI_SESSION_ID`, `PI_PROVIDER`, ..., and PATH with the agent's bin directory first.
    pub env: Vec<(String, String)>,
    pub model_takes_images: bool,
}

fn text_block(text: String) -> UserBlock {
    UserBlock::Text(TextContent {
        kind: TextType,
        text,
        text_signature: None,
    })
}

/// One of our tools as a durable tool. A thrown error becomes an error result holding only its
/// message, as upstream's agent loop makes it; the harness's own output bound is lifted, since
/// the tools truncate their output themselves.
fn registration(spec: ToolSpec, settings: Arc<ToolSettings>) -> ToolRegistration {
    let prepare: Option<PrepareArguments> = spec
        .prepare_arguments
        .map(|f| Arc::new(move |v: Value| Ok(f(v))) as PrepareArguments);
    let execute_fn = spec.execute;
    let execute: ToolExecute = Arc::new(move |args, _api, cancel| {
        let mut ctx = ToolContext {
            cwd: &settings.cwd,
            cancel,
            model_takes_images: settings.model_takes_images,
            shell_path: settings.shell_path.as_deref(),
            command_prefix: settings.command_prefix.as_deref(),
            env: &settings.env,
            on_update: None,
        };
        Ok(match execute_fn(&args, &mut ctx) {
            Ok(out) => ToolExecutionResult {
                content: Some(out.content),
                is_error: Some(out.is_error),
                details: out.details,
                ..ToolExecutionResult::default()
            },
            Err(_) if cancel.is_cancelled() => return Err(Error::Aborted),
            Err(message) => ToolExecutionResult {
                content: Some(vec![text_block(message)]),
                is_error: Some(true),
                details: Some(Value::Object(Default::default())),
                ..ToolExecutionResult::default()
            },
        })
    });
    ToolRegistration {
        tool: Tool {
            name: spec.name.to_owned(),
            description: spec.description,
            parameters: spec.parameters,
            constrained_sampling: None,
        },
        replay: None,
        execution_mode: None,
        output_limits: Some(OutputLimitsSpec {
            max_bytes: Some(usize::MAX),
            max_lines: Some(usize::MAX),
            retain: None,
        }),
        prepare_arguments: prepare,
        execute,
    }
}

fn fixed_section(key: &str, text: String) -> PromptSection {
    let render: Render =
        Arc::new(move |_: &PromptInput<'_>, _: &CancelToken| Ok(Some(text.clone())));
    PromptSection {
        key: key.to_owned(),
        render,
        tag: Some(false),
    }
}

/// The run's extension: the active tools, in order, and the system prompt's sections, already
/// wrapped in their tags as `buildSystemPromptSections` makes them.
pub fn extension(
    tool_names: &[String],
    sections: Vec<(String, String)>,
    settings: ToolSettings,
) -> Extension {
    extension_with(tool_names, sections, settings, Vec::new())
}

/// The built-in tools and the ones other parts add (MCP's servers'), in that order.
pub fn extension_with(
    tool_names: &[String],
    sections: Vec<(String, String)>,
    settings: ToolSettings,
    extra: Vec<ToolRegistration>,
) -> Extension {
    let settings = Arc::new(settings);
    Extension {
        name: "pi-coding-agent".to_owned(),
        tools: tool_names
            .iter()
            .filter_map(|name| tools::spec(name))
            .map(|spec| registration(spec, Arc::clone(&settings)))
            .chain(extra)
            .collect(),
        sections: sections
            .into_iter()
            .map(|(key, text)| fixed_section(&key, text))
            .collect(),
        wraps: Vec::new(),
        hooks: Vec::new(),
        tasks: Vec::new(),
    }
}

/// The models the harness can call: the composed catalog plus the chosen model (a custom id
/// from `--model` is not in it), with each request's auth from the registry.
pub fn providers(registry: Arc<ModelRegistry>, chosen: &Model) -> Providers {
    let mut providers = Providers::builtin();
    providers.add_models(registry.models().iter().cloned());
    providers.add_models([chosen.clone()]);
    providers.set_auth(Arc::new(move |model: &Model| registry.resolve_auth(model)));
    providers
}

pub struct RunInput {
    pub model: Model,
    pub thinking: ModelThinkingLevel,
    pub cwd: String,
    /// The first message (stdin, `@file`s and the first argument) and its images.
    pub initial: Option<(String, Vec<UserBlock>)>,
    /// The other message arguments, each sent after the previous one finishes.
    pub messages: Vec<String>,
}

/// The messages of the conversation the model still sees, oldest first.
pub fn transcript(
    conversation: &Conversation,
    cancel: &CancelToken,
) -> Result<Vec<Message>, Error> {
    let mut entries: Vec<(pi_durable::ids::EntryId, Vec<Message>)> = Vec::new();
    let mut omitted = std::collections::BTreeSet::new();
    let mut from = None;
    loop {
        let page = conversation.entries(
            EntryQuery {
                conversation_id: conversation.id(),
                min_entry_id: from,
                max_entry_id: None,
                order: Some(ScanOrder::Ascending),
            },
            500,
            None,
            cancel,
        )?;
        let last = page.items.last().map(|e| e.id);
        for entry in page.items {
            for edit in entry.edits.iter().flatten() {
                if matches!(edit.action, pi_durable::types::ContextAction::Omit) {
                    omitted.insert(edit.target);
                }
            }
            if let Some(model) = entry.model {
                entries.push((entry.id, model));
            }
        }
        match (page.next, last) {
            (Some(_), Some(id)) => from = Some(pi_durable::ids::EntryId(id.0 + 1)),
            _ => break,
        }
    }
    // Hidden entries (a cut-off reply the harness dropped) are left out, as upstream prints
    // from the projected context.
    Ok(entries
        .into_iter()
        .filter(|(id, _)| !omitted.contains(id))
        .flat_map(|(_, model)| model)
        .collect())
}

/// Sends the messages and waits for each run to finish, as `session.prompt` does in print mode.
pub fn run_turns(
    harness: &Harness,
    conversation: &Conversation,
    input: &RunInput,
    cancel: &CancelToken,
) -> Result<(), Error> {
    conversation.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: input.model.provider.clone(),
                model_id: input.model.id.clone(),
            }),
            thinking_level: Change::Set(input.thinking),
            cwd: Change::Set(input.cwd.clone()),
            ..AgentChange::default()
        },
        cancel,
    )?;
    let mut prompts: Vec<UserContent> = Vec::new();
    // `session.prompt` sends `[{type: "text", text}, ...images]`, never a bare string.
    if let Some((text, images)) = &input.initial {
        let mut blocks = vec![text_block(text.clone())];
        blocks.extend(images.iter().cloned());
        prompts.push(UserContent::Blocks(blocks));
    }
    prompts.extend(
        input
            .messages
            .iter()
            .map(|m| UserContent::Blocks(vec![text_block(m.clone())])),
    );
    for content in prompts {
        conversation.submit(
            SubmissionDraft::Input {
                request_id: None,
                content,
                when_busy: WhenBusy::default(),
            },
            cancel,
        )?;
        harness.wait_for_idle(cancel)?;
    }
    Ok(())
}

/// Text mode's ending: the last assistant message's text on stdout (exit 0), or its error on
/// stderr (exit 1); a kept reply cut by the output limit is printed and exits 1. The harness
/// hides a recoverable cut reply, so a run that only produced one prints nothing.
pub fn text_result(messages: &[Message]) -> (String, String, i32) {
    let Some(Message::Assistant(last)) = messages.last() else {
        return (String::new(), String::new(), 0);
    };
    match last.stop_reason {
        StopReason::Error | StopReason::Aborted => {
            let reason = if last.stop_reason == StopReason::Error {
                "error"
            } else {
                "aborted"
            };
            let message = last
                .error_message
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| format!("Request {reason}"));
            (String::new(), format!("{message}\n"), 1)
        }
        _ => {
            let mut out = String::new();
            for block in &last.content {
                if let AssistantBlock::Text(t) = block {
                    out.push_str(&t.text);
                    out.push('\n');
                }
            }
            if last.stop_reason == StopReason::Length {
                return (
                    out,
                    "Response was truncated: the model reached its output token limit.\n".into(),
                    1,
                );
            }
            (out, String::new(), 0)
        }
    }
}

/// Set once this process starts closing its harness. pi-durable's scheduler can still run a
/// drain kicked by the last task's end after `Harness::close` has closed the session, and
/// reports "Session is closed" from it; reports after closing began are dropped as shutdown noise.
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn open_harness(
    storage: Arc<dyn pi_durable::types::Storage>,
    providers: Providers,
    extension: Extension,
    compaction: CompactionOverrides,
) -> Result<Harness, Error> {
    open_harness_with(
        storage,
        providers,
        extension,
        compaction,
        Arc::new(|error| {
            if !CLOSING.load(std::sync::atomic::Ordering::SeqCst) {
                eprintln!("{error}");
            }
        }),
    )
}

/// `open_harness` with the scheduler's error reports going to `report` (interactive mode shows
/// them in the chat rather than on stderr, under the TUI).
pub fn open_harness_with(
    storage: Arc<dyn pi_durable::types::Storage>,
    providers: Providers,
    extension: Extension,
    compaction: CompactionOverrides,
    report: pi_durable::harness::types::ErrorReport,
) -> Result<Harness, Error> {
    let registry = Arc::new(Registry::default());
    registry.install(extension)?;
    CLOSING.store(false, std::sync::atomic::Ordering::SeqCst);
    Harness::open(
        storage,
        HarnessOptions {
            models: Arc::new(providers),
            registry,
            // Upstream's agent loop sends a bad call's error as bare text.
            settings: Some(HarnessSettings {
                tool_errors: Some(ToolErrorStyle::Plain),
                // A reply cut by the output limit is hidden and retried after compaction.
                length_recovery: Some(true),
                compaction: Some(compaction),
                ..HarnessSettings::default()
            }),
            conversation_created: None,
            now: None,
            env: None,
            set_timer: None,
            on_report: Some(Arc::new(move |error| {
                if !CLOSING.load(std::sync::atomic::Ordering::SeqCst) {
                    report(error);
                }
            })),
        },
    )
}

/// `Harness::close`, without the scheduler's shutdown reports.
pub fn close(harness: &Harness, cancel: &CancelToken) -> Result<(), Error> {
    CLOSING.store(true, std::sync::atomic::Ordering::SeqCst);
    harness.close(cancel)
}

pub fn root(harness: &Harness, cancel: &CancelToken) -> Result<Conversation, Error> {
    harness.root(RootOptions::default(), cancel)
}
