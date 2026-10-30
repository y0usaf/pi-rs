use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::ai::types::{
    AssistantBlock, AssistantMessage, Message, ToolCall, UserContent, UserMessage, UserRole,
};
use crate::ai::utils::transcript::get_current_tools;
use crate::cancel::CancelToken;
use crate::entries::{reset_entry, user_entry};
use crate::errors::{Error, Result};
use crate::harness::agent::add_tools;
use crate::harness::generation::classify::Request;
use crate::harness::generation::{
    GenerationCheckpoint, GenerationResult, Runtime, append_assistant, create_generation,
    create_tool_task, end_run, hand_over, queue_modes, read_live, snapshot_live, start_run,
    write_live,
};
use crate::harness::inbox::{Phase, apply_boundary, prepare_boundary};
use crate::harness::live::{SlotStatus, ToolSlot};
use crate::harness::tool::{ToolTaskResult, erase};
use crate::harness::types::{HookApi, ToolControl, ToolExecutionMode};
use crate::ids::{EntryId, TaskId};
use crate::tasks::{JoinPolicy, NextTaskState, RunningTask, TaskOutcome};
use crate::types::{EntryDraft, SubmissionSettlement};

pub fn read_calls(
    runtime: Runtime<'_>,
    assistant: EntryId,
    call_ids: &[String],
    cancel: &CancelToken,
) -> Result<Vec<ToolCall>> {
    let Some(record) = runtime.entry(assistant, cancel)? else {
        return Ok(Vec::new());
    };
    let calls: Vec<ToolCall> = match record.model.as_deref() {
        Some([Message::Assistant(message)]) => message
            .content
            .iter()
            .filter_map(|block| match block {
                AssistantBlock::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(call_ids
        .iter()
        .filter_map(|id| calls.iter().find(|call| &call.id == id).cloned())
        .collect())
}

pub fn answer(runtime: Runtime<'_>, message: AssistantMessage, cancel: &CancelToken) -> Result<()> {
    answer_with(runtime, message, false, cancel)
}

/// Ends the run on `message`; with `omit`, the reply is kept in history but hidden from
/// the model's context.
pub fn answer_with(
    runtime: Runtime<'_>,
    message: AssistantMessage,
    omit: bool,
    cancel: &CancelToken,
) -> Result<()> {
    if answer_hooked(runtime)? {
        return store_answer(runtime, message, omit, cancel);
    }
    let continuation = yielded(runtime, &message)?;
    let conversation_id = runtime.conversation_id();
    let task_id = erase(runtime.task_id());
    let modes = queue_modes(runtime.settings());
    let now = runtime.now();
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut boundary = prepare_boundary(tx, conversation_id, modes)?;
            let mut live = read_live(tx, conversation_id)?;
            let entry = append_assistant(tx, conversation_id, &message)?;
            if omit {
                crate::harness::generation::omit_entry(tx, conversation_id, entry.id)?;
            }
            close_answer(
                tx,
                &mut boundary,
                &mut live,
                Closing {
                    task_id,
                    entry: entry.id,
                    continuation: continuation.as_ref(),
                    proceed: false,
                    now,
                },
            )
        }),
        cancel,
    )
}

fn yielded(runtime: Runtime<'_>, message: &AssistantMessage) -> Result<Option<UserContent>> {
    let mut continuation: Option<UserContent> = None;
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    runtime.hooks().each(&mut |hooks| {
        if continuation.is_some() {
            return Ok(());
        }
        if let Some(hook) = hooks.on_yield.as_ref() {
            continuation = hook(&api, message)?;
        }
        Ok(())
    })?;
    Ok(continuation)
}

fn answer_hooked(runtime: Runtime<'_>) -> Result<bool> {
    let mut hooked = false;
    runtime.hooks().each(&mut |hooks| {
        hooked |= hooks.after_answer.is_some();
        Ok(())
    })?;
    Ok(hooked)
}

