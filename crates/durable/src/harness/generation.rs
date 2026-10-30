use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ai::event_stream::now_ms;
use crate::ai::stream::new_output;
use crate::ai::types::{
    AssistantMessage, DeferredHandle, Message, Model, ModelThinkingLevel, SimpleStreamOptions,
    StopReason, ThinkingLevel,
};
use crate::cancel::CancelToken;
use crate::entries::assistant_entry;
use crate::errors::{Error, Result};
use crate::harness::compaction::{create_compaction, estimate_context, select_cut};
use crate::harness::context::ContextView;
use crate::harness::inbox::{Boundary, Phase, QueueModes, apply_boundary, prepare_boundary};
use crate::harness::live::live_doc;
use crate::harness::live::{GenerationStatus, LiveState, RunState};
use crate::harness::prompt::{plan_system_entries, render_sections, replay_sections};
use crate::harness::provider::ensure_provider_session_id;
use crate::harness::runtime::{HarnessPhaseHandler, HarnessTaskDefinition, HarnessTaskRuntime};
use crate::harness::tool::{ToolTaskInput, erase, harness_error, tool_task};
use crate::harness::types::{
    CompactionPolicy, CompactionReason, ConversationStreamOptions, Deferred, HookApi, HookTask,
    ModelRef, PromptInput, Settings,
};
use crate::harness::usage::{UsageBucket, record_usage};
use crate::harness::util::{conversation_doc, edit_doc, read_draft, write_draft};
use crate::ids::{ConversationId, EntryId, SubmissionId, TaskId};
use crate::session::types::{TaskCreate, TaskOptions, Tx};
use crate::tasks::{NextTaskState, RunningTask, TaskOutcome, TaskOutcomeError, TaskOwnership};
use crate::types::{EntryDraft, EntryQuery, EntryRecord, ScanOrder, SubmissionSettlement};

pub mod classify;
pub mod rounds;

