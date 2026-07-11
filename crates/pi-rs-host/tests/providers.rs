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