fn store_answer(
    runtime: Runtime<'_>,
    message: AssistantMessage,
    omit: bool,
    cancel: &CancelToken,
) -> Result<()> {
    let conversation_id = runtime.conversation_id();
    runtime.commit(
        Box::new(move |tx, _task| {
            let entry = append_assistant(tx, conversation_id, &message)?;
            if omit {
                crate::harness::generation::omit_entry(tx, conversation_id, entry.id)?;
            }
            crate::harness::generation::edit_live(tx, conversation_id, |live| {
                live.generation = None
            })?;
            Ok(Some(NextTaskState::Running {
                checkpoint: GenerationCheckpoint::Answered {
                    assistant: entry.id,
                },
            }))
        }),
        cancel,
    )
}

pub fn answered_phase(
    task: RunningTask<super::GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let GenerationCheckpoint::Answered { assistant } = task.checkpoint().clone() else {
        return Err(Error::Invalid(
            "generation phase handler answered got another checkpoint".to_owned(),
        ));
    };
    let stored = runtime
        .entry(assistant, cancel)?
        .and_then(|record| record.model)
        .and_then(|mut messages| messages.pop());
    let continuation = match &stored {
        Some(Message::Assistant(message)) => yielded(runtime, message)?,
        _ => None,
    };
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    let mut proceed = false;
    runtime.hooks().each(&mut |hooks| {
        if let Some(hook) = hooks.after_answer.as_ref() {
            proceed |= hook(&api, assistant)?;
        }
        Ok(())
    })?;
    let conversation_id = runtime.conversation_id();
    let task_id = erase(runtime.task_id());
    let modes = queue_modes(runtime.settings());
    let now = runtime.now();
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut boundary = prepare_boundary(tx, conversation_id, modes)?;
            let mut live = read_live(tx, conversation_id)?;
            close_answer(
                tx,
                &mut boundary,
                &mut live,
                Closing {
                    task_id,
                    entry: assistant,
                    continuation: continuation.as_ref(),
                    proceed,
                    now,
                },
            )
        }),
        cancel,
    )
}

struct Closing<'a> {
    task_id: TaskId,
    entry: EntryId,
    continuation: Option<&'a UserContent>,
    proceed: bool,
    now: u64,
}

fn close_answer(
    tx: &mut dyn crate::session::types::Tx,
    boundary: &mut crate::harness::inbox::Boundary,
    live: &mut crate::harness::live::LiveState,
    closing: Closing<'_>,
) -> Result<Option<NextTaskState<GenerationCheckpoint, GenerationResult>>> {
    let conversation_id = boundary.conversation_id;
    let result = apply_boundary(tx, boundary, Phase::Final, closing.now)?;
    let idle = result.users.is_empty() && !result.reset;
    if idle && (closing.continuation.is_some() || closing.proceed) {
        if let Some(content) = closing.continuation {
            tx.append_entry(
                conversation_id,
                EntryDraft {
                    kind: user_entry().kind().to_owned(),
                    model: Some(vec![Message::User(Arc::new(UserMessage {
                        role: UserRole,
                        content: content.clone(),
                        timestamp: closing.now,
                    }))]),
                    data: None,
                    head: None,
                    edits: None,
                },
            )?;
        }
        let successor = create_generation(tx, conversation_id)?;
        hand_over(live, closing.task_id, successor);
        live.generation = None;
    } else {
        end_run(
            tx,
            live,
            closing.task_id,
            SubmissionSettlement::Done {
                answer: closing.entry,
            },
        )?;
        if !result.users.is_empty() {
            start_run(tx, conversation_id, live, result.users)?;
        }
    }
    write_live(tx, conversation_id, live)?;
    Ok(Some(NextTaskState::Terminal {
        outcome: TaskOutcome::Completed {
            result: GenerationResult {
                entry_id: closing.entry,
            },
        },
    }))
}

