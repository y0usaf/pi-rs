use std::process::{Command, ExitCode, Output};
use std::sync::Arc;

use pi_coding_agent::mount::{self, Mount, Mounted};
use pi_durable::ai::event_stream::AssistantMessageEventStream;
use pi_durable::ai::models::{Models, ModelsError};
use pi_durable::ai::providers::faux::{Faux, FauxModelDefinition, FauxOptions, FauxResponse};
use pi_durable::ai::registry::Providers;
use pi_durable::ai::types::{
    AssistantMessage, DeferredHandle, Model, SimpleStreamOptions, TranscriptContext,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::types::{Extension, ToolControl, ToolExecutionResult, ToolRegistration};
use serde_json::{Value, json};

const PROVIDER: &str = "mounted";
const MODEL: &str = "probe";
const TOOL_TEXT: &str = "mounted tool ran";
const CHILD: &str = "MOUNT_E2E_CHILD";
const STOP_TEXT: &str = "said before stopping";

struct Layer {
    inner: Providers,
    faux: Arc<Faux>,
}

impl Layer {
    fn pick(&self, provider: &str) -> &dyn Models {
        if provider == PROVIDER {
            self.faux.as_ref()
        } else {
            &self.inner
        }
    }
}

impl Models for Layer {
    fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        self.pick(provider).get_model(provider, id)
    }

    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        self.pick(&model.provider)
            .stream_simple(model, context, options)
    }

    fn fetch_deferred(
        &self,
        model: &Model,
        handle: &DeferredHandle,
        signal: &CancelToken,
    ) -> Result<AssistantMessage, ModelsError> {
        self.pick(&model.provider)
            .fetch_deferred(model, handle, signal)
    }

    fn cancel_deferred(
        &self,
        model: &Model,
        handle: &DeferredHandle,
        signal: &CancelToken,
    ) -> Result<(), ModelsError> {
        self.pick(&model.provider)
            .cancel_deferred(model, handle, signal)
    }
}

fn message(content: Value, stop_reason: &str) -> Result<AssistantMessage, String> {
    serde_json::from_value(json!({
        "role": "assistant", "content": content, "api": "faux", "provider": PROVIDER, "model": MODEL,
        "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
        "stopReason": stop_reason, "timestamp": 0
    }))
    .map_err(|e| e.to_string())
}

fn probe_tool() -> ToolRegistration {
    ToolRegistration {
        tool: serde_json::from_value(json!({
            "name": "mount_probe",
            "description": "Answers with a fixed text",
            "parameters": {"type": "object", "properties": {}}
        }))
        .expect("tool schema"),
        replay: None,
        exposure: Default::default(),
        hidden: false,
        execution_mode: None,
        output_limits: None,
        prepare_arguments: None,
        execute: Arc::new(|_arguments, _api, _cancel| {
            Ok(ToolExecutionResult {
                content: Some(vec![serde_json::from_value(
                    json!({"type": "text", "text": TOOL_TEXT}),
                )?]),
                ..Default::default()
            })
        }),
    }
}

