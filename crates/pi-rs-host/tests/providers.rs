//! `pi.register_provider` / `pi.unregister_provider` — the host-side
//! half of the spec's `registerProvider` seam (loader.ts queued
//! registrations; `upsertRegisteredProvider` merge; global-by-name
//! unregistration), exercised through the public API by the
//! `provider-demo.lua` example.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use serde_json::json;

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

#[test]
fn provider_demo_example_registers_through_the_mirror() {
    let host = host();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/provider-demo.lua"
    );
    host.load_file(path).unwrap();

    let providers = host.providers().unwrap();
    let names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["my-proxy", "anthropic", "corporate-ai"],
        "registration order; short-lived unregistered"
    );

    // upsert merge: the re-registration added `name`, kept the models.
    let my_proxy = &providers[0];
    assert_eq!(my_proxy.source, path);
    assert_eq!(my_proxy.config["name"], json!("My Proxy"));
    assert_eq!(
        my_proxy.config["baseUrl"],
        json!("https://proxy.example.com")
    );
    assert_eq!(my_proxy.config["apiKey"], json!("$PROXY_API_KEY"));
    assert_eq!(
        my_proxy.config["models"][0]["id"],
        json!("claude-sonnet-4-20250514")
    );
    assert_eq!(my_proxy.config["models"][0]["cost"]["cacheRead"], json!(0));

    // Override-only registration.
    assert_eq!(
        providers[1].config,
        json!({ "baseUrl": "https://proxy.example.com" })
    );

    // Functions stripped at depth; oauth.name survives.
    let corporate = &providers[2];
    assert_eq!(
        corporate.config["oauth"]["name"],
        json!("Corporate AI (SSO)")
    );
    assert_eq!(corporate.config["oauth"].as_object().unwrap().len(), 1);
}

#[test]
fn register_provider_validates_name() {
    let host = host();
    let err = host
        .load(
            "<bad>",
            r#"
                local pi = ...
                pi.register_provider("  ", { baseUrl = "https://x" })
            "#,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("register_provider: name must be a non-empty string"),
        "{err}"
    );
}

#[test]
fn unregister_is_global_by_name_across_extensions() {
    let host = host();
    host.load(
        "<a>",
        r#"
            local pi = ...
            pi.register_provider("shared", { baseUrl = "https://a.example.com" })
        "#,
    )
    .unwrap();
    host.load(
        "<b>",
        r#"
            local pi = ...
            pi.register_provider("own", { baseUrl = "https://b.example.com" })
            pi.unregister_provider("shared")
        "#,
    )
    .unwrap();

    let providers = host.providers().unwrap();
    let names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["own"], "spec: removal by name, any registrant");
}