pub type GenerationInput = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum GenerationCheckpoint {
    Prepare {
        attempt: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compacted: Option<TaskId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        overflow: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<Route>,
    },
    Request {
        attempt: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compacted: Option<TaskId>,
        model: ModelRef,
        thinking_level: ModelThinkingLevel,
        stream_options: ConversationStreamOptions,
        cutoff: EntryId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<Route>,
    },
    Retry {
        attempt: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compacted: Option<TaskId>,
        until: u64,
    },
    Poll {
        attempt: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compacted: Option<TaskId>,
        model: ModelRef,
        cutoff: EntryId,
        handle: DeferredHandle,
        poll_at: u64,
    },
    Tools {
        assistant: EntryId,
        tools: Vec<TaskId>,
        pending: Vec<String>,
    },
    Answered {
        assistant: EntryId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationResult {
    pub entry_id: EntryId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub model: ModelRef,
    pub thinking_level: ModelThinkingLevel,
}

pub type Generation =
    HarnessTaskDefinition<GenerationInput, GenerationCheckpoint, GenerationResult, GenerationHooks>;

pub type Runtime<'a> = &'a (
        dyn HarnessTaskRuntime<
    GenerationInput,
    GenerationCheckpoint,
    GenerationResult,
    GenerationHooks,
> + Sync
    );

pub type BeforeRequestHook =
    Box<dyn Fn(&HookApi, &[Message]) -> Result<Option<Vec<Message>>> + Send + Sync>;
pub type AfterResponseHook = Box<dyn Fn(&HookApi, &AssistantMessage) -> Result<()> + Send + Sync>;
pub type OnYieldHook = Box<
    dyn Fn(&HookApi, &AssistantMessage) -> Result<Option<crate::harness::types::UserInput>>
        + Send
        + Sync,
>;
pub type AfterToolsHook = Box<dyn Fn(&HookApi, EntryId, &[EntryId]) -> Result<()> + Send + Sync>;
pub type AfterAnswerHook = Box<dyn Fn(&HookApi, EntryId) -> Result<bool> + Send + Sync>;
pub type PartialHook = Arc<dyn Fn(&HookApi, Option<Arc<AssistantMessage>>) + Send + Sync>;
pub type MessageEndHook = Arc<dyn Fn(&HookApi, &Message) -> Result<Option<Message>> + Send + Sync>;
pub type RoutesHook = Box<dyn Fn(&Model) -> bool + Send + Sync>;
pub type RouteHook = Box<
    dyn Fn(
            &HookApi,
            &Model,
            &[Message],
            ModelThinkingLevel,
            &CancelToken,
        ) -> std::result::Result<Option<Route>, String>
        + Send
        + Sync,
>;

#[derive(Default)]
pub struct GenerationHooks {
    pub before_request: Option<BeforeRequestHook>,
    pub routes: Option<RoutesHook>,
    pub route: Option<RouteHook>,
    pub after_response: Option<AfterResponseHook>,
    pub on_yield: Option<OnYieldHook>,
    pub after_tools: Option<AfterToolsHook>,
    pub after_answer: Option<AfterAnswerHook>,
    pub on_partial: Option<PartialHook>,
    pub message_end: Option<MessageEndHook>,
    pub on_stream: Option<classify::StreamHook>,
    pub before_tools: Option<classify::BeforeToolsHook>,
}

/// The `message_end` hooks of the extensions this task runs under, in registration order.
pub fn message_end_hooks(runtime: Runtime<'_>) -> Result<Vec<MessageEndHook>> {
    let mut hooks = Vec::new();
    runtime.hooks().each(&mut |generation: &GenerationHooks| {
        if let Some(hook) = generation.message_end.as_ref() {
            hooks.push(Arc::clone(hook));
        }
        Ok(())
    })?;
    Ok(hooks)
}

/// Runs the hooks over a finished message, each seeing what the one before returned. A
/// replacement of another role is dropped, as upstream's `emitMessageEnd` refuses one.
pub fn apply_message_end(
    hooks: &[MessageEndHook],
    api: &HookApi,
    message: Message,
) -> Result<Message> {
    let mut current = message;
    for hook in hooks {
        let Some(next) = hook(api, &current)? else {
            continue;
        };
        if std::mem::discriminant(&next) == std::mem::discriminant(&current) {
            current = next;
        }
    }
    Ok(current)
}

pub const GENERATION_KIND: &str = "pi.generation";
pub const GENERATION_HOOK: HookTask<GenerationHooks> = HookTask::new(GENERATION_KIND);
pub const GENERATION_VERSION: u64 = 1;

fn initial_checkpoint() -> GenerationCheckpoint {
    GenerationCheckpoint::Prepare {
        attempt: 1,
        compacted: None,
        overflow: None,
        route: None,
    }
}

pub fn create_generation(tx: &mut dyn Tx, conversation_id: ConversationId) -> Result<TaskId> {
    tx.create_task(
        TaskCreate {
            kind: GENERATION_KIND.to_owned(),
            version: GENERATION_VERSION,
            input: serde_json::json!({}),
            checkpoint: serde_json::to_value(initial_checkpoint())?,
        },
        TaskOptions {
            ownership: TaskOwnership::Conversation,
            conversation_id: Some(conversation_id),
            background: None,
        },
    )
}

pub fn create_tool_task(
    tx: &mut dyn Tx,
    owner: TaskId,
    assistant: EntryId,
    call_id: &str,
) -> Result<TaskId> {
    let definition = tool_task();
    let input = ToolTaskInput {
        assistant,
        call_id: call_id.to_owned(),
    };
    tx.create_task(
        TaskCreate {
            kind: definition.name.clone(),
            version: definition.version,
            input: serde_json::to_value(&input)?,
            checkpoint: serde_json::to_value((definition.initial)(&input))?,
        },
        TaskOptions {
            ownership: TaskOwnership::Task { task_id: owner },
            conversation_id: None,
            background: None,
        },
    )
}

pub fn definition() -> Generation {
    let mut phases: BTreeMap<
        String,
        HarnessPhaseHandler<
            GenerationInput,
            GenerationCheckpoint,
            GenerationResult,
            GenerationHooks,
        >,
    > = BTreeMap::new();
    phases.insert("prepare".to_owned(), Box::new(prepare));
    phases.insert("request".to_owned(), Box::new(request));
    phases.insert("retry".to_owned(), Box::new(retry));
    phases.insert("poll".to_owned(), Box::new(poll));
    phases.insert("tools".to_owned(), Box::new(rounds::tools_phase));
    phases.insert("answered".to_owned(), Box::new(rounds::answered_phase));
    HarnessTaskDefinition {
        name: GENERATION_KIND.to_owned(),
        version: GENERATION_VERSION,
        initial: Box::new(|_| initial_checkpoint()),
        phases,
        abort: Box::new(abort),
    }
}

pub fn read_live(tx: &mut dyn Tx, conversation_id: ConversationId) -> Result<LiveState> {
    read_draft(tx, &live_doc()?, conversation_id)
}

pub fn write_live(
    tx: &mut dyn Tx,
    conversation_id: ConversationId,
    live: &LiveState,
) -> Result<()> {
    write_draft(tx, &live_doc()?, conversation_id, live)
}

pub fn edit_live<R>(
    tx: &mut dyn Tx,
    conversation_id: ConversationId,
    edit: impl FnOnce(&mut LiveState) -> R,
) -> Result<R> {
    edit_doc(tx, &live_doc()?, conversation_id, edit)
}

pub fn snapshot_live(runtime: Runtime<'_>, cancel: &CancelToken) -> Result<LiveState> {
    let token = live_doc()?;
    let doc = conversation_doc(token.definition(), runtime.conversation_id());
    let value = runtime.snapshot(doc, cancel)?.unwrap_or_default();
    Ok(serde_json::from_value(serde_json::Value::Object(value))?)
}

pub fn start_run(
    tx: &mut dyn Tx,
    conversation_id: ConversationId,
    live: &mut LiveState,
    inputs: Vec<SubmissionId>,
) -> Result<()> {
    live.run = Some(RunState {
        task_id: create_generation(tx, conversation_id)?,
        inputs,
    });
    Ok(())
}

pub fn hand_over(live: &mut LiveState, from: TaskId, to: TaskId) {
    if let Some(run) = live.run.as_mut().filter(|run| run.task_id == from) {
        run.task_id = to;
    }
}

pub fn end_run(
    tx: &mut dyn Tx,
    live: &mut LiveState,
    task_id: TaskId,
    settlement: SubmissionSettlement,
) -> Result<()> {
    if let Some(inputs) = live.end_run(task_id) {
        for id in inputs {
            tx.settle_submission(id, settlement.clone())?;
        }
    }
    Ok(())
}

pub fn fail_run(
    tx: &mut dyn Tx,
    boundary: &mut Boundary,
    live: &mut LiveState,
    task_id: TaskId,
    settlement: SubmissionSettlement,
    now: u64,
) -> Result<()> {
    end_run(tx, live, task_id, settlement)?;
    if live.run.is_some() {
        return Ok(());
    }
    let result = apply_boundary(tx, boundary, Phase::Final, now)?;
    if !result.users.is_empty() {
        start_run(tx, boundary.conversation_id, live, result.users)?;
    }
    Ok(())
}

pub fn queue_modes(settings: &Settings) -> QueueModes {
    QueueModes {
        steering_mode: settings.steering_mode,
        follow_up_mode: settings.follow_up_mode,
        steering_first: settings.steering_first,
    }
}

/// Hides an entry from the model's context; it stays in history.
pub fn omit_entry(tx: &mut dyn Tx, conversation_id: ConversationId, target: EntryId) -> Result<()> {
    tx.append_entry(
        conversation_id,
        EntryDraft {
            kind: crate::entries::context_edit_entry().kind().to_owned(),
            model: None,
            data: None,
            head: None,
            edits: Some(vec![crate::types::ContextEdit {
                target,
                action: crate::types::ContextAction::Omit,
            }]),
        },
    )?;
    Ok(())
}

pub fn append_assistant(
    tx: &mut dyn Tx,
    conversation_id: ConversationId,
    message: &AssistantMessage,
) -> Result<EntryRecord> {
    let key = format!("{}/{}", message.provider, message.model);
    record_usage(
        tx,
        conversation_id,
        UsageBucket::Models,
        &key,
        &message.usage,
    )?;
    tx.append_entry(
        conversation_id,
        EntryDraft {
            kind: assistant_entry().kind().to_owned(),
            model: Some(vec![Message::Assistant(Arc::new(message.clone()))]),
            data: None,
            head: None,
            edits: None,
        },
    )
}

pub fn convert_partial(
    tx: &mut dyn Tx,
    live: &LiveState,
    conversation_id: ConversationId,
    hooks: &[MessageEndHook],
    api: &HookApi,
) -> Result<()> {
    let Some(partial) = live
        .generation
        .as_ref()
        .and_then(|status| status.message.as_ref())
    else {
        return Ok(());
    };
    let aborted = AssistantMessage {
        stop_reason: StopReason::Aborted,
        ..partial.clone()
    };
    let ended = apply_message_end(hooks, api, Message::Assistant(Arc::new(aborted)))?;
    let Message::Assistant(aborted) = ended else {
        return Ok(());
    };
    append_assistant(tx, conversation_id, &aborted)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    Blocking,
    Background,
}

fn budgeted_window(context_window: u64, budget: Option<u64>) -> u64 {
    match (context_window, budget) {
        (window, None) => window,
        (0, Some(budget)) => budget,
        (window, Some(budget)) => window.min(budget),
    }
}

pub fn threshold_compaction(
    view: &ContextView,
    planned: &[EntryDraft],
    context_window: u64,
    policy: &CompactionPolicy,
) -> Option<Threshold> {
    if !policy.enabled || context_window == 0 {
        return None;
    }
    let planned: Vec<Message> = planned
        .iter()
        .flat_map(|entry| entry.model.clone().unwrap_or_default())
        .collect();
    let tokens = estimate_context(view, &planned) as i128;
    let blocking = context_window as i128 - policy.reserve_tokens as i128;
    let background = blocking - policy.background_tokens as i128;
    let over = if tokens > blocking {
        Some(Threshold::Blocking)
    } else if policy.background_tokens > 0 && tokens > background {
        Some(Threshold::Background)
    } else {
        None
    };
    over.filter(|_| select_cut(view, policy.keep_recent_tokens).is_some())
}

pub fn reasoning(level: ModelThinkingLevel) -> Option<ThinkingLevel> {
    Some(match level {
        ModelThinkingLevel::Off => return None,
        ModelThinkingLevel::Minimal => ThinkingLevel::Minimal,
        ModelThinkingLevel::Low => ThinkingLevel::Low,
        ModelThinkingLevel::Medium => ThinkingLevel::Medium,
        ModelThinkingLevel::High => ThinkingLevel::High,
        ModelThinkingLevel::Xhigh => ThinkingLevel::Xhigh,
        ModelThinkingLevel::Max => ThinkingLevel::Max,
    })
}

pub fn simple_options(
    stream: &ConversationStreamOptions,
    level: ModelThinkingLevel,
    signal: &CancelToken,
    session_id: String,
) -> SimpleStreamOptions {
    SimpleStreamOptions {
        signal: Some(signal.clone()),
        api_key: None,
        base_url: None,
        session_id: Some(session_id),
        reasoning: reasoning(level),
        cache_retention: stream.cache_retention,
        deferred: match stream.deferred {
            None => false,
            Some(Deferred::Enabled(enabled)) => enabled,
            Some(Deferred::Window { .. }) => true,
        },
        max_tokens: None,
        transport: stream.transport,
        timeout_ms: stream.timeout_ms,
        max_retries: stream.max_retries,
        max_retry_delay_ms: stream.max_retry_delay_ms,
        headers: stream.headers.clone(),
        cache_points: Vec::new(),
        metadata: stream.metadata.clone(),
        temperature: None,
        tool_choice: None,
        hooks: Default::default(),
    }
}

fn mismatch(phase: &str) -> Error {
    Error::Invalid(format!(
        "generation phase handler {phase} got another checkpoint"
    ))
}

fn settle(
    runtime: Runtime<'_>,
    cancel: &CancelToken,
    settlement: SubmissionSettlement,
    outcome: TaskOutcome<GenerationResult>,
) -> Result<()> {
    let conversation_id = runtime.conversation_id();
    let task_id = runtime.task_id();
    let modes = queue_modes(runtime.settings());
    let now = runtime.now();
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut boundary = prepare_boundary(tx, conversation_id, modes)?;
            let mut live = read_live(tx, conversation_id)?;
            fail_run(
                tx,
                &mut boundary,
                &mut live,
                erase(task_id),
                settlement,
                now,
            )?;
            write_live(tx, conversation_id, &live)?;
            Ok(Some(NextTaskState::Terminal { outcome }))
        }),
        cancel,
    )
}

