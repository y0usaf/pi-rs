use crate::ai::event_stream::{Next, now_ms};
use crate::ai::stream::new_output;
use crate::ai::types::{
    AssistantBlock, AssistantMessage, AssistantMessageEvent, Message, Model, SimpleStreamOptions,
    StopReason, ToolCall,
};
use crate::ai::utils::estimate::{calculate_context_tokens, estimate_message_tokens};
use crate::ai::utils::overflow::is_context_overflow;
use crate::ai::utils::retry::{is_retryable_assistant_error, retry_delay_ms};
use crate::cancel::CancelToken;
use crate::entries::compaction_entry;
use crate::errors::{Error, Result};
use crate::harness::compaction::{create_answer_compaction, create_compaction, select_cut};
use crate::harness::context::ContextView;
use crate::harness::generation::{
    GenerationCheckpoint, PartialHook, Runtime, append_assistant, edit_live, fail_run, queue_modes,
    read_live, write_live,
};
use crate::harness::inbox::prepare_boundary;
use crate::harness::live::{DeferredStatus, GenerationStatus, RetryStatus};
use crate::harness::tool::ToolRound;
use crate::harness::types::{CompactionReason, HookApi, ModelRef};
use crate::ids::{EntryId, TaskId};
use crate::tasks::{JoinPolicy, NextTaskState, TaskOutcome, TaskOutcomeError};
use crate::types::SubmissionSettlement;
use std::sync::Arc;

use std::time::{Duration, Instant};

pub const DEFAULT_POLL_AFTER_MS: u64 = 5000;

pub type StreamHook = Arc<
    dyn Fn(&HookApi, &AssistantMessageEvent, &dyn Fn() -> Option<AssistantMessage>) + Send + Sync,
>;

