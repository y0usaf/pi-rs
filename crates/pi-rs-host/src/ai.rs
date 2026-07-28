//! Private provider-stream and abort-signal bridge used by versioned bindings.

use std::sync::Arc;

use mlua::{AnyUserData, Function, Lua, Table, UserData, UserDataMethods, Value};
use pi_rs_ai::protocols::SimpleStreamOptions;
use pi_rs_ai::transport::AbortSignal;
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Model,
    ProviderResponse, StopReason, TextContent, Usage, now_ms,
};
use tokio::sync::{mpsc, oneshot};

use crate::convert::{json_to_lua, lua_to_json};

enum ProviderHookRequest {
    Payload {
        payload: serde_json::Value,
        model: Model,
        reply: oneshot::Sender<Option<serde_json::Value>>,
    },
    Response {
        response: ProviderResponse,
        model: Model,
        reply: oneshot::Sender<()>,
    },
}

struct ProviderHookCallbacks {
    payload: Option<Function>,
    response: Option<Function>,
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

pub(crate) fn install(lua: &Lua, bridge: &Table) -> mlua::Result<()> {
    let ai = lua.create_table()?;
    ai.set(
        "find_model",
        lua.create_function(|lua, (provider, model_id): (String, String)| {
            match pi_rs_ai::registry::get_model(&provider, &model_id) {
                Some(model) => to_lua_json(lua, model),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    // Catalog inventory. The catalog is reviewed mechanism data: which rows
    // exist, and which of them a package selects, are two different questions,
    // and only the first one is answered here.
    ai.set(
        "list_providers",
        lua.create_function(|lua, ()| {
            lua.create_sequence_from(pi_rs_ai::registry::get_providers())
        })?,
    )?;
    ai.set(
        "list_models",
        lua.create_function(|lua, (provider, offset, limit): (String, usize, usize)| {
            let models = pi_rs_ai::registry::get_models(&provider);
            let window = models
                .iter()
                .skip(offset)
                .take(limit)
                .map(|model| to_lua_json(lua, model))
                .collect::<mlua::Result<Vec<Value>>>()?;
            Ok((lua.create_sequence_from(window)?, models.len()))
        })?,
    )?;
    // The advertised wire-protocol families a model row may dispatch through.
    ai.set(
        "list_apis",
        lua.create_function(|lua, ()| {
            pi_rs_ai::registry::ensure_builtin_api_providers();
            lua.create_sequence_from(
                pi_rs_ai::registry::get_api_providers()
                    .into_iter()
                    .map(|provider| provider.api),
            )
        })?,
    )?;
    // Wire-schema validation for a package-authored model row: the same shape
    // `find_model` returns, checked once at declaration time instead of failing
    // mid-stream. No row is stored here; where declarations live is Lua policy.
    ai.set(
        "validate_model",
        lua.create_function(|lua, value: Value| {
            pi_rs_ai::registry::ensure_builtin_api_providers();
            let model: Model = from_lua_json(value, "model")?;
            if pi_rs_ai::registry::get_api_provider(&model.api).is_none() {
                let supported = pi_rs_ai::registry::get_api_providers()
                    .into_iter()
                    .map(|provider| provider.api)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(mlua::Error::runtime(format!(
                    "invalid model: unsupported api {}; supported: {supported}",
                    model.api
                )));
            }
            if model.id.is_empty() || model.provider.is_empty() || model.base_url.is_empty() {
                return Err(mlua::Error::runtime(
                    "invalid model: id, provider, and baseUrl must be non-empty".to_owned(),
                ));
            }
            to_lua_json(lua, &model)
        })?,
    )?;
    ai.set(
        "stream_simple",
        lua.create_async_function(
            |lua,
             (model, context, options, on_event): (
                Value,
                Value,
                Option<Table>,
                Function,
            )| async move {
                let model: Model = from_lua_json(model, "model")?;
                let context = context_from_lua(context)?;
                let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
                let (options, hooks) = stream_options(options, hook_tx)?;
                let signal = options.base.signal.clone();
                let stream = match pi_rs_ai::registry::stream_simple(
                    &model,
                    &context,
                    Some(options),
                ) {
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

                let mut hooks_open = true;
                loop {
                    tokio::select! {
                        biased;
                        request = hook_rx.recv(), if hooks_open => {
                            let Some(request) = request else {
                                hooks_open = false;
                                continue;
                            };
                            match request {
                                ProviderHookRequest::Payload { payload, model, reply } => {
                                    let value = if let Some(callback) = &hooks.payload {
                                        let payload = json_to_lua(&lua, &payload)?;
                                        let model = to_lua_json(&lua, &model)?;
                                        let value: Value = callback.call_async((payload, model)).await?;
                                        if matches!(value, Value::Nil) {
                                            None
                                        } else {
                                            Some(lua_to_json(value)?)
                                        }
                                    } else {
                                        None
                                    };
                                    let _ = reply.send(value);
                                }
                                ProviderHookRequest::Response { response, model, reply } => {
                                    if let Some(callback) = &hooks.response {
                                        let response = to_lua_json(&lua, &response)?;
                                        let model = to_lua_json(&lua, &model)?;
                                        callback.call_async::<()>((response, model)).await?;
                                    }
                                    let _ = reply.send(());
                                }
                            }
                        }
                        event = stream.next() => {
                            let Some(event) = event else { break };
                            call_event(&lua, &on_event, &event).await?;
                        }
                    }
                }
                match stream.result().await {
                    Some(message) => to_lua_json(&lua, &message),
                    None => to_lua_json(
                        &lua,
                        &failure_message(
                            &model,
                            StopReason::Error,
                            "event stream completed without a result".to_owned(),
                        ),
                    ),
                }
            },
        )?,
    )?;
    bridge.set("ai", ai)
}

fn stream_options(
    options: Option<Table>,
    hook_tx: mpsc::UnboundedSender<ProviderHookRequest>,
) -> mlua::Result<(SimpleStreamOptions, ProviderHookCallbacks)> {
    let mut result = SimpleStreamOptions::default();
    let Some(options) = options else {
        return Ok((
            result,
            ProviderHookCallbacks {
                payload: None,
                response: None,
            },
        ));
    };
    let payload = options.get::<Option<Function>>("onPayload")?;
    let response = options.get::<Option<Function>>("onResponse")?;
    if payload.is_some() {
        let tx = hook_tx.clone();
        result.base.on_payload = Some(Arc::new(move |payload, model| {
            let tx = tx.clone();
            Box::pin(async move {
                let (reply, result) = oneshot::channel();
                if tx
                    .send(ProviderHookRequest::Payload {
                        payload,
                        model,
                        reply,
                    })
                    .is_err()
                {
                    return None;
                }
                result.await.unwrap_or(None)
            })
        }));
    }
    if response.is_some() {
        result.base.on_response = Some(Arc::new(move |response, model| {
            let tx = hook_tx.clone();
            Box::pin(async move {
                let (reply, result) = oneshot::channel();
                if tx
                    .send(ProviderHookRequest::Response {
                        response,
                        model,
                        reply,
                    })
                    .is_ok()
                {
                    let _ = result.await;
                }
            })
        }));
    }
    result.base.api_key = options.get("apiKey")?;
    result.base.max_tokens = options.get("maxTokens")?;
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
    Ok((result, ProviderHookCallbacks { payload, response }))
}

fn context_from_lua(value: Value) -> mlua::Result<Context> {
    let mut json = lua_to_json(value).map_err(|error| mlua::Error::runtime(error.to_string()))?;
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
    callback.call_async::<()>(to_lua_json(lua, event)?).await
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