fn failed(message: String, reason: &str) -> TaskOutcome<GenerationResult> {
    TaskOutcome::Failed {
        error: TaskOutcomeError {
            message,
            detail: Some(serde_json::json!({ "reason": reason })),
        },
        result: None,
    }
}

fn fail_no_model(
    runtime: Runtime<'_>,
    model: Option<&ModelRef>,
    cancel: &CancelToken,
) -> Result<()> {
    let message = match model {
        None => "No model is configured".to_owned(),
        Some(model) => format!(
            "Model {}/{} is not available",
            model.provider, model.model_id
        ),
    };
    settle(
        runtime,
        cancel,
        SubmissionSettlement::Unanswered {
            reason: "no_model".to_owned(),
            detail: None,
        },
        failed(message, "no_model"),
    )
}

fn fail_model_error(runtime: Runtime<'_>, text: String, cancel: &CancelToken) -> Result<()> {
    settle(
        runtime,
        cancel,
        SubmissionSettlement::Unanswered {
            reason: "model_error".to_owned(),
            detail: Some(serde_json::Value::String(text.clone())),
        },
        failed(text, "model_error"),
    )
}

fn prepare(
    task: RunningTask<GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let GenerationCheckpoint::Prepare {
        attempt,
        compacted,
        overflow,
        route,
    } = task.checkpoint().clone()
    else {
        return Err(mismatch("prepare"));
    };
    let conversation_id = runtime.conversation_id();
    let agent = runtime.agent(cancel)?;
    let settings = runtime.settings().clone();
    let Some(model_ref) = agent.model.clone() else {
        return fail_no_model(runtime, None, cancel);
    };
    let Some(resolved) = runtime
        .models()
        .get_model(&model_ref.provider, &model_ref.model_id)
    else {
        return fail_no_model(runtime, Some(&model_ref), cancel);
    };
    if let (Some(compacted), Some(overflow)) = (compacted.as_ref(), overflow.as_ref()) {
        let outcomes = runtime.outcomes(std::slice::from_ref(compacted), cancel)?;
        if !matches!(outcomes.first(), Some(TaskOutcome::Completed { .. })) {
            return fail_model_error(runtime, overflow.clone(), cancel);
        }
    }
    let view = runtime.context(None, cancel)?;
    let shown = replay_sections(&view.messages);
    let env = match runtime.env(cancel) {
        Ok(env) => env,
        Err(error) => {
            if cancel.is_cancelled() {
                return Err(error);
            }
            runtime.report(error);
            None
        }
    };
    let tools: Vec<crate::ai::types::Tool> = agent
        .tools
        .iter()
        .map(|registration| registration.tool.clone())
        .collect();
    let shown_map: BTreeMap<String, String> = shown.iter().cloned().collect();
    let input = PromptInput {
        conversation_id,
        agent: &agent,
        env: env.as_deref(),
        shown: &shown_map,
        read: runtime,
    };
    let desired = render_sections(
        &agent.sections,
        &input,
        &shown,
        &mut |error| runtime.report(Error::Invalid(error.to_string())),
        cancel,
    )?;
    let entries = plan_system_entries(&view, &desired, &tools, runtime.now());
    let threshold = if compacted.is_none() && !routes(runtime, &resolved)? {
        threshold_compaction(
            &view,
            &entries,
            budgeted_window(resolved.context_window, agent.context_budget_tokens),
            &settings.compaction,
        )
    } else {
        None
    };
    let task_id = erase(runtime.task_id());
    if threshold == Some(Threshold::Blocking) {
        runtime.commit(
            Box::new(move |tx, _task| {
                let child = create_compaction(
                    tx,
                    conversation_id,
                    CompactionReason::Threshold,
                    Some(task_id),
                )?;
                Ok(Some(NextTaskState::Waiting {
                    checkpoint: GenerationCheckpoint::Prepare {
                        attempt,
                        compacted: Some(child),
                        overflow: None,
                        route,
                    },
                    on: vec![child],
                    policy: crate::tasks::JoinPolicy::AllSettled,
                }))
            }),
            cancel,
        )?;
        return Ok(());
    }
    let thinking_level = agent.thinking_level;
    let stream_options = settings.stream.clone();
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut cutoff = tx
                .scan_entries(
                    &EntryQuery {
                        conversation_id,
                        min_entry_id: None,
                        max_entry_id: None,
                        order: Some(ScanOrder::Descending),
                    },
                    1,
                    None,
                )?
                .items
                .first()
                .map(|entry| entry.id);
            for entry in entries {
                cutoff = Some(tx.append_entry(conversation_id, entry)?.id);
            }
            let Some(cutoff) = cutoff else {
                return Err(Error::Invalid(format!(
                    "Conversation {conversation_id} has no entries to send"
                )));
            };
            if threshold == Some(Threshold::Background) {
                let live = read_live(tx, conversation_id)?;
                if live.compactions.is_none() {
                    create_compaction(tx, conversation_id, CompactionReason::Threshold, None)?;
                }
            }
            Ok(Some(NextTaskState::Running {
                checkpoint: GenerationCheckpoint::Request {
                    attempt,
                    compacted,
                    model: model_ref,
                    thinking_level,
                    stream_options,
                    cutoff,
                    route,
                },
            }))
        }),
        cancel,
    )
}

