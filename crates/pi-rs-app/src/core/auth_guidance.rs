//! Port of `core/auth-guidance.ts` — provider login guidance strings.

use crate::config::get_docs_path;

const UNKNOWN_PROVIDER: &str = "unknown";

/// Spec: `getProviderLoginHelp()`.
pub fn get_provider_login_help() -> String {
    let docs = get_docs_path();
    format!(
        "Use /login to log into a provider via OAuth or API key. See:\n  {}\n  {}",
        docs.join("providers.md").display(),
        docs.join("models.md").display()
    )
}

/// Spec: `formatNoModelsAvailableMessage()`.
pub fn format_no_models_available_message() -> String {
    format!("No models available. {}", get_provider_login_help())
}

/// Spec: `formatNoModelSelectedMessage()`.
pub fn format_no_model_selected_message() -> String {
    format!(
        "No model selected.\n\n{}\n\nThen use /model to select a model.",
        get_provider_login_help()
    )
}

/// Spec: `formatNoApiKeyFoundMessage(provider)`.
pub fn format_no_api_key_found_message(provider: &str) -> String {
    let provider_display = if provider == UNKNOWN_PROVIDER {
        "the selected model"
    } else {
        provider
    };
    format!(
        "No API key found for {provider_display}.\n\n{}",
        get_provider_login_help()
    )
}
