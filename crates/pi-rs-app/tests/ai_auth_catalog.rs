//! Whole catalog/auth acceptance gate for PLAN item 8.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use pi_rs_ai::registry::{get_api_providers, get_models, get_providers, reset_api_providers};
use pi_rs_ai_auth::{get_oauth_providers, reset_oauth_providers};
use serde_json::Value;

#[test]
fn every_catalog_api_dispatches_and_subscription_auth_registry_is_complete() {
    reset_api_providers();
    let registered = get_api_providers()
        .into_iter()
        .map(|provider| provider.api)
        .collect::<Vec<_>>();
    assert_eq!(
        registered,
        [
            "anthropic-messages",
            "openai-completions",
            "mistral-conversations",
            "openai-responses",
            "azure-openai-responses",
            "openai-codex-responses",
            "google-generative-ai",
            "google-vertex",
            "bedrock-converse-stream",
        ]
    );

    let provenance: Value =
        serde_json::from_str(include_str!("../../pi-rs-ai/data/models.provenance.json")).unwrap();
    let expected = provenance["inventory"]["apis"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(api, count)| (api.clone(), count.as_u64().unwrap() as usize))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::new();
    for provider in get_providers() {
        for model in get_models(provider) {
            assert!(
                registered.iter().any(|api| api == &model.api),
                "{provider}/{} advertises unregistered api {}",
                model.id,
                model.api
            );
            *actual.entry(model.api.clone()).or_insert(0) += 1;
        }
    }
    assert_eq!(actual, expected);

    reset_oauth_providers();
    let oauth = get_oauth_providers();
    assert_eq!(
        oauth
            .iter()
            .map(|provider| provider.id())
            .collect::<Vec<_>>(),
        ["anthropic", "github-copilot", "openai-codex"]
    );
    assert!(oauth.iter().all(|provider| !provider.name().is_empty()));
}