fn request(
    task: RunningTask<GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let GenerationCheckpoint::Request {
        attempt,
        compacted,
        model: model_ref,
        thinking_level,
        stream_options,
        cutoff,
        route,
    } = task.checkpoint().clone()
    else {
        return Err(mismatch("request"));
    };
    let conversation_id = runtime.conversation_id();
    runtime.commit(
        Box::new(move |tx, _task| {
            let live = read_live(tx, conversation_id)?;
            let api = HookApi { conversation_id };
            convert_partial(tx, &live, conversation_id, &[], &api)?;
            let mut live = live;
            live.generation = Some(GenerationStatus {
                attempt,
                message: None,
                retry: None,
                deferred: None,
            });
            write_live(tx, conversation_id, &live)?;
            Ok(None)
        }),
        cancel,
    )?;
    let Some(model) = runtime
        .models()
        .get_model(&model_ref.provider, &model_ref.model_id)
    else {
        return fail_no_model(runtime, Some(&model_ref), cancel);
    };
    let view = runtime.context(Some(cutoff), cancel)?;
    let mut replaced: Option<Vec<Message>> = None;
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    runtime.hooks().each(&mut |hooks| {
        if let Some(hook) = hooks.before_request.as_ref()
            && let Some(next) = hook(&api, replaced.as_deref().unwrap_or(&view.messages))?
        {
            replaced = Some(next);
        }
        Ok(())
    })?;
    let messages = replaced.as_deref().unwrap_or(&view.messages);
    let decided = match route.clone() {
        Some(route) => Ok(Some(route)),
        None => route_request(runtime, &api, &model, messages, thinking_level)?,
    };
    let (routed, refused) = match decided {
        Ok(routed) => (routed, None),
        Err(text) => (None, Some(text)),
    };
    let (target_ref, level) = routed.as_ref().map_or_else(
        || (model_ref.clone(), thinking_level),
        |routed| (routed.model.clone(), routed.thinking_level),
    );
    let Some(target) = runtime
        .models()
        .get_model(&target_ref.provider, &target_ref.model_id)
    else {
        return fail_no_model(runtime, Some(&target_ref), cancel);
    };
    if route.is_none()
        && compacted.is_none()
        && let Some(routed) = routed
    {
        let agent = runtime.agent(cancel)?;
        let threshold = threshold_compaction(
            &view,
            &[],
            budgeted_window(target.context_window, agent.context_budget_tokens),
            &runtime.settings().compaction,
        );
        if threshold == Some(Threshold::Blocking) {
            return compact_routed(runtime, attempt, routed, cancel);
        }
        if threshold == Some(Threshold::Background) {
            runtime.commit(
                Box::new(move |tx, _task| {
                    if read_live(tx, conversation_id)?.compactions.is_none() {
                        create_compaction(tx, conversation_id, CompactionReason::Threshold, None)?;
                    }
                    Ok(None)
                }),
                cancel,
            )?;
        }
    }
    let message = match refused {
        Some(text) => {
            let mut message = new_output(&model);
            message.stop_reason = StopReason::Error;
            message.error_message = Some(text);
            message.timestamp = now_ms();
            message
        }
        None => {
            let session_id = ensure_provider_session_id(runtime, cancel)?;
            let options = simple_options(&stream_options, level, runtime.signal(), session_id);
            classify::stream_response(runtime, &target, messages, options, attempt, cancel)?
        }
    };
    let ended = apply_message_end(
        &message_end_hooks(runtime)?,
        &api,
        Message::Assistant(Arc::new(message)),
    )?;
    let Message::Assistant(message) = ended else {
        return Err(Error::Invalid(
            "a finished message changed its role".to_owned(),
        ));
    };
    let message = message.as_ref().clone();
    let request = classify::Request {
        attempt,
        compacted,
        model: target_ref,
        cutoff,
        view: Some(view),
        poll_at: None,
    };
    classify::classify(runtime, &request, message, cancel)
}