pub fn start_tool_round(
    runtime: Runtime<'_>,
    request: &Request,
    message: AssistantMessage,
    calls: Vec<ToolCall>,
    cancel: &CancelToken,
) -> Result<()> {
    let conversation_id = runtime.conversation_id();
    let view = match &request.view {
        Some(view) => Arc::clone(view),
        None => runtime.context(Some(request.cutoff), cancel)?,
    };
    let offered: BTreeSet<String> = get_current_tools(&view.messages)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let agent = runtime.agent(cancel)?;
    let sequential = runtime.settings().tool_execution
        == crate::harness::types::ToolExecutionMode::Sequential
        || calls.iter().any(|call| {
            offered.contains(&call.name)
                && agent
                    .tools
                    .iter()
                    .find(|tool| tool.tool.name == call.name)
                    .is_some_and(|tool| tool.execution_mode == Some(ToolExecutionMode::Sequential))
        });
    let owner = erase(runtime.task_id());
    let now = runtime.now();
    let tool_errors = runtime.settings().tool_errors;
    super::classify::before_tools(
        runtime,
        &crate::harness::tool::ToolRound {
            conversation: conversation_id,
            calls: &calls,
            sequential,
            offered: &offered,
            agent: &agent,
            style: tool_errors,
        },
    )?;
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut live = read_live(tx, conversation_id)?;
            let entry = append_assistant(tx, conversation_id, &message)?;
            let mut slots = Vec::new();
            let mut tasks: Vec<TaskId> = Vec::new();
            let mut pending: Vec<String> = Vec::new();
            for call in &calls {
                if !offered.contains(&call.name) {
                    let unavailable =
                        crate::harness::tool::unavailable_tool(tool_errors, &call.name);
                    let result = crate::harness::tool::append_tool_result(
                        tx,
                        conversation_id,
                        call,
                        &unavailable,
                        now,
                        None,
                    )?;
                    slots.push(slot(call, None, SlotStatus::Done, Some(result.id)));
                    continue;
                }
                if sequential && !tasks.is_empty() {
                    pending.push(call.id.clone());
                    slots.push(slot(call, None, SlotStatus::Pending, None));
                    continue;
                }
                let task = create_tool_task(tx, owner, entry.id, &call.id)?;
                tasks.push(task);
                slots.push(slot(call, Some(task), SlotStatus::Pending, None));
            }
            live.generation = None;
            live.tools = Some(slots);
            write_live(tx, conversation_id, &live)?;
            Ok(Some(NextTaskState::Waiting {
                checkpoint: GenerationCheckpoint::Tools {
                    assistant: entry.id,
                    tools: tasks.clone(),
                    pending,
                },
                on: tasks,
                policy: JoinPolicy::AllSettled,
            }))
        }),
        cancel,
    )
}

fn slot(
    call: &ToolCall,
    task_id: Option<TaskId>,
    status: SlotStatus,
    entry: Option<EntryId>,
) -> ToolSlot {
    ToolSlot {
        call_id: call.id.clone(),
        name: call.name.clone(),
        task_id,
        status,
        output: None,
        dropped_bytes: None,
        dropped_lines: None,
        details: None,
        diagnostics: None,
        entry,
    }
}

pub fn tools_phase(
    task: RunningTask<super::GenerationInput, GenerationCheckpoint, GenerationResult>,
    runtime: Runtime<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let GenerationCheckpoint::Tools {
        assistant,
        tools,
        pending,
    } = task.checkpoint().clone()
    else {
        return Err(Error::Invalid(
            "generation phase handler tools got another checkpoint".to_owned(),
        ));
    };
    let Some((next, rest)) = pending.split_first() else {
        return finish_tool_round(runtime, assistant, &tools, cancel);
    };
    let next = next.clone();
    let rest = rest.to_vec();
    let conversation_id = runtime.conversation_id();
    let owner = erase(runtime.task_id());
    runtime.commit(
        Box::new(move |tx, _task| {
            let task_id = create_tool_task(tx, owner, assistant, &next)?;
            let mut live = read_live(tx, conversation_id)?;
            if let Some(slot) = live.tools.as_mut().and_then(|slots| {
                slots
                    .iter_mut()
                    .find(|slot| slot.call_id == next && slot.task_id.is_none())
            }) {
                slot.task_id = Some(task_id);
            }
            write_live(tx, conversation_id, &live)?;
            let mut all = tools.clone();
            all.push(task_id);
            Ok(Some(NextTaskState::Waiting {
                checkpoint: GenerationCheckpoint::Tools {
                    assistant,
                    tools: all,
                    pending: rest.clone(),
                },
                on: vec![task_id],
                policy: JoinPolicy::AllSettled,
            }))
        }),
        cancel,
    )
}

