//! Registry-layer parity tests against the spec's `api-registry.ts`,
//! `stream.ts`, `env-api-keys.ts` and the registry half of `models.ts`.
//!
//! The API-provider registry is process-global (as in the spec); tests
//! that touch it use unique api names so parallel execution can't
//! interfere, and the one clear/reset scenario runs behind a lock shared
//! with the resolution tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai::registry::{
    ApiProvider, find_env_keys, get_api_provider, get_api_providers, get_env_api_key, get_model,
    get_models, get_providers, register_api_provider, reset_api_providers, stream, stream_simple,
    unregister_api_providers,
};
use pi_rs_ai::transport::event_stream::create_assistant_message_event_stream;
use pi_rs_ai_types::{Context, KNOWN_APIS, Model};
use serde_json::Value;

/// Serializes every test that mutates the process-global API-provider
/// registry or the environment (reset clears *all* registrations, so
/// unique api names alone are not enough).
fn global_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------
// Catalog (models.ts registry half + generated data)
// ---------------------------------------------------------------------

/// Pin the generation: 35 providers / 969 models, in the spec's
/// `MODELS` declaration order (first key: amazon-bedrock).
#[test]
fn catalog_counts_and_order_pin_the_generation() {
    let providers = get_providers();
    assert_eq!(providers.len(), 35, "provider count");
    assert_eq!(providers.first().copied(), Some("amazon-bedrock"));
    assert!(providers.contains(&"anthropic"));
    assert!(providers.contains(&"openai"));

    let total: usize = providers.iter().map(|p| get_models(p).len()).sum();
    assert_eq!(total, 969, "model count");
}

#[test]
fn catalog_provenance_matches_inventory_and_protocol_vocabulary() {
    let provenance: Value =
        serde_json::from_str(include_str!("../data/models.provenance.json")).unwrap();
    let providers = get_providers();
    let total: usize = providers
        .iter()
        .map(|provider| get_models(provider).len())
        .sum();
    assert_eq!(provenance["schemaVersion"], 1);
    assert_eq!(provenance["inventory"]["providers"], providers.len());
    assert_eq!(provenance["inventory"]["models"], total);

    for provider in providers {
        for model in get_models(provider) {
            assert!(
                KNOWN_APIS.contains(&model.api.as_str()),
                "{provider}/{} uses unreviewed API {}",
                model.id,
                model.api
            );
        }
    }
}

#[test]
fn get_models_unknown_provider_is_empty() {
    assert!(get_models("no-such-provider").is_empty());
    assert!(get_model("no-such-provider", "x").is_none());
}

/// The anthropic opus row matches the fixture transcribed verbatim from
/// the spec in WS2.1 — catalog data and hand-pinned data agree.
#[test]
fn catalog_row_matches_spec_fixture() {
    let path = format!(
        "{}/../pi-rs-ai-types/tests/fixtures/model_anthropic_opus47.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap();
    let fixture: Value = serde_json::from_str(&raw).unwrap();

    let model = get_model("anthropic", "claude-opus-4-7").expect("catalog row");
    let row = serde_json::to_value(model).unwrap();
    assert_eq!(normalize(&fixture), normalize(&row));
}

/// Every generated row round-trips through `Model` to an identical
/// `Value` — no field of the spec's catalog is silently dropped.
#[test]
fn every_catalog_row_roundtrips() {
    let raw = include_str!("../data/models.json");
    let catalog: Vec<Value> = serde_json::from_str(raw).unwrap();
    for entry in &catalog {
        let provider = entry["provider"].as_str().unwrap();
        for raw_model in entry["models"].as_array().unwrap() {
            let typed: Model = serde_json::from_value(raw_model.clone())
                .unwrap_or_else(|e| panic!("{provider}: deserialize failed: {e}"));
            let reserialized = serde_json::to_value(&typed).unwrap();
            assert_eq!(
                normalize(raw_model),
                normalize(&reserialized),
                "{provider}/{}: round-trip mismatch",
                typed.id
            );
        }
    }
}

/// Normalize every number to f64 so `5` and `5.0` compare equal (same
/// harness rule as the WS2.1 fixtures).
fn normalize(value: &Value) -> Value {
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| value.clone()),
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), normalize(v))).collect())
        }
        _ => value.clone(),
    }
}

// ---------------------------------------------------------------------
// api-registry.ts
// ---------------------------------------------------------------------

fn test_model(api: &str, provider: &str) -> Model {
    serde_json::from_value(serde_json::json!({
        "id": "test-model",
        "name": "Test Model",
        "api": api,
        "provider": provider,
        "baseUrl": "http://127.0.0.1:1",
        "reasoning": false,
        "input": ["text"],
        "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
        "contextWindow": 1000,
        "maxTokens": 100
    }))
    .unwrap()
}

/// A provider that records the options it was called with.
fn capture_provider(api: &str) -> (ApiProvider, Arc<Mutex<Vec<Option<String>>>>) {
    let captured: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let stream_keys = Arc::clone(&captured);
    let simple_keys = Arc::clone(&captured);
    let provider = ApiProvider {
        api: api.to_owned(),
        stream: Arc::new(move |_, _, options: Option<StreamOptions>| {
            stream_keys
                .lock()
                .unwrap()
                .push(options.and_then(|o| o.api_key));
            Ok(create_assistant_message_event_stream())
        }),
        stream_simple: Arc::new(move |_, _, options: Option<SimpleStreamOptions>| {
            simple_keys
                .lock()
                .unwrap()
                .push(options.and_then(|o| o.base.api_key));
            Ok(create_assistant_message_event_stream())
        }),
    };
    (provider, captured)
}