fn routes(runtime: Runtime<'_>, model: &Model) -> Result<bool> {
    let mut routed = false;
    runtime.hooks().each(&mut |hooks| {
        if let Some(routes) = hooks.routes.as_ref() {
            routed |= routes(model);
        }
        Ok(())
    })?;
    Ok(routed)
}

fn route_request(
    runtime: Runtime<'_>,
    api: &HookApi,
    model: &Model,
    messages: &[Message],
    level: ModelThinkingLevel,
) -> Result<std::result::Result<Option<Route>, String>> {
    let mut decided = Ok(None);
    runtime.hooks().each(&mut |hooks| {
        if let (Ok(None), Some(route)) = (&decided, hooks.route.as_ref()) {
            decided = route(api, model, messages, level, runtime.signal());
        }
        Ok(())
    })?;
    Ok(decided)
}

fn compact_routed(
    runtime: Runtime<'_>,
    attempt: u64,
    route: Route,
    cancel: &CancelToken,
) -> Result<()> {
    let conversation_id = runtime.conversation_id();
    let task_id = erase(runtime.task_id());
    runtime.commit(
        Box::new(move |tx, _task| {
            edit_live(tx, conversation_id, |live| live.generation = None)?;
            let child = create_compaction(
                tx,
                conversation_id,
                CompactionReason::Threshold,
                Some(task_id),
            )?;
            Ok(Some(NextTaskState::Waiting {
                checkpoint: GenerationCheckpoint::Prepare {
                    attempt,
                    compacted: Some(child),
                    overflow: None,
                    route: Some(route),
                },
                on: vec![child],
                policy: crate::tasks::JoinPolicy::AllSettled,
            }))
        }),
        cancel,
    )
}

