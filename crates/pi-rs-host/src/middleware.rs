//! Generic root-dispatch middleware pipelines.
//!
//! Middleware is mechanism, not product policy: any package may register a
//! bounded handler stage before the resolved root (`"event"`) or a bounded
//! transform stage after it (`"render"`). Stages are ordinary Lua functions
//! that receive immutable snapshot-shaped data and return plain JSON data.
//! Rust owns ordering (ascending `order`, then registration sequence),
//! per-stage watchdog bounds, and short-circuit semantics; every payload's
//! meaning stays Lua policy.
//!
//! Event middleware runs before the root handler. Each stage receives
//! `{ version, root, phase = "event", event, context, actions }` and returns
//! a table with an optional replacement `event` and/or replacement `actions`
//! (queued actions so far). A stage returns `{ stop = true }` to skip the
//! remaining stages and the root handler; the queued actions then become the
//! dispatch batch. A stopped chain with no queued actions yields an empty
//! batch — a deliberate suppress.
//!
//! Render middleware runs after the root handler succeeds and transforms the
//! settled action list. Each stage receives
//! `{ version, root, phase = "render", event, actions }` and returns a table
//! whose optional `actions` array replaces the list; an explicit empty array
//! suppresses the whole batch. A failing transform rolls back the entire
//! dispatch: nothing publishes.
//!
//! Snapshot payloads are read-only views: a stage that keeps an action must
//! return it as a plain table, exactly as a root builds its own actions.
//!
//! Registration is scope-owned like roots and declarations: disposing a
//! package removes its stages, and a failed package load publishes nothing.
//! Re-registering the same id from the same source replaces that entry;
//! identical ids from different sources conflict deterministically.

use mlua::{Function, Table, Value};

use crate::kernel::{
    Action, Control, DispatchBatch, KERNEL_API_VERSION, MAX_BATCH_ITEMS, RootKind, ScopeId,
};
use crate::{HostConfig, HostError};

pub(crate) const EVENT_PHASE: &str = "event";
pub(crate) const RENDER_PHASE: &str = "render";
const MAX_MIDDLEWARE_STAGES: usize = 64;

struct ResolvedStage {
    sequence: u64,
    scope: ScopeId,
    source: String,
    handler: Function,
}

pub(crate) fn install(lua: &mlua::Lua, pi: &Table) -> mlua::Result<()> {
    let registry = crate::api::registry_table(lua)?;
    registry.set("kernel_middleware", lua.create_table()?)?;
    registry.set("kernel_middleware_sequence", 0_u64)?;

    let middleware = lua.create_table()?;
    middleware.set(
        "register",
        lua.create_function(|lua, definition: Table| {
            let kind: String = definition.get("kind")?;
            let kind = RootKind::parse(&kind).map_err(mlua::Error::external)?;
            let phase: String = definition
                .get::<Option<String>>("phase")?
                .unwrap_or_else(|| EVENT_PHASE.to_owned());
            if !matches!(phase.as_str(), EVENT_PHASE | RENDER_PHASE) {
                return Err(mlua::Error::runtime(
                    "middleware phase must be \"event\" or \"render\"",
                ));
            }
            let id: String = definition
                .get::<Option<String>>("id")?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| mlua::Error::runtime("middleware id must be a non-empty string"))?;
            let handler: Function = definition
                .get("handler")
                .map_err(|_| mlua::Error::runtime("middleware handler must be a function"))?;
            let order = definition.get::<Option<i64>>("order")?.unwrap_or(0);
            let source = crate::api::current_source(lua);
            let scope = crate::kernel_api::scope_for_current_entry(lua)?;

            let registry = crate::api::registry_table(lua)?;
            let entries: Table = registry.get("kernel_middleware")?;
            let key = format!("{}\0{phase}\0{id}", kind.as_str());
            if let Some(existing) = entries.get::<Option<Table>>(key.as_str())? {
                let other: String = existing.get("source")?;
                if other != source {
                    let mut sources = [source.clone(), other];
                    sources.sort_unstable();
                    return Err(mlua::Error::runtime(format!(
                        "{}middleware conflict for {}/{phase}/{id}: {} <> {}",
                        crate::error::CONFLICT_MARKER,
                        kind.as_str(),
                        sources[0],
                        sources[1]
                    )));
                }
            }
            let stage_count = entries.clone().pairs::<Value, Table>().count();
            if stage_count >= MAX_MIDDLEWARE_STAGES {
                return Err(mlua::Error::runtime(format!(
                    "middleware registrations exceed {MAX_MIDDLEWARE_STAGES} stages"
                )));
            }
            let sequence = registry.get::<u64>("kernel_middleware_sequence")?;
            registry.set("kernel_middleware_sequence", sequence + 1)?;
            let entry = lua.create_table()?;
            entry.set("root", kind.as_str())?;
            entry.set("phase", phase)?;
            entry.set("id", id)?;
            entry.set("order", order)?;
            entry.set("source", source)?;
            entry.set("scope", scope.get())?;
            entry.set("sequence", sequence)?;
            entry.set("handler", handler)?;
            entries.set(key, entry)
        })?,
    )?;

    let roots: Table = pi.get("roots")?;
    let v1: Table = roots.get("v1")?;
    v1.set("middleware", middleware)
}