pub type BeforeToolsHook = Box<dyn Fn(&HookApi, &ToolRound<'_>) -> Result<()> + Send + Sync>;

pub fn before_tools(runtime: Runtime<'_>, round: &ToolRound<'_>) -> Result<()> {
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    runtime
        .hooks()
        .each(&mut |hooks| match hooks.before_tools.as_ref() {
            Some(hook) => hook(&api, round),
            None => Ok(()),
        })
}

pub struct Request {
    pub attempt: u64,
    pub compacted: Option<TaskId>,
    pub model: ModelRef,
    pub cutoff: EntryId,
    pub view: Option<Arc<ContextView>>,
    pub poll_at: Option<u64>,
}

fn into_partial(event: AssistantMessageEvent) -> Option<AssistantMessage> {
    match event {
        AssistantMessageEvent::Start { partial }
        | AssistantMessageEvent::TextStart { partial, .. }
        | AssistantMessageEvent::TextDelta { partial, .. }
        | AssistantMessageEvent::TextEnd { partial, .. }
        | AssistantMessageEvent::ThinkingStart { partial, .. }
        | AssistantMessageEvent::ThinkingDelta { partial, .. }
        | AssistantMessageEvent::ThinkingEnd { partial, .. }
        | AssistantMessageEvent::ToolcallStart { partial, .. }
        | AssistantMessageEvent::ToolcallDelta { partial, .. }
        | AssistantMessageEvent::ToolcallEnd { partial, .. } => Some(partial),
        AssistantMessageEvent::Done { .. } | AssistantMessageEvent::Error { .. } => None,
    }
}

pub fn stream_response(
    runtime: Runtime<'_>,
    model: &Model,
    messages: &[Message],
    options: SimpleStreamOptions,
    attempt: u64,
    cancel: &CancelToken,
) -> Result<AssistantMessage> {
    let mut sinks: Vec<PartialHook> = Vec::new();
    let mut streams: Vec<StreamHook> = Vec::new();
    runtime.hooks().each(&mut |hooks| {
        if let Some(sink) = hooks.on_partial.as_ref() {
            sinks.push(Arc::clone(sink));
        }
        if let Some(stream) = hooks.on_stream.as_ref() {
            streams.push(Arc::clone(stream));
        }
        Ok(())
    })?;
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    let result = stream_events(
        runtime,
        model,
        messages,
        options,
        attempt,
        cancel,
        (&sinks, &streams),
        &api,
    );
    if result.is_err() || runtime.signal().is_cancelled() {
        for sink in &sinks {
            sink(&api, None);
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn stream_events(
    runtime: Runtime<'_>,
    model: &Model,
    messages: &[Message],
    mut options: SimpleStreamOptions,
    attempt: u64,
    cancel: &CancelToken,
    (sinks, streams): (&[PartialHook], &[StreamHook]),
    api: &HookApi,
) -> Result<AssistantMessage> {
    let progress = runtime.settings().progress;
    let interval = Duration::from_millis(progress.partial_interval_ms);
    let stall = Duration::from_millis(progress.stall_ms);
    let stop = CancelToken::new();
    let _link = options.signal.replace(stop.clone()).map(|signal| {
        let stop = stop.clone();
        signal.subscribe(move || stop.cancel())
    });
    let stream = runtime.models().stream_simple(
        model,
        crate::ai::types::TranscriptContext {
            messages: messages.to_vec(),
        },
        options,
    );
    let mut latest: Option<Arc<AssistantMessage>> = None;
    let mut pending: Option<Arc<AssistantMessage>> = None;
    let mut due: Option<Instant> = None;
    let mut committed = Instant::now()
        .checked_sub(interval)
        .unwrap_or_else(Instant::now);
    let mut heard = Instant::now();
    loop {
        let quiet = heard + stall;
        let wake = due.map_or(quiet, |at| at.min(quiet));
        match stream.next_event_timeout(wake.saturating_duration_since(Instant::now())) {
            Next::Event(event) => {
                heard = Instant::now();
                let current = std::cell::OnceCell::new();
                for hook in streams {
                    hook(api, &event, &|| {
                        current.get_or_init(|| stream.current()).clone()
                    });
                }
                let Some(partial) = into_partial(event) else {
                    continue;
                };
                if partial.content.is_empty() {
                    continue;
                }
                let partial = Arc::new(partial);
                for sink in sinks {
                    sink(api, Some(Arc::clone(&partial)));
                }
                latest = Some(Arc::clone(&partial));
                pending = Some(partial);
                due.get_or_insert(committed + interval);
            }
            Next::Timeout if Instant::now() >= quiet => {
                stop.cancel();
                let mut message = latest.map_or_else(|| new_output(model), Arc::unwrap_or_clone);
                message.stop_reason = StopReason::Error;
                message.error_message =
                    Some(format!("Request timed out: no stream event in {stall:?}"));
                message.timestamp = now_ms();
                return Ok(message);
            }
            Next::Timeout => {
                due = None;
                if let Some(partial) = pending.take() {
                    commit_partial(&runtime, attempt, partial, cancel)?;
                    committed = Instant::now();
                }
            }
            Next::End => break,
        }
    }
    stream
        .result()
        .ok_or_else(|| Error::Invalid("stream ended without a final message".to_owned()))
}

fn commit_partial(
    runtime: &Runtime<'_>,
    attempt: u64,
    snapshot: Arc<AssistantMessage>,
    cancel: &CancelToken,
) -> Result<()> {
    let snapshot = Arc::try_unwrap(snapshot).unwrap_or_else(|shared| (*shared).clone());
    let conversation_id = runtime.conversation_id();
    runtime.commit(
        Box::new(move |tx, _task| {
            edit_live(tx, conversation_id, |live| {
                live.generation
                    .get_or_insert(GenerationStatus {
                        attempt,
                        message: None,
                        retry: None,
                        deferred: None,
                    })
                    .message = Some(snapshot);
            })?;
            Ok(None)
        }),
        cancel,
    )?;
    Ok(())
}

pub fn classify(
    runtime: Runtime<'_>,
    request: &Request,
    message: AssistantMessage,
    cancel: &CancelToken,
) -> Result<()> {
    if runtime.signal().is_cancelled() {
        return Err(Error::Aborted);
    }
    let conversation_id = runtime.conversation_id();
    let task_id = crate::harness::tool::erase(runtime.task_id());
    if message.stop_reason == StopReason::Deferred
        && let Some(handle) = message.deferred.clone()
    {
        let after = handle.poll_after_ms.unwrap_or(DEFAULT_POLL_AFTER_MS);
        let poll_at = (runtime.now() + after).max(request.poll_at.map_or(0, |at| at + 1));
        let attempt = request.attempt;
        let compacted = request.compacted;
        let model = request.model.clone();
        let cutoff = request.cutoff;
        runtime.commit(
            Box::new(move |tx, _task| {
                edit_live(tx, conversation_id, |live| {
                    live.generation = Some(GenerationStatus {
                        attempt,
                        message: None,
                        retry: None,
                        deferred: Some(DeferredStatus { poll_at }),
                    });
                })?;
                Ok(Some(NextTaskState::Sleeping {
                    checkpoint: GenerationCheckpoint::Poll {
                        attempt,
                        compacted,
                        model,
                        cutoff,
                        handle,
                        poll_at,
                    },
                    until: poll_at,
                }))
            }),
            cancel,
        )?;
        return Ok(());
    }
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    runtime.hooks().each(&mut |hooks| {
        if let Some(hook) = hooks.after_response.as_ref() {
            hook(&api, &message)?;
        }
        Ok(())
    })?;
    let calls: Vec<ToolCall> = message
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantBlock::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect();
    if message.stop_reason == StopReason::ToolUse && !calls.is_empty() {
        return crate::harness::generation::rounds::start_tool_round(
            runtime, request, message, calls, cancel,
        );
    }
    if message.stop_reason == StopReason::Length
        && request.compacted.is_none()
        && runtime.settings().length_recovery
        && runtime
            .models()
            .get_model(&request.model.provider, &request.model.model_id)
            .is_some_and(|model| model.max_tokens > 0 && message.usage.output < model.max_tokens)
    {
        return recover_length(runtime, request, message, cancel);
    }
    if matches!(
        message.stop_reason,
        StopReason::Stop | StopReason::Length | StopReason::ToolUse
    ) {
        if calls.is_empty() && compacts_after(runtime, request, &message, cancel)? {
            runtime.commit(
                Box::new(move |tx, _task| {
                    create_answer_compaction(tx, conversation_id, task_id)?;
                    Ok(None)
                }),
                cancel,
            )?;
        }
        return crate::harness::generation::rounds::answer(runtime, message, cancel);
    }
    let settings = runtime.settings().clone();
    let overflow = message.stop_reason == StopReason::Error && is_context_overflow(&message, None);
    if overflow && request.compacted.is_none() && settings.compaction.enabled {
        let view = runtime.context(Some(request.cutoff), cancel)?;
        if select_cut(&view, settings.compaction.keep_recent_tokens).is_some() {
            let text = message
                .error_message
                .clone()
                .unwrap_or_else(|| "Context overflow".to_owned());
            let attempt = request.attempt;
            runtime.commit(
                Box::new(move |tx, _task| {
                    append_assistant(tx, conversation_id, &message)?;
                    edit_live(tx, conversation_id, |live| live.generation = None)?;
                    let child = create_compaction(
                        tx,
                        conversation_id,
                        CompactionReason::Overflow,
                        Some(task_id),
                    )?;
                    Ok(Some(NextTaskState::Waiting {
                        checkpoint: GenerationCheckpoint::Prepare {
                            attempt,
                            compacted: Some(child),
                            overflow: Some(text),
                            route: None,
                        },
                        on: vec![child],
                        policy: JoinPolicy::AllSettled,
                    }))
                }),
                cancel,
            )?;
            return Ok(());
        }
    }
    let policy = settings.retry;
    let retry = message.stop_reason == StopReason::Error
        && !overflow
        && is_retryable_assistant_error(&message)
        && policy.enabled
        && request.attempt <= policy.max_retries;
    let until = if retry {
        runtime.now()
            + retry_delay_ms(
                policy.base_delay_ms,
                policy.max_agent_delay_ms,
                request.attempt,
            )
    } else {
        0
    };
    let attempt = request.attempt;
    let compacted = request.compacted;
    let modes = queue_modes(&settings);
    let now = runtime.now();
    let compacts = !retry && !overflow && compacts_after(runtime, request, &message, cancel)?;
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut boundary = prepare_boundary(tx, conversation_id, modes)?;
            append_assistant(tx, conversation_id, &message)?;
            if compacts {
                create_answer_compaction(tx, conversation_id, task_id)?;
            }
            if retry {
                edit_live(tx, conversation_id, |live| {
                    live.generation = Some(GenerationStatus {
                        attempt,
                        message: None,
                        retry: Some(RetryStatus {
                            at: until,
                            error: message.error_message.clone().unwrap_or_default(),
                        }),
                        deferred: None,
                    });
                })?;
                return Ok(Some(NextTaskState::Sleeping {
                    checkpoint: GenerationCheckpoint::Retry {
                        attempt,
                        compacted,
                        until,
                    },
                    until,
                }));
            }
            let text = message.error_message.clone().unwrap_or_else(|| {
                format!(
                    "Model response ended with stop reason {}",
                    stop_reason_text(message.stop_reason)
                )
            });
            let mut live = read_live(tx, conversation_id)?;
            fail_run(
                tx,
                &mut boundary,
                &mut live,
                task_id,
                SubmissionSettlement::Unanswered {
                    reason: "model_error".to_owned(),
                    detail: Some(serde_json::Value::String(text.clone())),
                },
                now,
            )?;
            write_live(tx, conversation_id, &live)?;
            Ok(Some(NextTaskState::Terminal {
                outcome: TaskOutcome::Failed {
                    error: TaskOutcomeError {
                        message: text,
                        detail: Some(serde_json::json!({ "reason": "model_error" })),
                    },
                    result: None,
                },
            }))
        }),
        cancel,
    )
}

fn compacts_after(
    runtime: Runtime<'_>,
    request: &Request,
    answer: &AssistantMessage,
    cancel: &CancelToken,
) -> Result<bool> {
    let policy = runtime.settings().compaction;
    if !policy.enabled || answer.stop_reason == StopReason::Aborted {
        return Ok(false);
    }
    let models = runtime.models();
    let Some(model) = models
        .get_model(&answer.provider, &answer.model)
        .or_else(|| models.get_model(&request.model.provider, &request.model.model_id))
    else {
        return Ok(false);
    };
    let window = super::budgeted_window(
        model.context_window,
        runtime.agent(cancel)?.context_budget_tokens,
    );
    if window == 0 {
        return Ok(false);
    }
    let tokens = if measured(answer) {
        Some(calculate_context_tokens(&answer.usage))
    } else {
        let view = match &request.view {
            Some(view) => Arc::clone(view),
            None => runtime.context(Some(request.cutoff), cancel)?,
        };
        estimated(&view, answer)
    };
    Ok(
        tokens
            .is_some_and(|tokens| tokens as i128 > window as i128 - policy.reserve_tokens as i128),
    )
}

fn measured(message: &AssistantMessage) -> bool {
    !matches!(message.stop_reason, StopReason::Aborted | StopReason::Error)
        && calculate_context_tokens(&message.usage) > 0
}

fn estimated(view: &ContextView, answer: &AssistantMessage) -> Option<u64> {
    let answer = Message::Assistant(Arc::new(answer.clone()));
    let sum = |messages: &[Message]| {
        messages
            .iter()
            .chain([&answer])
            .map(estimate_message_tokens)
            .sum::<u64>()
    };
    let last = (0..view.entries.len()).rev().find_map(|index| {
        view.contributions[index]
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::Assistant(assistant) if measured(assistant) => {
                    Some((view.entries[index].id, assistant))
                }
                _ => None,
            })
    });
    let Some((entry, usage)) = last else {
        return Some(sum(&view.messages));
    };
    let compaction = view
        .head
        .as_ref()
        .filter(|head| head.kind == compaction_entry().kind())
        .map(|head| head.id);
    let edited = view
        .entries
        .iter()
        .filter(|entry| entry.edits.is_some())
        .map(|entry| entry.id)
        .max();
    let invalidated = match edited {
        Some(edited) => entry < edited.max(compaction.unwrap_or(edited)),
        None if compaction.is_some_and(|head| entry < head) => return None,
        None => false,
    };
    if invalidated {
        return Some(sum(&view.messages));
    }
    let after = view
        .messages
        .iter()
        .rposition(|message| matches!(message, Message::Assistant(a) if a == usage))
        .map_or(view.messages.len(), |index| index + 1);
    Some(calculate_context_tokens(&usage.usage) + sum(&view.messages[after..]))
}