fn stop_tool() -> ToolRegistration {
    ToolRegistration {
        tool: serde_json::from_value(json!({
            "name": "mount_stop",
            "description": "Ends the run",
            "parameters": {"type": "object", "properties": {}}
        }))
        .expect("tool schema"),
        replay: None,
        exposure: Default::default(),
        hidden: false,
        execution_mode: None,
        output_limits: None,
        prepare_arguments: None,
        execute: Arc::new(|_arguments, _api, _cancel| {
            Ok(ToolExecutionResult {
                content: Some(vec![serde_json::from_value(
                    json!({"type": "text", "text": "stopped"}),
                )?]),
                control: Some(ToolControl {
                    terminate: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
        }),
    }
}

fn child(script: &str) -> ExitCode {
    let faux = Arc::new(Faux::new(FauxOptions {
        provider: Some(PROVIDER.to_owned()),
        api: Some("faux".to_owned()),
        models: vec![FauxModelDefinition {
            id: MODEL.to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    }));
    let call = message(
        json!([{"type": "toolCall", "id": "c1", "name": "mount_probe", "arguments": {}}]),
        "toolUse",
    )
    .expect("tool call message");
    let stop = message(
        json!([
            {"type": "text", "text": STOP_TEXT},
            {"type": "toolCall", "id": "c2", "name": "mount_stop", "arguments": {}}
        ]),
        "toolUse",
    )
    .expect("stop message");
    let silent = message(
        json!([{"type": "toolCall", "id": "c3", "name": "mount_stop", "arguments": {}}]),
        "toolUse",
    )
    .expect("silent stop message");
    faux.set_responses(if script == "stop" {
        vec![FauxResponse::Message(Box::new(stop))]
    } else if script == "silent" {
        vec![FauxResponse::Message(Box::new(silent))]
    } else {
        vec![
            FauxResponse::Message(Box::new(call)),
            FauxResponse::Factory(Arc::new(|context, _, _, _| {
                let last =
                    serde_json::to_value(context.messages.last()).map_err(|e| e.to_string())?;
                let text = last["content"][0]["text"]
                    .as_str()
                    .unwrap_or("no tool result");
                let failed = last["isError"].as_bool().unwrap_or(true);
                message(
                    json!([{"type": "text", "text": format!("{failed}:{text}")}]),
                    "stop",
                )
            })),
        ]
    });
    let catalog = vec![faux.default_model()];
    let open = Box::new(move |inner: Providers| Mounted {
        models: Arc::new(Layer {
            inner,
            faux: Arc::clone(&faux),
        }),
        extensions: vec![Extension {
            name: "mount-probe".to_owned(),
            tools: vec![probe_tool(), stop_tool()],
            sections: vec![],
            wraps: vec![],
            hooks: vec![],
            tasks: vec![],
        }],
    });
    if let Err(e) = mount::set(Mount { catalog, open }) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    pi_coding_agent::cli::main()
}

fn run(bin: &str, home: &str, script: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(bin);
    command
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PI_CODING_AGENT_DIR", format!("{home}/agent"))
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .current_dir(home);
    if let Some(script) = script {
        command.env(CHILD, script);
    }
    command.output().expect("spawn")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

fn main() -> ExitCode {
    if let Ok(script) = std::env::var(CHILD) {
        return child(&script);
    }
    let me = std::env::current_exe().expect("current exe");
    let me = me.to_str().expect("utf-8 path");
    let plain = std::env::var("PI_RS_BIN").expect("PI_RS_BIN names the plain pi-rs");
    let home = std::env::temp_dir().join(format!("mount-e2e-{}", std::process::id()));
    std::fs::create_dir_all(home.join("agent")).expect("temp home");
    let home = home.to_str().expect("utf-8 path").to_owned();
    let model = format!("{PROVIDER}/{MODEL}");
    let turn = ["-p", "--no-session", "--model", model.as_str(), "probe"];
    let mut failures = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("{} {name}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            println!("  {detail}");
            failures += 1;
        }
    };

    let out = run(me, &home, Some("probe"), &turn);
    check(
        "a mounted model answers through the mounted layer, after calling the mounted extension's tool",
        out.status.success() && text(&out.stdout) == format!("false:{TOOL_TEXT}"),
        format!(
            "rc={:?} stdout={:?} stderr={:?}",
            out.status.code(),
            text(&out.stdout),
            text(&out.stderr)
        ),
    );

    let out = run(me, &home, Some("stop"), &turn);
    check(
        "a run a mounted tool ends prints the reply's text before the tool",
        out.status.success() && text(&out.stdout) == STOP_TEXT,
        format!(
            "rc={:?} stdout={:?} stderr={:?}",
            out.status.code(),
            text(&out.stdout),
            text(&out.stderr)
        ),
    );

    let saved = ["-p", "--model", model.as_str(), "probe"];
    let first = run(me, &home, Some("probe"), &saved);
    let resumed = ["-p", "-c", "--model", model.as_str(), "again"];
    let out = run(me, &home, Some("silent"), &resumed);
    check(
        "a continued session whose run a tool ends without words prints nothing, not the earlier run's reply",
        first.status.success()
            && text(&first.stdout) == format!("false:{TOOL_TEXT}")
            && out.status.success()
            && text(&out.stdout).is_empty(),
        format!(
            "first rc={:?} stdout={:?}; resumed rc={:?} stdout={:?} stderr={:?}",
            first.status.code(),
            text(&first.stdout),
            out.status.code(),
            text(&out.stdout),
            text(&out.stderr)
        ),
    );

    let out = run(me, &home, Some("probe"), &["--list-models", PROVIDER]);
    let listed = text(&out.stdout);
    check(
        "the mounted catalog is listed as available without credentials",
        out.status.success()
            && listed
                .lines()
                .any(|l| l.contains(PROVIDER) && l.contains(MODEL)),
        format!(
            "rc={:?} stdout={listed:?} stderr={:?}",
            out.status.code(),
            text(&out.stderr)
        ),
    );

    let out = run(&plain, &home, None, &turn);
    check(
        "plain pi-rs knows no mounted model",
        !out.status.success(),
        format!("rc={:?} stdout={:?}", out.status.code(), text(&out.stdout)),
    );

    let _ = std::fs::remove_dir_all(&home);
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