/// Collect the registered stages for one root kind and phase in execution
/// order: ascending `order`, then registration sequence. Deterministic.
fn resolve_stages(
    lua: &mlua::Lua,
    root: RootKind,
    phase: &str,
) -> Result<Vec<ResolvedStage>, HostError> {
    let registry =
        crate::api::registry_table(lua).map_err(|error| HostError::Lua(error.to_string()))?;
    let entries: Table = registry
        .get("kernel_middleware")
        .map_err(|error| HostError::Lua(error.to_string()))?;
    let mut collected = Vec::new();
    for pair in entries.pairs::<Value, Table>() {
        let (_, entry) = pair.map_err(|error| HostError::Lua(error.to_string()))?;
        let matches = entry
            .get::<String>("root")
            .map_err(|error| HostError::Lua(error.to_string()))?
            == root.as_str()
            && entry
                .get::<String>("phase")
                .map_err(|error| HostError::Lua(error.to_string()))?
                == phase;
        if matches {
            collected.push((
                entry
                    .get::<i64>("order")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                entry
                    .get::<u64>("sequence")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                ResolvedStage {
                    sequence: entry
                        .get("sequence")
                        .map_err(|error| HostError::Lua(error.to_string()))?,
                    scope: ScopeId::from_raw(
                        entry
                            .get("scope")
                            .map_err(|error| HostError::Lua(error.to_string()))?,
                    ),
                    source: entry
                        .get("source")
                        .map_err(|error| HostError::Lua(error.to_string()))?,
                    handler: entry
                        .get("handler")
                        .map_err(|error| HostError::Lua(error.to_string()))?,
                },
            ));
        }
    }
    collected.sort_by_key(|(order, sequence, _)| (*order, *sequence));
    Ok(collected.into_iter().map(|(_, _, stage)| stage).collect())
}

pub(crate) struct EventPipeline {
    /// `true` when a stage stopped the chain: the root handler is skipped and
    /// the queued actions become the dispatch batch.
    pub(crate) stopped: bool,
    /// The event after every event stage's replacements.
    pub(crate) event: serde_json::Value,
    /// Actions queued by event stages, published when the chain stopped.
    pub(crate) actions: Vec<Action>,
}

fn stage_snapshot(
    root: RootKind,
    phase: &str,
    event: &serde_json::Value,
    context: Option<&serde_json::Value>,
    actions: &[serde_json::Value],
) -> serde_json::Value {
    match context {
        Some(context) => serde_json::json!({
            "version": KERNEL_API_VERSION,
            "root": root.as_str(),
            "phase": phase,
            "event": event,
            "context": context,
            "actions": actions,
        }),
        None => serde_json::json!({
            "version": KERNEL_API_VERSION,
            "root": root.as_str(),
            "phase": phase,
            "event": event,
            "actions": actions,
        }),
    }
}

