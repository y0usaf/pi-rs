//! Source-neutral public Lua surface guard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{EmbeddedPack, Host, HostConfig};
const PROBE: &str = r#"
local pi = ...

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
pi.on("public_surface_probe", function()
  return { api_shape = received_shape }
end)
"#;

#[test]
fn embedded_and_file_sources_receive_the_same_api_table_shape() {
    let host = Host::new(HostConfig::default()).expect("host starts");
    let embedded = host.load_embedded(&[EmbeddedPack {
        name: "surface-probe",
        source: PROBE,
    }]);
    assert!(embedded.errors.is_empty(), "{:?}", embedded.errors);

    let directory = tempfile::tempdir().expect("temporary extension directory");
    let path = directory.path().join("surface-probe.lua");
    std::fs::write(&path, PROBE).expect("write file-backed extension");
    let path = path.to_string_lossy().into_owned();
    let file = host.load_extensions(std::slice::from_ref(&path));
    assert!(file.errors.is_empty(), "{:?}", file.errors);

    let outcomes = host
        .emit("public_surface_probe", &serde_json::json!({}))
        .expect("probe dispatch");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].source, "<surface-probe>");
    assert_eq!(outcomes[1].source, path);
    let embedded_result = outcomes[0]
        .result
        .as_ref()
        .expect("embedded probe succeeds")
        .as_ref()
        .expect("embedded probe returns a shape");
    let file_result = outcomes[1]
        .result
        .as_ref()
        .expect("file probe succeeds")
        .as_ref()
        .expect("file probe returns a shape");
    assert_eq!(embedded_result, file_result);

    let shape = embedded_result["api_shape"]
        .as_array()
        .expect("API shape is an array");
    for representative in [
        "register_tool:function", // composable declaration API
        "fs:table",               // additive mechanism API
        "fs.read_file:function",
    ] {
        assert!(
            shape.iter().any(|entry| entry == representative),
            "missing representative public member {representative}"
        );
    }
}
