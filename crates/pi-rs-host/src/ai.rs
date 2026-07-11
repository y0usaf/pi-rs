//! Provider-stream, cancellation, and model-registry mechanisms
//! exposed to Lua (WS4.1; registry bridge with PLAN 3a.3).

use std::sync::{Arc, Mutex, PoisonError};

use mlua::{AnyUserData, Function, Lua, Table, UserData, UserDataMethods, Value};
use pi_rs_ai::protocols::SimpleStreamOptions;
use pi_rs_ai::transport::AbortSignal;
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Model,
    ModelThinkingLevel, StopReason, TextContent, ThinkingLevelMap, Usage, now_ms,
};

use crate::auth::SharedStorage;
use crate::convert::{JSON_KEY_ORDER, json_to_lua, lua_to_json};
use crate::model_registry::ModelRegistry;

/// One registry instance per VM, sharing the `pi.auth` storage so
/// `/login` immediately changes model availability (the spec: the
/// session's `ModelRegistry` owns the process `AuthStorage`).
type SharedRegistry = Arc<Mutex<ModelRegistry>>;

fn lock_registry(registry: &SharedRegistry) -> std::sync::MutexGuard<'_, ModelRegistry> {
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Debug)]
pub(crate) struct LuaAbortSignal(pub(crate) AbortSignal);

pub(crate) fn signal_userdata(lua: &Lua, signal: AbortSignal) -> mlua::Result<AnyUserData> {
    lua.create_userdata(LuaAbortSignal(signal))
}

impl UserData for LuaAbortSignal {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_aborted", |_, this, ()| Ok(this.0.is_aborted()));
        methods.add_method("abort", |_, this, ()| {
            this.0.abort();
            Ok(())
        });
        methods.add_async_method("wait", |_, this, ()| async move {
            this.0.aborted().await;
            Ok(())
        });
    }
}

fn model_to_lua(lua: &Lua, model: &Model) -> mlua::Result<Value> {
    to_lua_json(lua, model)
}