fn retry(
    task: RunningTask<GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let GenerationCheckpoint::Retry {
        attempt,
        compacted,
        until,
    } = task.checkpoint().clone()
    else {
        return Err(mismatch("retry"));
    };
    if until > runtime.now() {
        return runtime.sleep_until(until, task.checkpoint().clone(), cancel);
    }
    let conversation_id = runtime.conversation_id();
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut live = read_live(tx, conversation_id)?;
            live.generation = Some(GenerationStatus {
                attempt: attempt + 1,
                message: None,
                retry: None,
                deferred: None,
            });
            write_live(tx, conversation_id, &live)?;
            Ok(Some(NextTaskState::Running {
                checkpoint: GenerationCheckpoint::Prepare {
                    attempt: attempt + 1,
                    compacted,
                    overflow: None,
                    route: None,
                },
            }))
        }),
        cancel,
    )
}

fn poll(
    task: RunningTask<GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let GenerationCheckpoint::Poll {
        attempt,
        compacted,
        model: model_ref,
        cutoff,
        handle,
        poll_at,
    } = task.checkpoint().clone()
    else {
        return Err(mismatch("poll"));
    };
    if poll_at > runtime.now() {
        return runtime.sleep_until(poll_at, task.checkpoint().clone(), cancel);
    }
    let Some(model) = runtime
        .models()
        .get_model(&model_ref.provider, &model_ref.model_id)
    else {
        return fail_no_model(runtime, Some(&model_ref), cancel);
    };
    let message = runtime
        .models()
        .fetch_deferred(&model, &handle, runtime.signal())
        .map_err(|error| Error::Invalid(error.0))?;
    let request = classify::Request {
        attempt,
        compacted,
        model: model_ref,
        cutoff,
        view: None,
        poll_at: Some(poll_at),
    };
    classify::classify(runtime, &request, message, cancel)
}

