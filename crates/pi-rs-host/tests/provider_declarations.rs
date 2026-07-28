//! Provider declarations authored by an ordinary file-backed package.
//!
//! Rust contributes three mechanisms and no policy: the reviewed catalog
//! inventory (`providers`/`catalog`), the advertised wire-protocol families
//! (`apis`), and wire-schema validation of a package-authored model row
//! (`validate`). Which providers a product offers, their order, their custom
//! endpoints, and which one a dispatch streams through are Lua decisions
//! carried by the one generic declaration path, `pi.kernel.v1.declare`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read, Write};

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

/// Declares a catalog-backed provider and a custom-endpoint provider, then
/// streams through whichever declaration its own ordering selected.
const DECLARING_PACKAGE: &str = r#"
local pi = ...
local models = pi.models.v1
local kernel = pi.kernel.v1
local effects = pi.effects.v1
local roots = pi.roots.v1

-- Inventory is mechanism data; the product order below is policy.
local providers = models.providers()
local apis = models.apis()
local source_provider = providers[1]
local page_one, catalog_total = models.catalog(source_provider, { limit = 2 })
local page_two = models.catalog(source_provider, { limit = 1, offset = 1 })
local paged_distinct = page_one[2] ~= nil and page_two[1] ~= nil
  and page_one[1].id ~= page_two[1].id

-- A catalog row is already a valid model row: validating one is identity.
local catalog_model = models.validate(page_one[1])
local catalog_identity = catalog_model.id == page_one[1].id
  and catalog_model.baseUrl == page_one[1].baseUrl
  and catalog_model.api == page_one[1].api

kernel.declare("provider", {
  id = "catalog-first",
  order = 10,
  label = "First catalog row",
  model = catalog_model,
})

-- A custom endpoint the catalog never mentions. The port arrives through the
-- public environment/filesystem effects, so nothing private links test to Lua.
local fixture_directory = effects.env.get("FIXTURE_DIR")
local port = effects.fs.read(effects.path.join(fixture_directory, "port.txt")):match("(%d+)")
local fixture_model = models.validate({
  id = "fixture-model",
  name = "Fixture Model",
  api = "openai-completions",
  provider = "local-fixture",
  baseUrl = "http://127.0.0.1:" .. port,
  reasoning = false,
  input = { "text" },
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
  contextWindow = 4096,
  maxTokens = 64,
})

kernel.declare("provider", {
  id = "local-fixture",
  order = 0,
  label = "Fixture endpoint",
  model = fixture_model,
})

local function supported(api)
  for _, name in ipairs(apis) do
    if name == api then return true end
  end
  return false
end

roots.register({
  kind = "application",
  id = "provider-declarations",
  dispatch = function()
    -- Selection is the package's own: the declaration order it chose.
    local declared = kernel.registered("provider")
    local ids = {}
    for _, entry in ipairs(declared) do ids[#ids + 1] = entry.declaration_id end
    local selected = declared[1]

    local streamed = ""
    local message = models.stream(
      selected.model,
      { messages = { { role = "user", content = "hello", timestamp = 0 } } },
      { apiKey = "fixture-key", max_events = 32 },
      function(event)
        if event.type == "text_delta" and event.delta then
          streamed = streamed .. event.delta
        end
      end
    )

    roots.action("declared", {
      provider_names = #providers,
      first_provider = source_provider,
      catalog_rows = #page_one,
      catalog_total = catalog_total,
      paged_distinct = paged_distinct,
      catalog_identity = catalog_identity,
      apis = apis,
      completions_supported = supported("openai-completions"),
      declared_ids = ids,
      selected = selected.declaration_id,
      selected_provider = selected.model.provider,
      selected_base_url = selected.model.baseUrl,
      streamed = streamed,
      stop = message.stopReason,
      default_max_models = models.default_max_models,
      max_models = models.max_models,
      default_max_events = models.default_max_events,
      max_events = models.max_events,
    })
  end,
})
"#;

/// Every refusal a configuration package needs at declaration time, before a
/// single provider request is made.
const REFUSING_PACKAGE: &str = r#"
local pi = ...
local models = pi.models.v1
local roots = pi.roots.v1

local function refusal(fn)
  local ok, error_value = pcall(fn)
  return { ok = ok, message = tostring(error_value) }
end

roots.register({
  kind = "application",
  id = "provider-refusals",
  dispatch = function()
    local provider = models.providers()[1]
    local unknown_rows, unknown_total = models.catalog("no-such-provider")
    local complete_row = {
      id = "fixture-model",
      name = "Fixture Model",
      api = "openai-completions",
      provider = "local-fixture",
      baseUrl = "http://127.0.0.1:9",
      reasoning = false,
      input = { "text" },
      cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
      contextWindow = 4096,
      maxTokens = 64,
    }

    local function with(field, value)
      local copy = {}
      for key, item in pairs(complete_row) do copy[key] = item end
      copy[field] = value
      return copy
    end

    roots.action("refused", {
      zero_limit = refusal(function() return models.catalog(provider, { limit = 0 }) end),
      over_limit = refusal(function()
        return models.catalog(provider, { limit = models.max_models + 1 })
      end),
      unknown_provider_rows = #unknown_rows,
      unknown_provider_total = unknown_total,
      unsupported_api = refusal(function()
        return models.validate(with("api", "not-a-wire-protocol"))
      end),
      empty_base_url = refusal(function() return models.validate(with("baseUrl", "")) end),
      incomplete = refusal(function() return models.validate({ id = "fixture-model" }) end),
      accepted = models.validate(complete_row).id,
    })
  end,
})
"#;

/// Canned OpenAI-completions SSE served by an ordinary local HTTP socket.
fn spawn_fixture_provider(directory: &std::path::Path) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture provider");
    let port = listener.local_addr().expect("fixture address").port();
    std::fs::write(directory.join("port.txt"), port.to_string()).expect("write fixture port");
    std::thread::spawn(move || {
        const CHUNKS: [&str; 3] = ["Hello", ", declared", " provider"];
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            match socket.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return,
            }
            if request.len() > 64 * 1024 {
                return;
            }
        }
        let mut body = String::new();
        for delta in CHUNKS {
            let chunk = serde_json::json!({
                "id": "fixture-completion",
                "model": "fixture-model",
                "choices": [{
                    "index": 0,
                    "delta": { "content": delta },
                    "finish_reason": serde_json::Value::Null,
                }],
            });
            body.push_str(&format!("data: {chunk}\n\n"));
        }
        let done = serde_json::json!({
            "id": "fixture-completion",
            "model": "fixture-model",
            "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8 },
        });
        body.push_str(&format!("data: {done}\n\n"));
        body.push_str("data: [DONE]\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len(),
        );
        let _ = socket.write_all(response.as_bytes());
    });
}

