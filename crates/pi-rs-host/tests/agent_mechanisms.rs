//! WS4.1 public binding exercisers: provider stream, cancellation/context,
//! and prepare-then-validate tool arguments.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    Host::new(HostConfig::default()).expect("host")
}

fn load_example(host: &Host, name: &str) {
    let path = format!(
        "{}/../../examples/extensions/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = std::fs::read_to_string(&path).expect("example exists");
    host.load(&format!("examples/extensions/{name}"), &source)
        .expect("example loads");
}

#[test]
fn ai_stream_binding_folds_resolution_failure_into_terminal_message() {
    let host = host();
    load_example(&host, "ai-stream-demo.lua");
    let result = host
        .call_command("ai-stream-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(result["events"], 1);
    assert_eq!(result["stopReason"], "error");
    assert_eq!(result["hasError"], true);
    assert_eq!(result["aborted"], false);
}

#[test]
fn tool_prepares_then_coerces_and_validates_arguments() {
    let host = host();
    load_example(&host, "schema-tool-demo.lua");
    let result = host
        .call_tool(
            "schema-demo",
            "call-1",
            &serde_json::json!({ "count": "41" }),
        )
        .expect("coerced call");
    assert_eq!(result["details"]["count"], 41);
    assert_eq!(result["content"][0]["text"], "42");

    let error = host
        .call_tool(
            "schema-demo",
            "call-2",
            &serde_json::json!({ "count": "nope" }),
        )
        .expect_err("invalid call");
    let message = error.to_string();
    assert!(message.contains("Validation failed for tool \"schema-demo\""));
    assert!(message.contains("count: must be integer"));
    assert!(message.contains("\"count\": \"nope\""));
}

#[test]
fn command_and_tool_receive_context_and_signal() {
    let host = host();
    host.load(
        "test://contexts",
        r#"
        local pi = ...
        pi.register_command("ctx", { handler = function(_args, ctx)
          return { cwd = ctx.cwd, idle = ctx.isIdle,
                   aborted = ctx.signal:is_aborted() }
        end })
        pi.register_tool({ name = "ctx-tool", parameters = { type = "object" },
          execute = function(_id, _params, signal, _update, ctx)
            return { sameState = signal:is_aborted() == ctx.signal:is_aborted(), cwd = ctx.cwd }
          end })
    "#,
    )
    .expect("load");
    let command = host
        .call_command("ctx", "")
        .expect("command")
        .expect("result");
    assert_eq!(command["idle"], false);
    assert_eq!(command["aborted"], false);
    assert!(command["cwd"].as_str().is_some_and(|cwd| !cwd.is_empty()));
    let tool = host
        .call_tool("ctx-tool", "call", &serde_json::json!({}))
        .expect("tool");
    assert_eq!(tool["sameState"], true);
    assert!(tool["cwd"].as_str().is_some_and(|cwd| !cwd.is_empty()));
}
