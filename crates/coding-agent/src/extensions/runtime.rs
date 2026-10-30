use std::sync::Arc;

use pi_durable::ai::event_stream::AssistantMessageEventStream;
use pi_durable::ai::registry::{Api, Origin};
use pi_durable::ai::stream::{fail, new_output};
use pi_durable::ai::thinking::{clamp_thinking_level, supported_thinking_levels};
use pi_durable::ai::types::{
    AssistantMessage, AssistantMessageEvent, Message, Model, ModelThinkingLevel, Payload,
    ProviderResponse, RequestHooks, SimpleStreamOptions, StopReason, TranscriptContext,
};
use pi_durable::cancel::CancelToken;
use pi_durable::chord::json::JsonObject;
use pi_durable::harness::generation::Route;
use pi_durable::harness::types::ModelRef;
use pi_durable::tasks::{TaskOutcome, TaskState};
use pi_durable::types::{EntryDraft, EntryQuery, ScanOrder};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Live, live, lock};
use crate::models::is_virtual;
use crate::run_config::{level_name, parse_level};

const CUSTOM_KIND: &str = "pi.custom";
const STATE_ENTRY: &str = "pi.virtual-model-state";
const SCAN_PAGE: usize = 256;

pub fn request_hooks() -> RequestHooks {
    RequestHooks {
        payload: Some(Arc::new(|payload: Payload, _model: &Model| match live() {
            Some(live) => live.provider_payload(payload),
            None => Ok(payload.wire()),
        })),
        response: Some(Arc::new(|response: &ProviderResponse, _model: &Model| {
            if let Some(live) = live() {
                live.provider_response(response);
            }
            Ok(())
        })),
        stream_event: Some(Arc::new(|data: &Value, model: &Model| {
            if let Some(live) = live() {
                live.provider_stream_event(data, model);
            }
        })),
    }
}