async fn run_stage(
    lua: &mlua::Lua,
    config: &HostConfig,
    control: &Control,
    root: RootKind,
    phase: &str,
    stage: &ResolvedStage,
    snapshot: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, HostError> {
    let token = control.token(stage.scope)?;
    let outer_scope = crate::api::registry_table(lua)
        .and_then(|registry| registry.get::<u64>("kernel_scope"))
        .unwrap_or(0);
    let outer_source = crate::api::current_source(lua);
    crate::kernel_api::set_scope(lua, Some(stage.scope))
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::api::set_current_source(lua, &stage.source);
    let argument = crate::convert::immutable_json_to_lua(lua, &snapshot)
        .map_err(|error| HostError::Lua(error.to_string()));
    let result = match argument {
        Ok(argument) => {
            crate::vm::dispatch_function_async(
                lua,
                config,
                stage.handler.clone(),
                argument,
                Some(token),
            )
            .await
        }
        Err(error) => Err(error),
    };
    crate::api::set_current_source(lua, &outer_source);
    let restore = if outer_scope == 0 {
        None
    } else {
        Some(ScopeId::from_raw(outer_scope))
    };
    let _ = crate::kernel_api::set_scope(lua, restore);
    let value = result?;
    let returned = crate::convert::lua_to_json_strict(value)
        .map_err(|error| HostError::Lua(error.to_string()))?;
    match returned {
        serde_json::Value::Null => Ok(serde_json::Map::new()),
        serde_json::Value::Object(table) => Ok(table),
        _ => Err(HostError::Lua(format!(
            "middleware {}/{phase} stage {} must return a table or nil",
            root.as_str(),
            stage.sequence
        ))),
    }
}

fn replacement_actions(
    stage: &ResolvedStage,
    root: RootKind,
    phase: &str,
    values: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, HostError> {
    if values.len() > MAX_BATCH_ITEMS {
        return Err(HostError::Lua(format!(
            "middleware {}/{phase} stage {} returned more than {MAX_BATCH_ITEMS} actions",
            root.as_str(),
            stage.sequence
        )));
    }
    for value in values {
        let valid = value
            .get("kind")
            .and_then(|kind| kind.as_str())
            .is_some_and(|kind| !kind.trim().is_empty());
        if !valid {
            return Err(HostError::Lua(format!(
                "middleware {}/{phase} stage {} returned an action without a kind",
                root.as_str(),
                stage.sequence
            )));
        }
    }
    Ok(values.to_vec())
}

fn json_actions(actions: &[Action]) -> Vec<serde_json::Value> {
    actions
        .iter()
        .map(|action| {
            serde_json::json!({
                "kind": action.kind,
                "payload": action.payload,
            })
        })
        .collect()
}

/// Run the event middleware chain for one root kind. Each stage sees the
/// previous stage's replacement event and the actions queued so far. The
/// chain stops early on `{ stop = true }`.
pub(crate) async fn run_event_pipeline(
    lua: &mlua::Lua,
    config: &HostConfig,
    control: &Control,
    root: RootKind,
    event: serde_json::Value,
    context: serde_json::Value,
) -> Result<EventPipeline, HostError> {
    let stages = resolve_stages(lua, root, EVENT_PHASE)?;
    let mut pipeline = EventPipeline {
        stopped: false,
        event,
        actions: Vec::new(),
    };
    for stage in stages {
        let snapshot = stage_snapshot(
            root,
            EVENT_PHASE,
            &pipeline.event,
            Some(&context),
            &json_actions(&pipeline.actions),
        );
        let returned = run_stage(lua, config, control, root, EVENT_PHASE, &stage, snapshot).await?;
        if let Some(replacement) = returned.get("event") {
            pipeline.event = replacement.clone();
        }
        if let Some(values) = returned.get("actions").and_then(|value| value.as_array()) {
            let values = replacement_actions(&stage, root, EVENT_PHASE, values)?;
            pipeline.actions = values
                .into_iter()
                .enumerate()
                .map(|(index, value)| Action {
                    sequence: index as u64,
                    kind: value
                        .get("kind")
                        .and_then(|kind| kind.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    payload: value
                        .get("payload")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })
                .collect();
        }
        if returned.get("stop").and_then(|value| value.as_bool()) == Some(true) {
            pipeline.stopped = true;
            break;
        }
    }
    Ok(pipeline)
}

/// Run the render transform chain over a settled batch. A failing transform
/// errors the whole dispatch so nothing publishes; an explicit empty
/// `actions` array suppresses every action.
pub(crate) async fn apply_render_pipeline(
    lua: &mlua::Lua,
    config: &HostConfig,
    control: &Control,
    root: RootKind,
    event: &serde_json::Value,
    mut batch: DispatchBatch,
) -> Result<DispatchBatch, HostError> {
    let stages = resolve_stages(lua, root, RENDER_PHASE)?;
    if stages.is_empty() {
        return Ok(batch);
    }
    let mut actions = json_actions(&batch.actions);
    for stage in stages {
        let snapshot = stage_snapshot(root, RENDER_PHASE, event, None, &actions);
        let returned =
            run_stage(lua, config, control, root, RENDER_PHASE, &stage, snapshot).await?;
        if let Some(values) = returned.get("actions").and_then(|value| value.as_array()) {
            actions = replacement_actions(&stage, root, RENDER_PHASE, values)?;
        }
    }
    batch.actions = actions
        .into_iter()
        .enumerate()
        .map(|(index, value)| Action {
            sequence: index as u64,
            kind: value
                .get("kind")
                .and_then(|kind| kind.as_str())
                .unwrap_or_default()
                .to_owned(),
            payload: value
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        })
        .collect();
    Ok(batch)
}

/// Remove every stage owned by a disposed or rolled-back package scope.
pub(crate) fn remove_scope(lua: &mlua::Lua, scope: ScopeId) -> mlua::Result<()> {
    let registry = crate::api::registry_table(lua)?;
    let entries: Table = registry.get("kernel_middleware")?;
    let mut dropped = Vec::new();
    for pair in entries.clone().pairs::<Value, Table>() {
        let (key, entry) = pair?;
        if entry.get::<u64>("scope")? == scope.get() {
            dropped.push(key);
        }
    }
    for key in dropped {
        entries.set(key, Value::Nil)?;
    }
    Ok(())
}
