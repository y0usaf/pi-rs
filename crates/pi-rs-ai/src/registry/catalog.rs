//! Built-in model catalog — the registry half of the spec's `models.ts`.
//!
//! The catalog itself is generated data, never hand code (locked
//! `pi-rs-ai` row): `data/models.json` is produced from the spec's
//! `models.generated.ts` (17.5k lines of rows) by
//! `scripts/gen-models-json.ts` and embedded via `include_str!`. The
//! shape is an ordered array of `{ provider, models }` — the spec's
//! `Record` insertion order made explicit.
//!
//! A parse failure of the embedded data yields an empty catalog rather
//! than a panic (library crates never panic); the registry tests pin the
//! parsed provider/model counts, so a bad generation cannot land.

use std::sync::LazyLock;

use pi_rs_ai_types::Model;
use serde::Deserialize;

/// One provider's models, in the spec's declaration order.
#[derive(Debug, Clone, Deserialize)]
struct ProviderModels {
    provider: String,
    models: Vec<Model>,
}

static MODELS_JSON: &str = include_str!("../../data/models.json");

static CATALOG: LazyLock<Vec<ProviderModels>> =
    LazyLock::new(|| serde_json::from_str(MODELS_JSON).unwrap_or_default());

/// Spec: `getProviders()` — provider names in catalog order.
pub fn get_providers() -> Vec<&'static str> {
    CATALOG.iter().map(|p| p.provider.as_str()).collect()
}

/// Spec: `getModels(provider)` — all models for a provider in catalog
/// order; empty for unknown providers.
pub fn get_models(provider: &str) -> &'static [Model] {
    CATALOG
        .iter()
        .find(|p| p.provider == provider)
        .map(|p| p.models.as_slice())
        .unwrap_or(&[])
}

/// Spec: `getModel(provider, modelId)` (the spec's TS overload returns
/// `undefined` for unknown pairs at runtime; here that is `None`).
pub fn get_model(provider: &str, model_id: &str) -> Option<&'static Model> {
    CATALOG
        .iter()
        .find(|p| p.provider == provider)?
        .models
        .iter()
        .find(|m| m.id == model_id)
}
