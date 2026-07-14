//! Source-neutral public Lua surface and behavior guard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const PROBE: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

local function api_shape(api)
  local shape = {}
  for key, value in pairs(api) do
    if type(key) == "string" then
      shape[#shape + 1] = key .. ":" .. type(value)
      if type(value) == "table" then
        for child_key, child_value in pairs(value) do
          if type(child_key) == "string" then
            shape[#shape + 1] = key .. "." .. child_key .. ":" .. type(child_value)
          end
        end
      end
    end
  end
  table.sort(shape)
  return shape
end

local received_shape = api_shape(pi)
roots.register({
  kind="application", id="source-neutral-probe", active=true, priority=0,
  dispatch=function()
    local input = terminal.input_buffer()
    local events = input:feed("same input")
    local text = ""
    for _, event in ipairs(events) do text = text .. event.data end
    local display = terminal.display()
    local submitted = display:submit({
      version=terminal.display_schema_version,
      viewport={columns=20, rows=1}, root=1,
      nodes={{
        id=1, rect={x=0, y=0, width=20, height=1},
        content={kind="text", runs={{text=text}}},
      }},
    })
    roots.action("probed", {
      api_shape=received_shape,
      input={text=text, events=#events},
      revision=submitted.revision,
      painted_cells=submitted.painted_cells,
    })
  end,
})
"#;

fn dispatch(host: &Host) -> pi_rs_host::kernel::DispatchBatch {
    host.dispatch(DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({ "kind": "probe" }),
        serde_json::json!({}),
    ))
    .expect("probe dispatch")
}

#[test]
fn embedded_and_file_sources_receive_identical_capability_and_behavior() {
    let embedded_host = Host::new(HostConfig::default()).expect("embedded host starts");
    embedded_host
        .load_package(PackageSource::Embedded {
            name: "surface-probe",
            source: PROBE,
        })
        .expect("embedded package loads");
    let embedded = dispatch(&embedded_host);

    let directory = tempfile::tempdir().expect("temporary package directory");
    let path = directory.path().join("surface-probe.lua");
    std::fs::write(&path, PROBE).expect("write file-backed package");
    let file_host = Host::new(HostConfig::default()).expect("file host starts");
    file_host
        .load_package(PackageSource::File { path: &path })
        .expect("file package loads");
    let file = dispatch(&file_host);

    assert_eq!(embedded.source, "<surface-probe>");
    assert_eq!(file.source, path.to_string_lossy());
    assert_eq!(embedded.actions, file.actions);
    assert_eq!(embedded.effects, file.effects);

    let payload = &embedded.actions[0].payload;
    assert_eq!(
        payload["input"],
        serde_json::json!({"text":"same input", "events":10})
    );
    assert_eq!(payload["revision"], 1);
    assert_eq!(payload["painted_cells"], 10);
    let shape = payload["api_shape"].as_array().expect("shape array");
    for member in [
        "roots:table",
        "terminal:table",
        "models:table",
        "effects:table",
    ] {
        assert!(
            shape.iter().any(|entry| entry == member),
            "missing {member}"
        );
    }
}