fn stop_reason_text(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Pending => "pending",
        StopReason::Stop => "stop",
        StopReason::Length => "length",
        StopReason::ToolUse => "toolUse",
        StopReason::Error => "error",
        StopReason::Aborted => "aborted",
        StopReason::Deferred => "deferred",
    }
}

/// A reply cut by the output limit below the model's own maximum: hide it, then compact and
/// retry once when compaction can cut anything, or end the run without an answer otherwise.
fn recover_length(
    runtime: Runtime<'_>,
    request: &Request,
    message: AssistantMessage,
    cancel: &CancelToken,
) -> Result<()> {
    let settings = runtime.settings().clone();
    let view = runtime.context(Some(request.cutoff), cancel)?;
    let compactable = settings.compaction.enabled
        && select_cut(&view, settings.compaction.keep_recent_tokens).is_some();
    if !compactable {
        return crate::harness::generation::rounds::answer_with(runtime, message, true, cancel);
    }
    let conversation_id = runtime.conversation_id();
    let task_id = crate::harness::tool::erase(runtime.task_id());
    let attempt = request.attempt;
    runtime.commit(
        Box::new(move |tx, _task| {
            let entry = append_assistant(tx, conversation_id, &message)?;
            crate::harness::generation::omit_entry(tx, conversation_id, entry.id)?;
            edit_live(tx, conversation_id, |live| live.generation = None)?;
            let child = create_compaction(
                tx,
                conversation_id,
                CompactionReason::Overflow,
                Some(task_id),
            )?;
            Ok(Some(NextTaskState::Waiting {
                checkpoint: GenerationCheckpoint::Prepare {
                    attempt,
                    compacted: Some(child),
                    overflow: Some(
                        "Truncated response recovery failed after one compact-and-retry attempt."
                            .to_owned(),
                    ),
                    route: None,
                },
                on: vec![child],
                policy: JoinPolicy::AllSettled,
            }))
        }),
        cancel,
    )?;
    Ok(())
}
