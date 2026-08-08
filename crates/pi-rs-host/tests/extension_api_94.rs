//! PLAN 9.4 host accept: the non-UI ExtensionAPI members are callable from
//! file-backed Lua and delegate session/message/model/tool policy to the
//! product-installed runtime bridge. Renderer registrations live
//! per-extension with first-registration-wins snapshots; bare-host reads
//! degrade to declaration defaults.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 5000,
        ..HostConfig::default()
    })
    .expect("host starts")
}

const BRIDGE: &str = r#"
local pi = ...
-- product-shaped bridge: the session/message/tool state the members mutate
local state = {
  session_name = nil, thinking_level = "medium",
  active_tools = {}, model = nil,
  entries = {}, custom_messages = {}, user_messages = {}, labels = {},
}
pi.install_runtime_bridge({
  send_message = function(message, options)
    state.custom_messages[#state.custom_messages + 1] = { message = message, options = options }
  end,
  send_user_message = function(content, options)
    state.user_messages[#state.user_messages + 1] = { content = content, options = options }
  end,
  append_entry = function(custom_type, data)
    state.entries[#state.entries + 1] = { customType = custom_type, data = data }
  end,
  set_session_name = function(name) state.session_name = name end,
  get_session_name = function() return state.session_name end,
  set_label = function(entry_id, label)
    state.labels[#state.labels + 1] = { entryId = entry_id, label = label }
  end,
  get_active_tools = function()
    local out = {}
    for i, name in ipairs(state.active_tools) do out[i] = name end
    return out
  end,
  set_active_tools = function(tool_names)
    state.active_tools = {}
    for i, name in ipairs(tool_names) do state.active_tools[i] = name end
  end,
  set_model = function(model)
    state.model = model
    return model and model.available ~= false
  end,
  get_thinking_level = function() return state.thinking_level end,
  set_thinking_level = function(level) state.thinking_level = level end,
})
pi.register_command("bridge-inspect", {
  description = "return the bridge-visible state",
  handler = function()
    return {
      session_name = state.session_name, thinking_level = state.thinking_level,
      active_tools = state.active_tools, model = state.model,
      entries = state.entries, custom_messages = state.custom_messages,
      user_messages = state.user_messages, labels = state.labels,
    }
  end,
})
"#;

const PROBE: &str = r#"
local pi = ...
pi.register_tool({
  name = "base-tool", label = "Base", description = "base",
  parameters = { type = "object", properties = {} },
  active_by_default = true,
  execute = function() return { content = {} } end,
})
pi.register_tool({
  name = "opt-tool", label = "Opt", description = "opt",
  parameters = { type = "object", properties = {} },
  active_by_default = false,
  execute = function() return { content = {} } end,
})
local rendered
pi.register_message_renderer("status-update", function(message, options, theme)
  rendered = message.content .. "|" .. tostring(options.expanded)
  return { rendered = rendered }
end)
pi.register_command("probe", {
  description = "exercise every 9.4 member",
  handler = function()
    local results = {}
    pi.send_message({ customType = "status-update", content = "hi", display = true,
      details = { level = "info" } }, { triggerTurn = false })
    pi.send_user_message("hello agent", { deliverAs = "steer" })
    pi.append_entry("note", { text = "remember me" })
    pi.set_session_name("my session")
    results.session_name = pi.get_session_name()
    pi.set_label("entry-1", "done")
    pi.set_thinking_level("high")
    results.thinking_level = pi.get_thinking_level()
    results.model_ok = pi.set_model({ provider = "p", id = "m", available = true })
    results.model_denied = pi.set_model({ provider = "p", id = "m", available = false })
    results.active_before = pi.get_active_tools()
    pi.set_active_tools({ "base-tool" })
    results.active_after = pi.get_active_tools()
    local all = pi.get_all_tools()
    local names, info = {}, {}
    for i, tool in ipairs(all) do
      names[i] = tool.name
      info[tool.name] = { description = tool.description, source = tool.sourceInfo.source,
        scope = tool.sourceInfo.scope, origin = tool.sourceInfo.origin,
        path = tool.sourceInfo.path, guidelines = tool.promptGuidelines }
    end
    results.all_names = names
    results.info = info
    results.renderers = pi.registered_message_renderers()
    return results
  end,
})
pi.register_command("render-probe", {
  description = "call the registered renderer snapshot",
  handler = function()
    local renderers = pi.registered_message_renderers()
    local entry = renderers[1]
    local outcome = entry.render({ customType = "status-update", content = "x",
      display = true, details = {} }, { expanded = true }, {})
    return { customType = entry.customType, source = entry.source, result = outcome }
  end,
})
"#;

#[test]
fn nine_four_members_delegate_to_runtime_bridge() {
    let host = host();
    host.load("test://bridge", BRIDGE).expect("bridge loads");
    host.load("test://probe", PROBE).expect("probe loads");

    let result = host
        .call_command("probe", "")
        .expect("probe runs")
        .expect("probe returns a value");

    assert_eq!(result["session_name"], "my session");
    assert_eq!(result["thinking_level"], "high");
    assert_eq!(result["model_ok"], true);
    assert_eq!(result["model_denied"], false);
    // Active tools: declared before, filtered by set_active_tools after.
    assert_eq!(result["active_before"], serde_json::json!({}));
    assert_eq!(result["active_after"], serde_json::json!(["base-tool"]));
    // getAllTools rows: every registered tool with source metadata.
    let names: Vec<&str> = result["all_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["base-tool", "opt-tool"]);
    assert_eq!(result["info"]["base-tool"]["description"], "base");
    assert_eq!(result["info"]["base-tool"]["source"], "local");
    assert_eq!(result["info"]["base-tool"]["scope"], "temporary");
    assert_eq!(result["info"]["base-tool"]["origin"], "top-level");
    assert_eq!(result["info"]["base-tool"]["path"], "test://probe");

    // Renderer snapshot: one row, first registration wins.
    let renderers = result["renderers"].as_array().unwrap();
    assert_eq!(renderers.len(), 1);
    assert_eq!(renderers[0]["customType"], "status-update");
    assert_eq!(renderers[0]["source"], "test://probe");

    // The mutations landed on the bridge side.
    let state = host
        .call_command("bridge-inspect", "")
        .expect("inspect runs")
        .expect("inspect returns a value");
    assert_eq!(state["session_name"], "my session");
    assert_eq!(state["thinking_level"], "high");
    assert_eq!(state["model"]["id"], "m");
    assert_eq!(state["active_tools"], serde_json::json!(["base-tool"]));
    assert_eq!(state["entries"][0]["customType"], "note");
    assert_eq!(state["entries"][0]["data"]["text"], "remember me");
    assert_eq!(state["custom_messages"][0]["message"]["customType"], "status-update");
    assert_eq!(state["user_messages"][0]["content"], "hello agent");
    assert_eq!(state["labels"][0]["entryId"], "entry-1");
    assert_eq!(state["labels"][0]["label"], "done");
}

#[test]
fn message_renderer_snapshot_invokes_with_source_attribution() {
    let host = host();
    host.load("test://bridge", BRIDGE).expect("bridge loads");
    host.load("test://probe", PROBE).expect("probe loads");

    let result = host
        .call_command("render-probe", "")
        .expect("render runs")
        .expect("render returns a value");
    assert_eq!(result["customType"], "status-update");
    assert_eq!(result["source"], "test://probe");
    assert_eq!(result["result"]["rendered"], "x|true");
}

#[test]
fn nine_four_reads_degrade_without_bridge() {
    let host = host();
    host.load("test://probe", PROBE).expect("probe loads");

    let result = host
        .call_command("probe", "")
        .expect("probe runs")
        .expect("probe returns a value");
    // No bridge: mutations are inert, reads fall back to defaults.
    assert_eq!(result["session_name"], serde_json::Value::Null);
    assert_eq!(result["thinking_level"], "medium");
    assert_eq!(result["model_ok"], false);
    assert_eq!(result["active_before"], serde_json::json!(["base-tool"]));
    assert_eq!(result["active_after"], serde_json::json!(["base-tool"]));
}


const PRODUCT_BRIDGE: &str = r#"
local pi = ...
local session = pi.session.in_memory({ cwd = "/" })
local agent_state = {
  tools = {}, thinkingLevel = "medium", model = nil,
  isStreaming = false, messages = {},
}
pi.install_runtime_bridge({
  append_entry = function(custom_type, data)
    session:append_custom_entry(custom_type, data)
  end,
  set_session_name = function(name) session:append_session_info(name) end,
  get_session_name = function() return session:get_session_name() end,
  set_label = function(entry_id, label) session:append_label_change(entry_id, label) end,
  get_active_tools = function()
    local out = {}
    for i, t in ipairs(agent_state.tools) do out[i] = t.name end
    return out
  end,
  set_active_tools = function(names)
    local known = {}
    for _, def in ipairs(pi.registered_tools()) do known[def.name] = def end
    agent_state.tools = {}
    for _, name in ipairs(names) do
      if known[name] then agent_state.tools[#agent_state.tools + 1] = { name = name } end
    end
  end,
  set_model = function(model)
    if model.available == false then return false end
    agent_state.model = model
    return true
  end,
  get_thinking_level = function() return agent_state.thinkingLevel end,
  set_thinking_level = function(level) agent_state.thinkingLevel = level end,
  send_message = function(message, options)
    agent_state.messages[#agent_state.messages + 1] = message
  end,
  send_user_message = function(content, options)
    agent_state.messages[#agent_state.messages + 1] = { role = "user", content = content }
  end,
})
pi.register_command("product-bridge-inspect", { handler = function()
  return { session_name = session:get_session_name(), thinking = agent_state.thinkingLevel,
    model = agent_state.model, messages = agent_state.messages, tools = agent_state.tools }
end })
"#;

const BASE_TOOLS: &str = r#"
local pi = ...
pi.register_tool({ name = "read", label = "Read", description = "Read a file",
  parameters = { type = "object", properties = { path = { type = "string" } } },
  execute = function() return { content = {} } end })
pi.register_tool({ name = "grep", label = "Grep", description = "Search files",
  parameters = { type = "object", properties = {} },
  execute = function() return { content = {} } end })
"#;

fn exerciser_path(name: &str) -> String {
    format!("{}/../../examples/extensions/{}.lua", env!("CARGO_MANIFEST_DIR"), name)
}

fn host_at(cwd: &str) -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 5000,
        cwd: Some(cwd.to_owned()),
        ..HostConfig::default()
    })
    .expect("host starts")
}

fn load_exerciser(host: &Host, name: &str) {
    let source = std::fs::read_to_string(exerciser_path(name))
        .unwrap_or_else(|e| panic!("read {name}: {e}"));
    host.load(&format!("examples/extensions/{name}.lua"), &source)
        .unwrap_or_else(|e| panic!("load {name}: {e}"));
}

#[test]
fn dynamic_tools_exerciser_registers_and_reports() {
    let host = host();
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    load_exerciser(&host, "dynamic-tools");

    // The session_start handler registers echo_session at runtime.
    let outcomes = host
        .emit("session_start", &serde_json::json!({ "reason": "startup" }))
        .expect("emit");
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].result.is_ok(), "{:?}", outcomes[0].result);

    let tools = host.tools().expect("mirror");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["echo_session"]);

    let result = host
        .call_command("add-echo-tool", "hello_tool")
        .expect("cmd")
        .expect("value");
    assert_eq!(result["created"], true);
    let again = host
        .call_command("add-echo-tool", "hello_tool")
        .expect("cmd")
        .expect("value");
    assert_eq!(again["created"], false, "re-registration is rejected");

    let echo = host
        .call_tool("hello_tool", "call-1", &serde_json::json!({ "message": "hi" }))
        .expect("tool");
    assert_eq!(echo["content"][0]["text"], "[hello_tool] hi");

    let inspect = host
        .call_command("dynamic-tools-inspect", "")
        .expect("cmd")
        .expect("value");
    assert_eq!(
        inspect["names"],
        serde_json::json!(["echo_session", "hello_tool"])
    );
    assert_eq!(inspect["active"], serde_json::json!({}));
}

#[test]
fn tool_override_exerciser_overrides_and_validates() {
    let host = host_at("/");
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    load_exerciser(&host, "tool-override");

    let probe = host
        .call_command("tool-override-probe", "")
        .expect("cmd")
        .expect("value");
    assert_eq!(probe["label"], "read (audited)");
    assert_eq!(probe["source"], "local");
    assert_eq!(probe["hasParameters"], true);

    // Blocked path: the override's own policy returns denied.
    let blocked = host
        .call_tool("read", "call-1", &serde_json::json!({ "path": "/app/.env" }))
        .expect("tool");
    assert_eq!(blocked["details"]["blocked"], true);
    assert!(blocked["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("blocked pattern"));

    // Allowed path: prepare_arguments normalizes, execute reports.
    let allowed = host
        .call_tool("read", "call-2", &serde_json::json!({ "path": "README.md", "offset": 2, "limit": 5 }))
        .expect("tool");
    assert_eq!(allowed["details"]["blocked"], false);
    assert_eq!(allowed["details"]["cwd"], "/");
    assert_eq!(allowed["content"][0]["text"], "read ./README.md (offset=2, limit=5)");

    // Schema validation still applies to the override.
    let invalid = host.call_tool("read", "call-3", &serde_json::json!({ "path": 42 }));
    assert!(invalid.is_err(), "non-string path must fail validation");
}

#[test]
fn message_renderer_exerciser_registers_and_sends() {
    let host = host();
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    load_exerciser(&host, "message-renderer");

    let probe = host
        .call_command("message-renderer-probe", "")
        .expect("cmd")
        .expect("value");
    assert_eq!(probe["renderers"][0]["customType"], "status-update");
    assert_eq!(probe["invoked"]["rendered"], "[INFO] hello");
    assert_eq!(probe["invoked"]["level"], "info");

    let sent = host.call_command("status", "warn oh no").expect("cmd").expect("value");
    assert_eq!(sent["sent"]["level"], "warn");
    assert_eq!(sent["sent"]["content"], "oh no");
    let state = host
        .call_command("product-bridge-inspect", "")
        .expect("cmd")
        .expect("value");
    assert_eq!(state["messages"][0]["customType"], "status-update");
    assert_eq!(state["messages"][0]["content"], "oh no");
    assert_eq!(state["messages"][0]["details"]["level"], "warn");
}

#[test]
fn session_name_exerciser_sets_and_reads() {
    let host = host();
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    load_exerciser(&host, "session-name");

    let set = host.call_command("session-name", "my session").expect("cmd").expect("value");
    assert_eq!(set["set"], "my session");
    assert_eq!(set["now"], "my session");

    let get = host.call_command("session-name", "").expect("cmd").expect("value");
    assert_eq!(get["current"], "my session");
}

#[test]
fn send_user_message_exerciser_sends() {
    let host = host();
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    load_exerciser(&host, "send-user-message");

    // The bare host reports not-idle while a command dispatch is active,
    // so the pinned ask path (plain send only when idle) refuses; the
    // busy guard is pinned below. steer always delivers and exercises
    // the same member with a delivery option.
    let busy = host.call_command("ask", "busy probe").expect("cmd").expect("value");
    assert_eq!(busy["error"], "busy");
    let steer = host.call_command("steer", "what is 2+2?").expect("cmd").expect("value");
    assert_eq!(steer["sent"], "what is 2+2?");
    assert_eq!(steer["deliverAs"], "steer");
    let state = host
        .call_command("product-bridge-inspect", "")
        .expect("cmd")
        .expect("value");
    assert_eq!(state["messages"][0]["role"], "user");
    assert_eq!(state["messages"][0]["content"], "what is 2+2?");
}

#[test]
fn preset_exerciser_applies_model_thinking_tools() {
    let host = host();
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    host.load("test://base", BASE_TOOLS).expect("base");
    load_exerciser(&host, "preset");

    // fast preset: thinking low + tools ["read"] (grep unknown).
    let report = host.call_command("preset-apply", "fast").expect("cmd").expect("value");
    assert_eq!(report["thinking"], "low");
    assert_eq!(report["active"], serde_json::json!(["read"]));
    assert_eq!(report["unknown"], serde_json::json!({}));

    // plan preset: model + thinking high + tools [read, grep].
    let report = host.call_command("preset-apply", "plan").expect("cmd").expect("value");
    assert_eq!(report["model_error"], "not-found");
    assert_eq!(report["thinking"], "high");
    assert_eq!(report["active"], serde_json::json!(["read", "grep"]));

    let unknown = host.call_command("preset-apply", "nope").expect("cmd").expect("value");
    assert_eq!(unknown["error"], "unknown-preset");
}

#[test]
fn stateful_tools_exerciser_persists_and_reports() {
    let host = host();
    host.load("test://bridge", PRODUCT_BRIDGE).expect("bridge");
    host.load("test://base", BASE_TOOLS).expect("base");
    load_exerciser(&host, "stateful-tools");

    let enabled = host
        .call_command("tools-enable", "read grep")
        .expect("cmd")
        .expect("value");
    assert_eq!(enabled["enabled"], serde_json::json!(["read", "grep"]));
    assert_eq!(enabled["active"], serde_json::json!(["read", "grep"]));

    // append_entry persisted a tools-config custom entry to the session.
    let state = host
        .call_command("tools-state", "")
        .expect("cmd")
        .expect("value");
    assert_eq!(state["active"], serde_json::json!(["read", "grep"]));
    assert_eq!(state["saved"]["enabledTools"], serde_json::json!(["read", "grep"]));
}