pub(crate) fn install(lua: &Lua, pi: &Table, storage: SharedStorage) -> mlua::Result<()> {
    pi.set(
        "abort_signal",
        lua.create_function(|lua, ()| lua.create_userdata(LuaAbortSignal(AbortSignal::new())))?,
    )?;

    let ai = lua.create_table()?;
    // Catalog provider ids (the model catalog is data; spec:
    // `getProviders()`); consumed by the interactive login option policy.
    ai.set(
        "providers",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for provider in pi_rs_ai::registry::get_providers() {
                result.push(provider)?;
            }
            Ok(result)
        })?,
    )?;
    ai.set(
        "stream_simple",
        lua.create_async_function(
            |lua, (model, context, options, on_event): (Value, Value, Option<Table>, Function)| async move {
                let model: Model = from_lua_json(model, "model")?;
                let context = context_from_lua(context)?;
                let options = stream_options(options)?;
                let signal = options.base.signal.clone();
                let stream = match pi_rs_ai::registry::stream_simple(&model, &context, Some(options)) {
                    Ok(stream) => stream,
                    Err(error) => {
                        let message = failure_message(
                            &model,
                            if signal.as_ref().is_some_and(AbortSignal::is_aborted) {
                                StopReason::Aborted
                            } else {
                                StopReason::Error
                            },
                            error.to_string(),
                        );
                        let event = AssistantMessageEvent::Error {
                            reason: message.stop_reason,
                            error: message.clone(),
                        };
                        call_event(&lua, &on_event, &event).await?;
                        return to_lua_json(&lua, &message);
                    }
                };

                while let Some(event) = stream.next().await {
                    call_event(&lua, &on_event, &event).await?;
                }
                match stream.result().await {
                    Some(message) => to_lua_json(&lua, &message),
                    None => {
                        let message = failure_message(
                            &model,
                            StopReason::Error,
                            "event stream completed without a result".to_owned(),
                        );
                        to_lua_json(&lua, &message)
                    }
                }
            },
        )?,
    )?;
    // -----------------------------------------------------------------
    // Model-registry bridge (spec: `core/model-registry.ts` consumed by
    // the interactive frontend). Presentation and selection policy stay
    // Lua; the registry is catalog + auth mechanism.
    // -----------------------------------------------------------------
    let registry: SharedRegistry = {
        let guard = storage.try_lock().map_err(|_| {
            mlua::Error::runtime("auth storage locked during registry construction")
        })?;
        Arc::new(Mutex::new(ModelRegistry::new(&guard)))
    };

    // Spec `refresh()` — re-reads the catalog and OAuth model overrides.
    let st = Arc::clone(&storage);
    let reg = Arc::clone(&registry);
    ai.set(
        "registry_refresh",
        lua.create_async_function(move |_, ()| {
            let st = Arc::clone(&st);
            let reg = Arc::clone(&reg);
            async move {
                let guard = st.lock().await;
                lock_registry(&reg).refresh(&guard);
                Ok(())
            }
        })?,
    )?;

    // Spec `getError()` — models.json load errors (none until the
    // models.json half lands).
    let reg = Arc::clone(&registry);
    ai.set(
        "registry_error",
        lua.create_function(move |_, ()| Ok(lock_registry(&reg).get_error().map(str::to_owned)))?,
    )?;

    // Spec `getAvailable()` — models whose provider has auth configured.
    let st = Arc::clone(&storage);
    let reg = Arc::clone(&registry);
    ai.set(
        "available_models",
        lua.create_async_function(move |lua, ()| {
            let st = Arc::clone(&st);
            let reg = Arc::clone(&reg);
            async move {
                let guard = st.lock().await;
                let registry = lock_registry(&reg);
                let result = lua.create_table()?;
                for model in registry.get_available(&guard) {
                    result.push(model_to_lua(&lua, model)?)?;
                }
                Ok(result)
            }
        })?,
    )?;

    // Spec `find(provider, modelId)`.
    let reg = Arc::clone(&registry);
    ai.set(
        "find_model",
        lua.create_function(
            move |lua, (provider, model_id): (String, String)| match lock_registry(&reg)
                .find(&provider, &model_id)
            {
                Some(model) => model_to_lua(lua, model),
                None => Ok(Value::Nil),
            },
        )?,
    )?;

    // Spec `hasConfiguredAuth(model)`.
    let st = Arc::clone(&storage);
    let reg = Arc::clone(&registry);
    ai.set(
        "has_configured_auth",
        lua.create_async_function(move |_, model: Value| {
            let st = Arc::clone(&st);
            let reg = Arc::clone(&reg);
            async move {
                let model: Model = from_lua_json(model, "model")?;
                let guard = st.lock().await;
                Ok(lock_registry(&reg).has_configured_auth(&guard, &model))
            }
        })?,
    )?;

    // Spec `getSupportedThinkingLevels(model)` (`packages/ai/src/models.ts`)
    // — the thinking-level vocabulary a model accepts; consumed by the
    // agent-session thinking policy (PLAN 7.2). Duck-typed like the JS
    // original: only `reasoning` and `thinkingLevelMap` are read.
    ai.set(
        "supported_thinking_levels",
        lua.create_function(|lua, model: Table| {
            let (reasoning, map) = thinking_capabilities(&model)?;
            let result = lua.create_table()?;
            for level in pi_rs_ai_types::supported_thinking_levels_for(reasoning, map.as_ref()) {
                result.push(to_lua_json(lua, &level)?)?;
            }
            Ok(result)
        })?,
    )?;

    // Spec `clampThinkingLevel(model, level)` — nearest supported level.
    ai.set(
        "clamp_thinking_level",
        lua.create_function(|lua, (model, level): (Table, Value)| {
            let (reasoning, map) = thinking_capabilities(&model)?;
            let level: ModelThinkingLevel = from_lua_json(level, "thinking level")?;
            to_lua_json(
                lua,
                &pi_rs_ai_types::clamp_thinking_level_for(reasoning, map.as_ref(), level),
            )
        })?,
    )?;

    // Spec `isUsingOAuth(model)` — the footer's "(sub)" indicator.
    let st = Arc::clone(&storage);
    let reg = Arc::clone(&registry);
    ai.set(
        "is_using_oauth",
        lua.create_async_function(move |_, model: Value| {
            let st = Arc::clone(&st);
            let reg = Arc::clone(&reg);
            async move {
                let model: Model = from_lua_json(model, "model")?;
                let guard = st.lock().await;
                Ok(lock_registry(&reg).is_using_oauth(&guard, &model))
            }
        })?,
    )?;

    pi.set("ai", ai)?;
    Ok(())
}

