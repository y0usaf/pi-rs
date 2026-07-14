//! File-backed acceptance for the minimum public coding spine.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use pi_rs_ai::registry::{ApiProvider, register_api_provider, unregister_api_providers};
use pi_rs_ai::transport::{AssistantMessageEventStream, create_assistant_message_event_stream};
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, Model, StopReason,
    TextContent, Usage, now_ms,
};
use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const FIXTURE_API: &str = "coding-spine-fixture";
const FIXTURE_OWNER: &str = "coding-spine-test";

fn message(model: &Model, text: &str) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContent::Text(TextContent::new(text))],
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_model: None,
        response_id: None,
        diagnostics: None,
        usage: Usage::default(),
        stop_reason: StopReason::Stop,
        error_message: None,
        timestamp: now_ms(),
    }
}

fn fixture_stream(model: &Model) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let partial = message(model, "fixture");
    stream.push(AssistantMessageEvent::Start {
        partial: partial.clone(),
    });
    stream.push(AssistantMessageEvent::TextDelta {
        content_index: 0,
        delta: "fixture".to_owned(),
        partial: partial.clone(),
    });
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message: partial,
    });
    stream.end();
    stream
}

struct FixtureProvider;

impl FixtureProvider {
    fn install() -> Self {
        register_api_provider(
            ApiProvider {
                api: FIXTURE_API.to_owned(),
                stream: Arc::new(|model, _, _| Ok(fixture_stream(model))),
                stream_simple: Arc::new(|model, _, _| Ok(fixture_stream(model))),
            },
            Some(FIXTURE_OWNER),
        );
        Self
    }
}

impl Drop for FixtureProvider {
    fn drop(&mut self) {
        unregister_api_providers(FIXTURE_OWNER);
    }
}

#[test]
fn file_package_joins_input_display_model_effect_cancellation_and_shutdown() {
    let _provider = FixtureProvider::install();
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("effect.txt");
    let shutdown = directory.path().join("shutdown.txt");
    let source = format!(
        r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local models = pi.models.v1
local effects = pi.effects.v1

pi.kernel.v1.resource(function()
  effects.fs.write({shutdown}, "clean")
end)

roots.register({{
  kind="application", id="coding-spine", active=true, priority=0,
  dispatch=function(snapshot)
    local immutable = not pcall(function() snapshot.event.kind = "mutated" end)

    local input = terminal.input_buffer()
    local input_events = input:feed(snapshot.event.bytes)
    local input_text = ""
    for _, event in ipairs(input_events) do input_text = input_text .. event.data end
    local display = terminal.display()
    local frame = display:submit({{
      version=terminal.display_schema_version,
      viewport={{columns=40, rows=1}}, root=1,
      nodes={{{{
        id=1, rect={{x=0, y=0, width=40, height=1}},
        content={{kind="text", runs={{{{text=input_text}}}}}},
      }}}},
    }})

    effects.fs.write(snapshot.context.path, "coding-spine")
    local contents = effects.fs.read(snapshot.context.path, 64)
    local bounded_read, bounded_read_error = pcall(effects.fs.read, snapshot.context.path, 4)
    local process = effects.process.run("sh", {{"-c", "printf process-ok"}}, {{
      timeout_ms=2000, max_output_bytes=64,
    }})
    local signal = effects.cancellation.new()
    local cancelled = effects.process.run("sh", {{"-c", "printf partial; sleep 10"}}, {{
      timeout_ms=5000, max_output_bytes=64, signal=signal,
      onData=function() signal:abort() end,
    }})

    local model = models.find("moonshotai", "kimi-k2.6")
    model.api = "{fixture_api}"
    local bounded_stream, bounded_stream_error = pcall(function()
      models.stream(model, {{messages={{}}}}, {{max_events=2}}, function() end)
    end)
    local events = {{}}
    local final = models.stream(model, {{messages={{}}}}, {{max_events=4}}, function(event)
      events[#events + 1] = event.type
    end)

    roots.action("coding_spine", {{
      immutable=immutable,
      input={{text=input_text, events=#input_events}},
      frame={{revision=frame.revision, painted_cells=frame.painted_cells}},
      model={{provider=model.provider, id=model.id}},
      events=events,
      final={{stop_reason=final.stopReason, text=final.content[1].text}},
      fs={{contents=contents, bounded=not bounded_read,
           error=tostring(bounded_read_error)}},
      process={{stdout=process.stdout, code=process.code}},
      cancelled={{killed=cancelled.killed, stdout=cancelled.stdout}},
      stream_bounded=not bounded_stream,
      stream_error=tostring(bounded_stream_error),
    }})
    roots.action("shutdown", {{reason="complete"}})
  end,
}})
"#,
        shutdown = serde_json::to_string(&shutdown.to_string_lossy()).unwrap(),
        fixture_api = FIXTURE_API,
    );
    let path = directory.path().join("application.lua");
    std::fs::write(&path, source).unwrap();

    let host = Host::new(HostConfig {
        cwd: Some(directory.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let package = host
        .load_package(PackageSource::File { path: &path })
        .unwrap();
    assert_eq!(host.scope_stats(&package).unwrap().resources, 1);
    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            serde_json::json!({"kind":"terminal_input", "bytes":"hello"}),
            serde_json::json!({"path": data}),
        ))
        .unwrap();

    assert_eq!(batch.source, path.to_string_lossy());
    assert_eq!(batch.actions.len(), 2);
    let result = &batch.actions[0].payload;
    assert_eq!(result["immutable"], true);
    assert_eq!(
        result["input"],
        serde_json::json!({"text":"hello", "events":5})
    );
    assert_eq!(
        result["frame"],
        serde_json::json!({"revision":1, "painted_cells":5})
    );
    assert_eq!(
        result["model"],
        serde_json::json!({"provider":"moonshotai", "id":"kimi-k2.6"})
    );
    assert_eq!(
        result["events"],
        serde_json::json!(["start", "text_delta", "done"])
    );
    assert_eq!(
        result["final"],
        serde_json::json!({"stop_reason":"stop", "text":"fixture"})
    );
    assert_eq!(result["fs"]["contents"], "coding-spine");
    assert_eq!(result["fs"]["bounded"], true);
    assert!(
        result["fs"]["error"]
            .as_str()
            .unwrap()
            .contains("exceeds 4 bytes")
    );
    assert_eq!(
        result["process"],
        serde_json::json!({"stdout":"process-ok", "code":0})
    );
    assert_eq!(
        result["cancelled"],
        serde_json::json!({"killed":true, "stdout":"partial"})
    );
    assert_eq!(result["stream_bounded"], true);
    assert!(
        result["stream_error"]
            .as_str()
            .unwrap()
            .contains("exceeded 2 events")
    );
    assert_eq!(batch.actions[1].kind, "shutdown");
    assert!(!shutdown.exists());

    drop(host);
    assert_eq!(std::fs::read_to_string(shutdown).unwrap(), "clean");
}