fn abort(
    task: RunningTask<GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let checkpoint = task.checkpoint().clone();
    if let GenerationCheckpoint::Poll { model, handle, .. } = &checkpoint
        && let Some(model) = runtime.models().get_model(&model.provider, &model.model_id)
        && let Err(error) = runtime
            .models()
            .cancel_deferred(&model, handle, runtime.signal())
    {
        runtime.report(Error::Invalid(error.0));
    }
    let conversation_id = runtime.conversation_id();
    let unstarted = match &checkpoint {
        GenerationCheckpoint::Tools {
            assistant, pending, ..
        } => rounds::read_calls(runtime, *assistant, pending, cancel)?,
        _ => Vec::new(),
    };
    let task_id = erase(runtime.task_id());
    let now = runtime.now();
    let hooks = message_end_hooks(runtime)?;
    let api = HookApi { conversation_id };
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut live = read_live(tx, conversation_id)?;
            convert_partial(tx, &live, conversation_id, &hooks, &api)?;
            for call in &unstarted {
                let result = harness_error("aborted", &format!("Tool {} was aborted", call.name));
                crate::harness::tool::append_tool_result(
                    tx,
                    conversation_id,
                    call,
                    &result,
                    now,
                    None,
                )?;
            }
            end_run(
                tx,
                &mut live,
                task_id,
                SubmissionSettlement::Unanswered {
                    reason: "aborted".to_owned(),
                    detail: None,
                },
            )?;
            write_live(tx, conversation_id, &live)?;
            Ok(Some(NextTaskState::Terminal {
                outcome: TaskOutcome::Aborted {
                    reason: None,
                    result: None,
                },
            }))
        }),
        cancel,
    )
}
