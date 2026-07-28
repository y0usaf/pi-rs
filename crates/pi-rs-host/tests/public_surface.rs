//! Exact public-surface ablation and source-neutrality guard.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const PROBE: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

local function keys(table_value)
  local result = {}
  for key in pairs(table_value) do
    if type(key) == "string" then result[#result + 1] = key end
  end
  table.sort(result)
  return result
end

local shape = {
  top = keys(pi),
  kernel = keys(pi.kernel),
  roots = keys(pi.roots),
  terminal = keys(pi.terminal),
  models = keys(pi.models),
  effects = keys(pi.effects),
  records = keys(pi.records),
  packages = keys(pi.packages),
  kernel_v1 = keys(pi.kernel.v1),
  module = keys(pi.kernel.v1.module),
  effects_v1 = keys(pi.effects.v1),
  packages_v1 = keys(pi.packages.v1),
}

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
      shape=shape,
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
fn file_and_embedded_packages_see_only_the_same_compact_surface() {
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
        payload["shape"]["top"],
        serde_json::json!([
            "effects", "kernel", "models", "packages", "records", "roots", "terminal"
        ])
    );
    for member in [
        "kernel", "roots", "terminal", "models", "effects", "records", "packages",
    ] {
        assert_eq!(payload["shape"][member], serde_json::json!(["v1"]));
    }
    // Package composition exposes exactly load, list, and its bounds.
    assert_eq!(
        payload["shape"]["packages_v1"],
        serde_json::json!([
            "api_version",
            "list",
            "load",
            "max_depth",
            "max_packages",
            "max_source_bytes"
        ])
    );
    // The kernel transaction and its exact-version module lifecycle are pinned:
    // reload is `remove` + `define` or `reset`, never a second declaration path.
    assert_eq!(
        payload["shape"]["kernel_v1"],
        serde_json::json!([
            "action",
            "api_version",
            "cancellation",
            "declare",
            "effect",
            "module",
            "read_handle",
            "registered",
            "resource",
            "root"
        ])
    );
    assert_eq!(
        payload["shape"]["module"],
        serde_json::json!(["define", "list", "remove", "require", "reset"])
    );
    // The effect families are pinned too: filesystem, path arithmetic,
    // environment snapshot, processes, timers, and cancellation.
    assert_eq!(
        payload["shape"]["effects_v1"],
        serde_json::json!([
            "api_version",
            "cancellation",
            "env",
            "fs",
            "path",
            "process",
            "timer"
        ])
    );
    assert_eq!(
        payload["input"],
        serde_json::json!({"text":"same input", "events":10})
    );
    assert_eq!(payload["revision"], 1);
    assert_eq!(payload["painted_cells"], 10);
}
