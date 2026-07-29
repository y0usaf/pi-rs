//! File-backed v1 input and retained-display capability.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const PACKAGE: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

local function frame(text, cursor_visible)
  return {
    version = terminal.display_schema_version,
    viewport = { columns = 12, rows = 3 },
    root = 1,
    nodes = {
      {
        id = 1, rect = { x = 0, y = 0, width = 12, height = 3 },
        clip_children = true, content = { kind = "group" }, children = { 2 },
      },
      {
        id = 2, rect = { x = 1, y = 1, width = 10, height = 1 },
        clip_children = true, focusable = true,
        content = {
          kind = "text", wrap = "clip",
          runs = { { text = text, style = { bold = true } } },
        },
      },
    },
    focused = 2,
    cursor = {
      node = 2, row = 0, column = 3, shape = "bar", visible = cursor_visible,
    },
  }
end

roots.register({
  kind="application", id="terminal-probe", active=true, priority=0,
  dispatch=function()
    local input = terminal.input_buffer()
    local first = input:feed("a\27[")
    local pending = input:buffer()
    local second = input:feed("A\27[200~hello")
    local paste = input:feed(" world\27[201~")
    input:feed("\27[")
    input:clear()
    local cleared = input:buffer()
    input:feed("\27[")
    local flushed = input:flush()

    local display = terminal.display()
    local initial = display:submit(frame("A界", true))
    local unchanged = display:submit(frame("A界", true))
    local changed = display:submit(frame("A好", true))
    local revision_before_error = display:revision()
    local malformed_ok, malformed_error = pcall(function()
      display:submit(frame("bad\27[2J", true))
    end)
    local revision_after_error = display:revision()
    display:reset_presentation()
    local redrawn = display:submit(frame("A好", false))

    roots.action("terminal_probe", {
      input={
        first=first, pending=pending, second=second, paste=paste,
        cleared=cleared, flushed=flushed,
      },
      display={
        schema_version=terminal.display_schema_version,
        initial=initial, unchanged=unchanged, changed=changed,
        malformed_ok=malformed_ok, malformed_error=tostring(malformed_error),
        revision_before_error=revision_before_error,
        revision_after_error=revision_after_error, redrawn=redrawn,
      },
    })
  end,
})
"#;

#[test]
fn input_is_batched_and_retained_display_is_transactional() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("terminal-probe.lua");
    std::fs::write(&path, PACKAGE).unwrap();
    let host = Host::new(HostConfig::default()).unwrap();
    host.load_package(PackageSource::File { path: &path })
        .unwrap();
    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            serde_json::json!({"kind":"probe"}),
            serde_json::json!({}),
        ))
        .unwrap();
    let result = &batch.actions[0].payload;

    assert_eq!(
        result["input"]["first"],
        serde_json::json!([{ "kind": "data", "data": "a" }])
    );
    assert_eq!(result["input"]["pending"], "\u{1b}[");
    assert_eq!(
        result["input"]["second"],
        serde_json::json!([{ "kind": "data", "data": "\u{1b}[A" }])
    );
    assert_eq!(
        result["input"]["paste"],
        serde_json::json!([{ "kind": "paste", "data": "hello world" }])
    );
    assert_eq!(result["input"]["cleared"], "");
    assert_eq!(
        result["input"]["flushed"],
        serde_json::json!([{ "kind": "data", "data": "\u{1b}[" }])
    );

    let display = &result["display"];
    assert_eq!(display["schema_version"], 2);
    assert_eq!(display["initial"]["revision"], 1);
    assert_eq!(display["initial"]["painted_cells"], 3);
    assert_eq!(
        display["initial"]["identities"]["added"],
        serde_json::json!([1, 2])
    );
    assert_eq!(display["unchanged"]["ansi"], "");
    assert_eq!(display["changed"]["changed_cells"], 1);
    assert_eq!(display["malformed_ok"], false);
    assert!(
        display["malformed_error"]
            .as_str()
            .is_some_and(|error| error.contains("terminal control data"))
    );
    assert_eq!(display["revision_before_error"], 3);
    assert_eq!(display["revision_after_error"], 3);
    assert_eq!(display["redrawn"]["revision"], 4);
    assert_eq!(display["redrawn"]["full_redraw"], true);
}