/// The `reasoning` + `thinkingLevelMap` slice of a model table. JSON
/// `null` map entries decode to absent Lua keys; the decode-order
/// metatable ([`JSON_KEY_ORDER`]) still lists them, so an order-listed
/// key without a value is reconstructed as the spec's explicit-null
/// "level unsupported" marker (120 catalog rows carry such entries).
fn thinking_capabilities(model: &Table) -> mlua::Result<(bool, Option<ThinkingLevelMap>)> {
    let reasoning = model.get::<Option<bool>>("reasoning")?.unwrap_or(false);
    let Some(map_table) = model.get::<Option<Table>>("thinkingLevelMap")? else {
        return Ok((reasoning, None));
    };
    let parse_level = |key: &str| -> Option<ModelThinkingLevel> {
        serde_json::from_value(serde_json::Value::String(key.to_owned())).ok()
    };
    let mut map = ThinkingLevelMap::new();
    for pair in map_table.pairs::<String, Value>() {
        let (key, value) = pair?;
        let Some(level) = parse_level(&key) else {
            continue;
        };
        if let Value::String(effort) = value {
            map.insert(level, Some(effort.to_str()?.to_owned()));
        }
    }
    if let Some(order) = map_table
        .metatable()
        .and_then(|meta| meta.raw_get::<Table>(JSON_KEY_ORDER).ok())
    {
        for key in order.sequence_values::<String>() {
            let key = key?;
            if let Some(level) = parse_level(&key) {
                map.entry(level).or_insert(None);
            }
        }
    }
    Ok((reasoning, Some(map)))
}

fn stream_options(options: Option<Table>) -> mlua::Result<SimpleStreamOptions> {
    let mut result = SimpleStreamOptions::default();
    let Some(options) = options else {
        return Ok(result);
    };
    result.base.api_key = options.get("apiKey")?;
    result.base.max_tokens = options.get("maxTokens")?;
    // Spec `SimpleStreamOptions.reasoning` — compaction/summarization
    // requests carry the session thinking level (PLAN 6.5).
    if let Some(reasoning) = options.get::<Option<String>>("reasoning")? {
        result.reasoning = Some(
            serde_json::from_value(serde_json::Value::String(reasoning)).map_err(|error| {
                mlua::Error::runtime(format!("invalid reasoning level: {error}"))
            })?,
        );
    }
    result.base.session_id = options.get("sessionId")?;
    result.base.max_retries = options.get("maxRetries")?;
    result.base.max_retry_delay_ms = options.get("maxRetryDelayMs")?;
    result.base.timeout_ms = options.get("timeoutMs")?;
    if let Some(signal) = options.get::<Option<AnyUserData>>("signal")? {
        result.base.signal = Some(signal.borrow::<LuaAbortSignal>()?.0.clone());
    }
    Ok(result)
}

fn context_from_lua(value: Value) -> mlua::Result<Context> {
    let mut json = lua_to_json(value).map_err(|error| mlua::Error::runtime(error.to_string()))?;
    // Lua has one empty-table value for both `{}` and `[]`; these Context
    // fields are arrays by contract, so normalize empty maps at this typed edge.
    if let Some(object) = json.as_object_mut() {
        for key in ["messages", "tools"] {
            if object
                .get(key)
                .is_some_and(|value| value.as_object().is_some_and(|map| map.is_empty()))
            {
                object.insert(key.to_owned(), serde_json::Value::Array(Vec::new()));
            }
        }
    }
    serde_json::from_value(json)
        .map_err(|error| mlua::Error::runtime(format!("invalid context: {error}")))
}

fn from_lua_json<T: serde::de::DeserializeOwned>(value: Value, label: &str) -> mlua::Result<T> {
    let json = lua_to_json(value).map_err(|error| mlua::Error::runtime(error.to_string()))?;
    serde_json::from_value(json)
        .map_err(|error| mlua::Error::runtime(format!("invalid {label}: {error}")))
}

fn to_lua_json<T: serde::Serialize>(lua: &Lua, value: &T) -> mlua::Result<Value> {
    let json =
        serde_json::to_value(value).map_err(|error| mlua::Error::runtime(error.to_string()))?;
    json_to_lua(lua, &json)
}

async fn call_event(
    lua: &Lua,
    callback: &Function,
    event: &AssistantMessageEvent,
) -> mlua::Result<()> {
    let value = to_lua_json(lua, event)?;
    callback.call_async::<()>(value).await
}

fn failure_message(model: &Model, reason: StopReason, error: String) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContent::Text(TextContent::new(""))],
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_model: None,
        response_id: None,
        diagnostics: None,
        usage: Usage::default(),
        stop_reason: reason,
        error_message: Some(error),
        timestamp: now_ms(),
    }
}