#[test]
fn register_get_and_replace_in_place() {
    let _guard = global_lock();
    let (provider, _) = capture_provider("test-api-replace");
    register_api_provider(provider, Some("test-a"));
    assert!(get_api_provider("test-api-replace").is_some());

    // Map.set on an existing key replaces the value, keeps its position.
    let before: Vec<String> = get_api_providers().iter().map(|p| p.api.clone()).collect();
    let (replacement, _) = capture_provider("test-api-replace");
    register_api_provider(replacement, Some("test-b"));
    let after: Vec<String> = get_api_providers().iter().map(|p| p.api.clone()).collect();
    assert_eq!(before, after, "insertion order preserved on re-register");

    // The replacement's sourceId owns the entry now.
    unregister_api_providers("test-a");
    assert!(get_api_provider("test-api-replace").is_some());
    unregister_api_providers("test-b");
    assert!(get_api_provider("test-api-replace").is_none());
}

#[test]
fn mismatched_api_is_the_spec_error() {
    let _guard = global_lock();
    let (provider, _) = capture_provider("test-api-mismatch");
    register_api_provider(provider, Some("test-mismatch"));
    let registered = get_api_provider("test-api-mismatch").unwrap();

    let model = test_model("some-other-api", "test");
    let err = match (registered.stream)(&model, &Context::default(), None) {
        Err(err) => err,
        Ok(_) => panic!("mismatched api must error"),
    };
    assert_eq!(
        err.to_string(),
        "Mismatched api: some-other-api expected test-api-mismatch"
    );
    unregister_api_providers("test-mismatch");
}

// ---------------------------------------------------------------------
// stream.ts resolution + env-api-keys.ts
// ---------------------------------------------------------------------

#[test]
fn unregistered_api_is_the_spec_error() {
    let _guard = global_lock();
    let model = test_model("api-that-never-registers", "test");
    let err = match stream(&model, &Context::default(), None) {
        Err(err) => err,
        Ok(_) => panic!("unregistered api must error"),
    };
    assert_eq!(
        err.to_string(),
        "No API provider registered for api: api-that-never-registers"
    );
}

#[test]
fn builtins_resolve_after_reset() {
    let _guard = global_lock();
    reset_api_providers();
    assert!(get_api_provider("anthropic-messages").is_some());
    assert!(get_api_provider("openai-completions").is_some());
}

#[test]
fn env_api_key_injection_and_explicit_precedence() {
    let _guard = global_lock();
    // SAFETY: single-threaded here (the lock serializes every test that
    // reads this variable; nothing else in the binary touches env).
    unsafe { std::env::set_var("DEEPSEEK_API_KEY", "sk-env-key") };

    assert_eq!(find_env_keys("deepseek"), Some(vec!["DEEPSEEK_API_KEY"]));
    assert_eq!(get_env_api_key("deepseek"), Some("sk-env-key".to_owned()));
    assert_eq!(get_env_api_key("provider-with-no-env"), None);

    let (provider, captured) = capture_provider("test-api-env");
    register_api_provider(provider, Some("test-env"));
    let model = test_model("test-api-env", "deepseek");

    // No explicit key → env key injected (stream and streamSimple).
    assert!(stream(&model, &Context::default(), None).is_ok());
    assert!(stream_simple(&model, &Context::default(), None).is_ok());
    // Explicit key wins; blank explicit key does not count as explicit.
    let explicit = StreamOptions {
        api_key: Some("sk-explicit".to_owned()),
        ..StreamOptions::default()
    };
    assert!(stream(&model, &Context::default(), Some(explicit)).is_ok());
    let blank = StreamOptions {
        api_key: Some("   ".to_owned()),
        ..StreamOptions::default()
    };
    assert!(stream(&model, &Context::default(), Some(blank)).is_ok());

    let calls = captured.lock().unwrap();
    assert_eq!(
        *calls,
        vec![
            Some("sk-env-key".to_owned()),
            Some("sk-env-key".to_owned()),
            Some("sk-explicit".to_owned()),
            Some("sk-env-key".to_owned()),
        ]
    );
    drop(calls);
    unregister_api_providers("test-env");
}

#[test]
fn anthropic_env_precedence_is_oauth_token_first() {
    // Pure mapping check (no env mutation): the anthropic row lists the
    // OAuth token before the API key.
    let _guard = global_lock();
    // SAFETY: serialized by the lock, as above.
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "sk-api");
        std::env::set_var("ANTHROPIC_OAUTH_TOKEN", "sk-oauth");
    }
    assert_eq!(get_env_api_key("anthropic"), Some("sk-oauth".to_owned()));
    assert_eq!(
        find_env_keys("anthropic"),
        Some(vec!["ANTHROPIC_OAUTH_TOKEN", "ANTHROPIC_API_KEY"])
    );
    unsafe {
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("ANTHROPIC_OAUTH_TOKEN");
    }
}