/// A custom `streamSimple` registered via `pi.register_provider` participates
/// in `pi.ai.stream_simple` dispatch ahead of Rust providers (spec:
/// `applyProviderConfig` → `registerApiProvider`; stream.ts resolveApiProvider),
/// and unregistration removes only the named provider's handler.
#[test]
fn custom_stream_simple_provider_dispatches_and_unregisters() {
    let host = host();
    // Probe command first so it is present for both halves of the test.
    host.load(
        "<probe>",
        r#"
            local pi = ...
            pi.register_command("custom-stream-probe", {
                handler = function(args)
                    local model = pi.json.decode(args)
                    local events = {}
                    local final = pi.ai.stream_simple(model, { messages = {} }, {},
                        function(event) events[#events + 1] = { type = event.type, delta = event.delta } end)
                    return { events = events, stopReason = final.stopReason }
                end,
            })
        "#,
    )
    .expect("probe loads");

    host.load(
        "<custom>",
        r#"
            local pi = ...
            pi.register_provider("my-stream", {
                api = "custom-stream-api",
                streamSimple = function(model, context, options, on_event)
                    on_event({ type = "text_delta", delta = "hello", partial = {} })
                    on_event({ type = "done", reason = "stop", message = {
                        role = "assistant", content = {}, api = "custom-stream-api",
                        provider = "my-stream", model = model.id,
                        stopReason = "stop", timestamp = 0,
                    } })
                    return { role = "assistant", content = {}, api = "custom-stream-api",
                             provider = "my-stream", model = model.id,
                             stopReason = "stop", timestamp = 0 }
                end,
            })
        "#,
    )
    .expect("custom provider loads");

    // A model of the custom api streams through the Lua handler, not Rust.
    let model = json!({
        "id":"custom-1", "name":"Custom", "api":"custom-stream-api", "provider":"my-stream",
        "baseUrl":"", "reasoning":false, "input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000, "maxTokens":1024
    });
    let got = host
        .call_command("custom-stream-probe", &model.to_string())
        .unwrap()
        .unwrap();
    // serde_json arrays are 0-indexed (Lua tables were 1-indexed).
    assert_eq!(got["events"][0]["delta"], "hello");
    assert_eq!(got["events"][1]["type"], "done");
    assert_eq!(got["stopReason"], "stop");

    // Unregistering the provider removes the custom handler for its api.
    host.load(
        "<unregister>",
        r#"
            local pi = ...
            pi.unregister_provider("my-stream")
        "#,
    )
    .expect("unregister loads");
    let got = host
        .call_command("custom-stream-probe", &model.to_string())
        .unwrap()
        .unwrap();
    let rendered = serde_json::to_string(&got).unwrap();
    assert!(
        !rendered.contains("hello"),
        "custom handler should no longer run after unregister: {rendered}"
    );
}

/// Spec `ModelRegistry.unregisterProvider(name)` → `refresh()`: it never
/// removes a single api-registry handler directly. After deleting the provider
/// it rebuilds the whole custom API stream map from the remaining registered
/// providers (`resetApiProviders()` + re-`applyProviderConfig`). So when two
/// providers share one api string, unregistering one must NOT drop the handler
/// that the other still-registered provider keeps alive — the survivor's
/// streamSimple continues to dispatch by that api.
#[test]
fn shared_api_handler_survives_co_tenant_unregister() {
    let host = host();
    host.load(
        "<probe>",
        r#"
            local pi = ...
            pi.register_command("shared-api-probe", {
                handler = function(args)
                    local model = pi.json.decode(args)
                    local events = {}
                    local final = pi.ai.stream_simple(model, { messages = {} }, {},
                        function(event) events[#events + 1] = event.type end)
                    return { events = events, stopReason = final.stopReason,
                             provider = final.provider }
                end,
            })
        "#,
    )
    .expect("probe loads");

    // Two providers share the same api string, each with its own streamSimple.
    host.load(
        "<a>",
        r#"
            local pi = ...
            pi.register_provider("tenant-a", {
                api = "shared-api",
                streamSimple = function(model, context, options, on_event)
                    on_event({ type = "text_delta", delta = "A", partial = {} })
                    on_event({ type = "done", reason = "stop", message = {
                        role = "assistant", content = {}, api = "shared-api",
                        provider = "tenant-a", model = model.id,
                        stopReason = "stop", timestamp = 0,
                    } })
                    return { role = "assistant", content = {}, api = "shared-api",
                             provider = "tenant-a", model = model.id,
                             stopReason = "stop", timestamp = 0 }
                end,
            })
        "#,
    )
    .expect("tenant-a loads");
    host.load(
        "<b>",
        r#"
            local pi = ...
            pi.register_provider("tenant-b", {
                api = "shared-api",
                streamSimple = function(model, context, options, on_event)
                    on_event({ type = "text_delta", delta = "B", partial = {} })
                    on_event({ type = "done", reason = "stop", message = {
                        role = "assistant", content = {}, api = "shared-api",
                        provider = "tenant-b", model = model.id,
                        stopReason = "stop", timestamp = 0,
                    } })
                    return { role = "assistant", content = {}, api = "shared-api",
                             provider = "tenant-b", model = model.id,
                             stopReason = "stop", timestamp = 0 }
                end,
            })
        "#,
    )
    .expect("tenant-b loads");

    let model = json!({
        "id":"shared-1", "name":"Shared", "api":"shared-api", "provider":"tenant-b",
        "baseUrl":"", "reasoning":false, "input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000, "maxTokens":1024
    });

    // Pi's Map<api,provider> keeps the last registration for a shared api
    // (`registerApiProvider` Map.set replacement). In this load order,
    // tenant-b is last, so it owns the handler.
    let got = host
        .call_command("shared-api-probe", &model.to_string())
        .unwrap()
        .unwrap();
    assert_eq!(got["provider"], "tenant-b", "last registration wins for shared api");

    // Unregister tenant-a (the other co-tenant). Pi's refresh rebuilds the
    // handler from the remaining provider (tenant-b), so it stays live.
    host.load(
        "<unregister-a>",
        r#"
            local pi = ...
            pi.unregister_provider("tenant-a")
        "#,
    )
    .expect("unregister tenant-a loads");
    let got = host
        .call_command("shared-api-probe", &model.to_string())
        .unwrap()
        .unwrap();
    let rendered = serde_json::to_string(&got).unwrap();
    assert!(
        rendered.contains("provider") && got["provider"] == "tenant-b",
        "shared api handler must survive co-tenant unregister (spec refresh): {rendered}"
    );
}

/// Spec `validateProviderConfig` (coding-agent model-registry.ts): a
/// `streamSimple` without a non-empty `api` aborts registration (Pi throws
/// `Provider {name}: "api" is required when registering streamSimple.`).
/// A failed registration stores no config — the provider never appears.
#[test]
fn stream_simple_without_api_is_rejected_and_not_stored() {
    let host = host();
    let err = host
        .load(
            "<bad>",
            r#"
                local pi = ...
                pi.register_provider("no-api", {
                    streamSimple = function(model, ctx, opts, on_event)
                        on_event({ type = "done", reason = "stop", message = {} })
                        return {}
                    end,
                })
            "#,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Provider no-api: \"api\" is required when registering streamSimple."),
        "{err}"
    );
    let providers = host.providers().unwrap();
    let names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    assert!(
        !names.contains(&"no-api"),
        "failed registration must not store the provider: {names:?}"
    );

    // An empty-string api is rejected by Pi (`streamSimple && !config.api`).
    let err = host
        .load(
            "<bad-empty-api>",
            r#"
                local pi = ...
                pi.register_provider("empty-api", {
                    api = "",
                    streamSimple = function() end,
                })
            "#,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Provider empty-api: \"api\" is required when registering streamSimple."),
        "{err}"
    );

    // Pi accepts a whitespace-only api (`"   "` is truthy; the differential
    // oracle shows it registers a handler keyed by those spaces) — the port
    // must match, not trim-and-reject.
    host.load(
        "<blank-api>",
        r#"
            local pi = ...
            pi.register_provider("blank-api", {
                api = "   ",
                streamSimple = function() end,
            })
        "#,
    )
    .expect("whitespace-only api is accepted, matching Pi");
}

/// Spec `validateProviderConfig`: registering a provider with `models`
/// (non-empty) requires `baseUrl` and (`apiKey` or `oauth`), and each model
/// needs its own or the provider's `api`.
#[test]
fn provider_models_validation_matches_spec() {
    let host = host();

    // models without baseUrl.
    let err = host
        .load(
            "<m1>",
            r#"
                local pi = ...
                pi.register_provider("m1", {
                    api = "x",
                    apiKey = "k",
                    models = { { id = "a", model_id = "a" } },
                })
            "#,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Provider m1: \"baseUrl\" is required when defining models."),
        "{err}"
    );

    // models without apiKey or oauth.
    let err = host
        .load(
            "<m2>",
            r#"
                local pi = ...
                pi.register_provider("m2", {
                    api = "x",
                    baseUrl = "https://x",
                    models = { { id = "b" } },
                })
            "#,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Provider m2: \"apiKey\" or \"oauth\" is required when defining models."),
        "{err}"
    );

    // a model with neither its own api nor the provider api.
    let err = host
        .load(
            "<m3>",
            r#"
                local pi = ...
                pi.register_provider("m3", {
                    baseUrl = "https://x",
                    apiKey = "k",
                    models = { { id = "c" } },
                })
            "#,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Provider m3, model c: no \"api\" specified."),
        "{err}"
    );

    // A valid registration (model carries its own api) is accepted.
    host.load(
        "<ok>",
        r#"
            local pi = ...
            pi.register_provider("ok", {
                baseUrl = "https://x",
                apiKey = "k",
                models = { { id = "d", api = "x", model_id = "d" } },
            })
        "#,
    )
    .unwrap();
    let providers = host.providers().unwrap();
    let names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"ok"), "valid registration is stored: {names:?}");
}