fn host_with(environment: &[(&str, &str)]) -> Host {
    let environment = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Host::new(HostConfig {
        environment: Some(environment),
        ..HostConfig::default()
    })
    .expect("host starts")
}

fn load(host: &Host, directory: &std::path::Path, name: &str, source: &str) {
    let path = directory.join(format!("{name}.lua"));
    std::fs::write(&path, source).expect("write file-backed package");
    host.load_package(PackageSource::File { path: &path })
        .expect("file-backed package loads");
}

fn dispatch(host: &Host) -> DispatchBatch {
    host.dispatch(DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({ "kind": "startup" }),
        serde_json::json!({}),
    ))
    .expect("application dispatch")
}

#[test]
fn file_backed_package_declares_catalog_and_custom_providers_and_streams() {
    let directory = tempfile::tempdir().expect("temporary directory");
    spawn_fixture_provider(directory.path());
    let host = host_with(&[("FIXTURE_DIR", &directory.path().to_string_lossy())]);
    load(&host, directory.path(), "declaring", DECLARING_PACKAGE);

    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    // Inventory: every catalog provider is visible, windows are bounded, and
    // the count of the full row set comes back with the window.
    assert!(payload["provider_names"].as_u64().expect("provider count") >= 1);
    assert!(
        !payload["first_provider"]
            .as_str()
            .expect("provider name")
            .is_empty()
    );
    assert!(payload["catalog_rows"].as_u64().expect("window size") <= 2);
    assert!(
        payload["catalog_total"].as_u64().expect("row total")
            >= payload["catalog_rows"].as_u64().expect("window size")
    );
    assert_eq!(payload["paged_distinct"], serde_json::json!(true));
    assert_eq!(payload["catalog_identity"], serde_json::json!(true));

    // The advertised wire-protocol families are visible without streaming.
    assert_eq!(payload["completions_supported"], serde_json::json!(true));
    let apis = payload["apis"].as_array().expect("api families");
    assert!(apis.iter().any(|api| api == "anthropic-messages"));

    // Declarations rode the one generic path and kept the package's order.
    assert_eq!(
        payload["declared_ids"],
        serde_json::json!(["local-fixture", "catalog-first"])
    );
    assert_eq!(payload["selected"], serde_json::json!("local-fixture"));
    assert_eq!(
        payload["selected_provider"],
        serde_json::json!("local-fixture")
    );
    assert!(
        payload["selected_base_url"]
            .as_str()
            .expect("custom endpoint")
            .starts_with("http://127.0.0.1:")
    );

    // The custom-endpoint declaration streams through the same binding a
    // catalog row would.
    assert_eq!(
        payload["streamed"],
        serde_json::json!("Hello, declared provider")
    );
    assert_eq!(payload["stop"], serde_json::json!("stop"));

    // The bounds are part of the surface, not folklore.
    assert_eq!(payload["default_max_models"], serde_json::json!(64));
    assert_eq!(payload["max_models"], serde_json::json!(512));
    assert_eq!(payload["default_max_events"], serde_json::json!(256));
    assert_eq!(payload["max_events"], serde_json::json!(1024));
}

#[test]
fn invalid_rows_and_windows_are_refused_before_any_request() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let host = host_with(&[]);
    load(&host, directory.path(), "refusing", REFUSING_PACKAGE);

    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    for (field, fragment) in [
        ("zero_limit", "limit must be in 1..=512"),
        ("over_limit", "limit must be in 1..=512"),
        ("unsupported_api", "unsupported api not-a-wire-protocol"),
        ("empty_base_url", "must be non-empty"),
        ("incomplete", "invalid model: missing field"),
    ] {
        assert_eq!(
            payload[field]["ok"],
            serde_json::json!(false),
            "{field} should be refused"
        );
        let message = payload[field]["message"].as_str().expect("diagnostic");
        assert!(
            message.contains(fragment),
            "{field} diagnostic {message:?} should name {fragment:?}"
        );
    }

    // The refusal for an unsupported api names the families that do work.
    let unsupported = payload["unsupported_api"]["message"]
        .as_str()
        .expect("diagnostic");
    assert!(unsupported.contains("openai-completions"));

    // An unknown provider is an empty window, not an error: absence is data.
    assert_eq!(payload["unknown_provider_rows"], serde_json::json!(0));
    assert_eq!(payload["unknown_provider_total"], serde_json::json!(0));
    assert_eq!(payload["accepted"], serde_json::json!("fixture-model"));
}