#[derive(Deserialize)]
struct Replaced {
    payload: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Routing {
    provider: String,
    id: String,
    thinking_level: ModelThinkingLevel,
    state: Option<RouterState>,
}

#[derive(Deserialize)]
struct RouterState {
    value: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomEntry {
    custom_type: String,
    data: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateData {
    provider: String,
    model_id: String,
    state: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompactArgs {
    custom_instructions: Option<String>,
}

pub struct Virtual {
    pub origin: Origin,
}

impl Api for Virtual {
    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        let out = AssistantMessageEventStream::new();
        let Some(live) = live() else {
            fail(
                &out,
                new_output(model),
                None,
                "no extension host is running".to_owned(),
            );
            return out;
        };
        let signal = options.signal.clone();
        if let Some(signal) = &signal {
            out.bind_cancel_with(signal, new_output(model));
        }
        let (live, model, sink, origin) = (live.clone(), model.clone(), out.clone(), self.origin);
        std::thread::spawn(move || {
            if let Err(message) = live.route(&model, context, options, origin, &sink) {
                fail(&sink, new_output(&model), signal.as_ref(), message);
            }
        });
        out
    }
}

fn user_turn(messages: &[Message]) -> bool {
    let after = messages
        .iter()
        .rposition(|message| matches!(message, Message::Assistant(_)))
        .map_or(0, |last| last + 1);
    messages[after..]
        .iter()
        .any(|message| matches!(message, Message::User(_)))
}

impl Live {
    fn provider_payload(&self, payload: Payload) -> Result<JsonObject, String> {
        if !self.handles("before_provider_request") {
            return Ok(payload.wire());
        }
        let event = json!({ "payload": payload.body, "unset": payload.unset });
        let Ok(answer) = self.ask("before_provider_request", event) else {
            return Ok(payload.wire());
        };
        let replaced: Replaced = serde_json::from_value(answer)
            .map_err(|e| format!("the extension host answered before_provider_request with {e}"))?;
        match replaced.payload {
            Value::Object(body) => Ok(body),
            other => Err(format!(
                "a before_provider_request handler returned {other}, which is not a request body"
            )),
        }
    }

    fn provider_response(&self, response: &ProviderResponse) {
        if self.handles("after_provider_response") {
            let event = json!({ "status": response.status, "headers": response.headers });
            let _ = self.ask("after_provider_response", event);
        }
    }

    fn provider_stream_event(&self, data: &Value, model: &Model) {
        if self.handles("provider_stream_event") {
            let event = json!({
                "data": data,
                "provider": model.provider,
                "api": model.api,
                "model": model.id,
            });
            let _ = self.ask("provider_stream_event", event);
        }
    }

    pub(super) fn routed(
        &self,
        model: &Model,
        messages: &[Message],
        selected: ModelThinkingLevel,
        signal: &CancelToken,
    ) -> Result<Option<Route>, String> {
        if !is_virtual(model) {
            return Ok(None);
        }
        let (target, thinking_level) =
            self.decide(model, messages, selected, Origin::AgentLoop, Some(signal))?;
        Ok(Some(Route {
            model: ModelRef {
                provider: target.provider,
                model_id: target.id,
            },
            thinking_level,
        }))
    }

    pub(super) fn responded(&self, message: &AssistantMessage) {
        *lock(&self.0.sink.failed) =
            (message.stop_reason == StopReason::Error).then(|| message.clone());
    }

    fn route(
        &self,
        model: &Model,
        context: TranscriptContext,
        mut options: SimpleStreamOptions,
        origin: Origin,
        out: &AssistantMessageEventStream,
    ) -> Result<(), String> {
        let models = lock(&self.0.sink.models)
            .clone()
            .ok_or("pi-rs has no model catalog to route a virtual model with yet")?;
        let selected = options
            .reasoning
            .map_or(ModelThinkingLevel::Off, ModelThinkingLevel::from);
        let (target, thinking_level) = self.decide(
            model,
            &context.messages,
            selected,
            origin,
            options.signal.as_ref(),
        )?;
        options.reasoning = pi_durable::harness::generation::reasoning(thinking_level);
        if origin != Origin::AgentLoop
            && let Some(limit) = options.max_tokens
            && target.max_tokens > 0
        {
            options.max_tokens = Some(limit.min(target.max_tokens));
        }
        if target.provider != model.provider {
            options.api_key = None;
            options.headers = None;
            options.base_url = None;
        }
        let stream = models.providers.stream(&target, context, options, origin);
        while let Some(event) = stream.next_event() {
            let last = matches!(
                event,
                AssistantMessageEvent::Error { .. } | AssistantMessageEvent::Done { .. }
            );
            out.push(event);
            if last {
                out.end(None);
            }
        }
        Ok(())
    }

    fn decide(
        &self,
        model: &Model,
        messages: &[Message],
        selected: ModelThinkingLevel,
        origin: Origin,
        signal: Option<&CancelToken>,
    ) -> Result<(Model, ModelThinkingLevel), String> {
        let models = lock(&self.0.sink.models)
            .clone()
            .ok_or("pi-rs has no model catalog to route a virtual model with yet")?;
        let agent = origin == Origin::AgentLoop;
        let failed = agent.then(|| lock(&self.0.sink.failed).take()).flatten();
        let reason = match (agent, &failed) {
            (false, _) => "direct",
            (true, Some(_)) => "retry",
            (true, None) if user_turn(messages) => "user",
            (true, None) => "continuation",
        };
        let messages = serde_json::to_value(messages).map_err(|e| e.to_string())?;
        let mut request = json!({
            "provider": model.provider,
            "id": model.id,
            "reason": reason,
            "thinkingLevel": selected,
            "messages": messages,
        });
        if let Some(failed) = &failed {
            request["failed"] = serde_json::to_value(failed).map_err(|e| e.to_string())?;
        }
        if agent && let Some(state) = self.router_state(model)? {
            request["state"] = state;
        }
        let answer = self
            .begin(|host, ctx| host.route(request, ctx))
            .and_then(|pending| pending.wait(signal, &mut |_| {}))
            .inspect_err(|_| self.died())?;
        let route: Routing = serde_json::from_value(answer)
            .map_err(|e| format!("the extension host answered a route with {e}"))?;
        let routed = format!(
            "Virtual model {}/{} routed to {}/{}",
            model.provider, model.id, route.provider, route.id
        );
        let target = models
            .registry
            .get_model(&route.provider, &route.id)
            .filter(|target| !is_virtual(target))
            .ok_or_else(|| format!("{routed}, which is not a physical model."))?;
        if !models.registry.has_configured_auth(&target.provider) {
            return Err(format!("{routed}, which has no credentials."));
        }
        if let (true, Some(state)) = (agent, route.state) {
            self.store_router_state(model, state.value)?;
        }
        let thinking_level = clamp_thinking_level(&target, route.thinking_level);
        Ok((target, thinking_level))
    }

    fn router_state(&self, model: &Model) -> Result<Option<Value>, String> {
        let Some(source) = lock(&self.0.session).clone() else {
            return Ok(None);
        };
        let cancel = CancelToken::new();
        let mut cursor = None;
        loop {
            let query = EntryQuery {
                conversation_id: source.root.id(),
                min_entry_id: None,
                max_entry_id: None,
                order: Some(ScanOrder::Descending),
            };
            let page = source
                .root
                .entries(query, SCAN_PAGE, cursor, &cancel)
                .map_err(|e| e.to_string())?;
            for record in page
                .items
                .into_iter()
                .filter(|record| record.kind == CUSTOM_KIND)
            {
                let Some(entry) = record
                    .data
                    .and_then(|data| serde_json::from_value::<CustomEntry>(data).ok())
                    .filter(|entry| entry.custom_type == STATE_ENTRY)
                else {
                    continue;
                };
                let Ok(data) = serde_json::from_value::<StateData>(entry.data) else {
                    continue;
                };
                if data.provider == model.provider && data.model_id == model.id {
                    return Ok(Some(data.state));
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(None),
            }
        }
    }

    fn store_router_state(&self, model: &Model, state: Value) -> Result<(), String> {
        let source = lock(&self.0.session)
            .clone()
            .ok_or("no session is open to keep the router's state in")?;
        let draft = EntryDraft {
            kind: CUSTOM_KIND.to_owned(),
            model: None,
            data: Some(json!({
                "customType": STATE_ENTRY,
                "data": { "provider": model.provider, "modelId": model.id, "state": state },
            })),
            head: None,
            edits: None,
        };
        let id = source.root.id();
        source
            .root
            .commit(
                |tx| tx.append_entry(id, draft).map(|_| ()),
                &CancelToken::new(),
            )
            .map_err(|e| e.to_string())
    }

    pub(super) fn append_custom(
        &self,
        args: &Value,
        cancel: &CancelToken,
    ) -> Result<Value, String> {
        let root = self
            .root()
            .ok_or("no session is open to append an entry to")?;
        let draft = json!({
            "type": "custom",
            "customType": args.get("customType"),
            "data": args.get("data"),
        });
        crate::sessions::drafts::append(&root, &[draft], cancel)?;
        let session = self
            .session_update(cancel)
            .ok_or("the session could not be read after the entry was appended")?;
        Ok(json!({ "session": session }))
    }

    pub(super) fn thinking_level(&self, cancel: &CancelToken) -> Result<Value, String> {
        let root = self
            .root()
            .ok_or("no session is open to read the thinking level of")?;
        let agent = root.agent(cancel).map_err(|e| e.to_string())?;
        Ok(json!({ "level": level_name(agent.thinking_level) }))
    }

    pub(super) fn set_thinking_level(
        &self,
        args: &Value,
        cancel: &CancelToken,
    ) -> Result<Value, String> {
        let root = self
            .root()
            .ok_or("no session is open to set the thinking level of")?;
        let agent = root.agent(cancel).map_err(|e| e.to_string())?;
        let model = agent.model.as_ref().and_then(|model| {
            self.0
                .sink
                .registry
                .get()?
                .get_model(&model.provider, &model.model_id)
        });
        let requested = args
            .get("level")
            .and_then(Value::as_str)
            .and_then(parse_level);
        let level = match (model, requested) {
            (Some(model), Some(level)) => clamp_thinking_level(&model, level),
            (Some(model), None) => supported_thinking_levels(&model)
                .first()
                .copied()
                .unwrap_or(ModelThinkingLevel::Off),
            (None, level) => level.unwrap_or(ModelThinkingLevel::Off),
        };
        let previous = agent.thinking_level;
        let mut answer =
            json!({ "level": level_name(level), "previousLevel": level_name(previous) });
        if level != previous {
            crate::run_config::set_thinking(&root, cancel, level)?;
            *lock(&self.0.thinking_change) = Some(level);
            self.print_event(
                json!({ "type": "thinking_level_changed", "level": level_name(level) }),
            );
            let session = self
                .session_update(cancel)
                .ok_or("the session could not be read after its thinking level changed")?;
            answer["updates"] = json!({ "session": session });
        }
        Ok(answer)
    }

    pub(super) fn set_label(&self, args: &Value, cancel: &CancelToken) -> Result<Value, String> {
        let root = self
            .root()
            .ok_or("no session is open to label an entry in")?;
        let target = match args.get("entryId") {
            Some(Value::String(id)) => id.clone(),
            Some(other) => other.to_string(),
            None => "undefined".to_owned(),
        };
        let held = self.session_update(cancel).is_some_and(|session| {
            session["entries"]
                .as_array()
                .is_some_and(|entries| entries.iter().any(|entry| entry["id"] == target.as_str()))
        });
        if !held {
            return Err(format!("Entry {target} not found"));
        }
        let draft = json!({ "type": "label", "targetId": target, "label": args.get("label") });
        crate::sessions::drafts::append(&root, &[draft], cancel)?;
        let session = self
            .session_update(cancel)
            .ok_or("the session could not be read after the label was set")?;
        Ok(json!({ "session": session }))
    }

    pub(super) fn name_session(&self, args: &Value, cancel: &CancelToken) -> Result<Value, String> {
        let root = self.root().ok_or("no session is open to name")?;
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or("pi.setSessionName takes a string")?;
        crate::session_info::set_name(&root, cancel, name).map_err(|e| e.to_string())?;
        let stored = crate::session_info::sanitize(name);
        if let Some(source) = lock(&self.0.session).as_mut() {
            source.name = (!stored.is_empty()).then_some(stored);
        }
        let session = self
            .session_update(cancel)
            .ok_or("the session could not be read after it was named")?;
        Ok(json!({ "session": session }))
    }

    pub(super) fn compact(&self, args: &Value, cancel: &CancelToken) -> Result<Value, String> {
        let args: CompactArgs = serde_json::from_value(args.clone())
            .map_err(|e| format!("ctx.compact was asked with {e}"))?;
        let source = lock(&self.0.session)
            .clone()
            .ok_or("no session is open to compact")?;
        let root = &source.root;
        root.abort(false, cancel).map_err(|e| e.to_string())?;
        let task = root
            .compact(args.custom_instructions.as_deref(), cancel)
            .map_err(|e| e.to_string())?;
        root.wait_for_idle(cancel).map_err(|e| e.to_string())?;
        let record = root
            .commit(|tx| tx.task(task), cancel)
            .map_err(|e| e.to_string())?
            .ok_or("the compaction task is gone")?;
        match record.state {
            TaskState::Terminal {
                outcome: TaskOutcome::Completed { .. },
            } => {}
            TaskState::Terminal {
                outcome: TaskOutcome::Failed { error, .. },
            } => return Err(error.message),
            _ => return Err("Compaction cancelled".to_owned()),
        }
        let session = self
            .session_update(cancel)
            .ok_or("the compacted session could not be read")?;
        Ok(json!({ "session": session }))
    }
}