pub fn finish_tool_round(
    runtime: Runtime<'_>,
    assistant: EntryId,
    tools: &[TaskId],
    cancel: &CancelToken,
) -> Result<()> {
    let conversation_id = runtime.conversation_id();
    let outcomes = runtime.outcomes(tools, cancel)?;
    let mut controls: BTreeMap<TaskId, ToolControl> = BTreeMap::new();
    let mut entries: BTreeMap<TaskId, EntryId> = BTreeMap::new();
    for (id, outcome) in tools.iter().zip(outcomes) {
        if let TaskOutcome::Completed { result } = outcome {
            let parsed: ToolTaskResult = serde_json::from_value(result)?;
            entries.insert(*id, parsed.entry_id);
            if let Some(control) = parsed.control {
                controls.insert(*id, control);
            }
        }
    }
    let live = snapshot_live(runtime, cancel)?;
    let slots = live.tools.clone().unwrap_or_default();
    let results: Vec<EntryId> = slots
        .iter()
        .filter_map(|slot| {
            slot.entry
                .or_else(|| slot.task_id.and_then(|id| entries.get(&id).copied()))
        })
        .collect();
    let api = HookApi {
        conversation_id: runtime.conversation_id(),
    };
    runtime.hooks().each(&mut |hooks| {
        if let Some(hook) = hooks.after_tools.as_ref() {
            hook(&api, assistant, &results)?;
        }
        Ok(())
    })?;
    let terminate = !slots.is_empty()
        && slots.iter().all(|slot| {
            slot.task_id
                .and_then(|id| controls.get(&id))
                .is_some_and(|control| control.terminate)
        });
    let added: Vec<String> = controls
        .values()
        .flat_map(|control| control.add_tools.clone().unwrap_or_default())
        .collect();
    let handoff = controls
        .values()
        .rev()
        .find_map(|control| control.handoff.clone());
    let task_id = erase(runtime.task_id());
    let modes = queue_modes(runtime.settings());
    let now = runtime.now();
    runtime.commit(
        Box::new(move |tx, _task| {
            let mut boundary = prepare_boundary(tx, conversation_id, modes)?;
            if !added.is_empty() {
                add_tools(tx, conversation_id, &added)?;
            }
            let mut live = read_live(tx, conversation_id)?;
            if terminate || handoff.is_some() {
                if let Some(content) = handoff.clone() {
                    let entry = tx.append_entry(
                        conversation_id,
                        EntryDraft {
                            kind: reset_entry().kind().to_owned(),
                            model: Some(vec![Message::User(Arc::new(UserMessage {
                                role: UserRole,
                                content: UserContent::Text(content),
                                timestamp: now,
                            }))]),
                            data: None,
                            head: Some(crate::types::EntryHead::SelfHead),
                            edits: None,
                        },
                    )?;
                    boundary.head = Some(entry.id);
                }
                let result = apply_boundary(tx, &mut boundary, Phase::Final, now)?;
                end_run(
                    tx,
                    &mut live,
                    task_id,
                    SubmissionSettlement::Done { answer: assistant },
                )?;
                if !result.users.is_empty() {
                    start_run(tx, conversation_id, &mut live, result.users)?;
                }
            } else {
                let result = apply_boundary(tx, &mut boundary, Phase::PostTools, now)?;
                if result.reset {
                    end_run(
                        tx,
                        &mut live,
                        task_id,
                        SubmissionSettlement::Unanswered {
                            reason: "reset".to_owned(),
                            detail: None,
                        },
                    )?;
                    if !result.users.is_empty() {
                        start_run(tx, conversation_id, &mut live, result.users)?;
                    }
                } else {
                    live.tools = None;
                    if let Some(run) = live.run.as_mut().filter(|run| run.task_id == task_id) {
                        run.inputs.extend(result.users);
                    }
                    let successor = create_generation(tx, conversation_id)?;
                    hand_over(&mut live, task_id, successor);
                }
            }
            write_live(tx, conversation_id, &live)?;
            Ok(Some(NextTaskState::Terminal {
                outcome: TaskOutcome::Completed {
                    result: GenerationResult {
                        entry_id: assistant,
                    },
                },
            }))
        }),
        cancel,
    )
}
